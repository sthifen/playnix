//! Portapapeles compartido entre Linux y Android (`playnix clipboard`, servicio de
//! usuario). Del lado de Android, scrcpy-server avisa de cada cambio y recibe lo que
//! se copia en Linux. Del lado de Linux se usa el portapapeles de X11 (XWayland):
//! GNOME no deja leer el de Wayland a un programa sin una ventana con foco, pero lo
//! mantiene sincronizado con el de X11, y ése sí se puede leer y vigilar siempre.

use crate::android;
use crate::controls::scrcpy::Link;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::io::{ErrorKind, Read};
use std::net::TcpStream;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

/// Más grande no se pasa: es texto, no archivos.
const MAX_TEXT: usize = 1 << 24;

enum Ev {
    /// Recién conectado a Android: se le manda lo que haya copiado en Linux.
    Connected,
    /// Se copió algo en Android.
    Text(String),
}

type Shared = Arc<Mutex<Option<Arc<Link>>>>;

pub fn run() -> glib::ExitCode {
    gdk::set_allowed_backends("x11");
    // Sin ventanas no hace falta la GPU (ahorra memoria: no carga el driver)
    // SAFETY: todavía no hay otros hilos que lean el entorno
    unsafe {
        if gtk::minor_version() >= 16 {
            std::env::set_var("GDK_DISABLE", "gl,vulkan");
        } else {
            std::env::set_var("GDK_DEBUG", "gl-disable,vulkan-disable");
        }
    }
    if gtk::init().is_err() {
        eprintln!("playnix: sin XWayland no hay portapapeles compartido con Android");
        return glib::ExitCode::SUCCESS;
    }
    let Some(display) = gdk::Display::default() else { return glib::ExitCode::SUCCESS };
    let board = display.clipboard();
    let link = Shared::default();
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn({
        let link = link.clone();
        move || connection(&link, &tx)
    });

    // Lo último que pasó en cualquier sentido, para no devolverlo al otro lado
    let last: Rc<RefCell<String>> = Rc::default();
    let send = {
        let (link, last) = (link.clone(), last.clone());
        move |text: String| {
            if text.is_empty() || text.len() > MAX_TEXT || *last.borrow() == text {
                return;
            }
            if let Some(l) = link.lock().ok().and_then(|l| l.clone()) {
                l.set_clipboard(&text);
            }
            *last.borrow_mut() = text;
        }
    };

    // Linux → Android
    board.connect_changed({
        let send = send.clone();
        move |b| {
            if b.is_local() {
                return; // lo pusimos nosotros
            }
            let (b, send) = (b.clone(), send.clone());
            glib::spawn_future_local(async move {
                if let Ok(Some(text)) = b.read_text_future().await {
                    send(text.to_string());
                }
            });
        }
    });

    // Android → Linux
    glib::spawn_future_local(async move {
        while let Ok(ev) = rx.recv().await {
            match ev {
                Ev::Connected => {
                    last.borrow_mut().clear();
                    if !board.is_local() {
                        if let Ok(Some(text)) = board.read_text_future().await {
                            send(text.to_string());
                        }
                    }
                }
                Ev::Text(text) => {
                    if !text.is_empty() && *last.borrow() != text {
                        board.set_text(&text);
                        *last.borrow_mut() = text;
                    }
                }
            }
        }
    });
    glib::MainLoop::new(None, false).run();
    glib::ExitCode::SUCCESS
}

/// Espera a que Android esté encendido, se conecta y escucha hasta que se apague.
fn connection(link: &Shared, tx: &async_channel::Sender<Ev>) {
    loop {
        // `session_alive` no lanza procesos; `ready` sí, así que se revisa menos seguido
        if !android::session_alive() {
            sleep(Duration::from_secs(3));
            continue;
        }
        if !android::ready() {
            sleep(Duration::from_secs(10));
            continue;
        }
        let l = match Link::connect(true) {
            Ok(l) => Arc::new(l),
            Err(e) => {
                eprintln!("playnix: portapapeles: {e}");
                sleep(Duration::from_secs(60));
                continue;
            }
        };
        let reader = l.reader();
        if let Ok(mut l2) = link.lock() {
            *l2 = Some(l.clone());
        }
        let _ = tx.send_blocking(Ev::Connected);
        if let Ok(r) = reader {
            listen(r, tx);
        }
        if let Ok(mut l2) = link.lock() {
            *l2 = None;
        }
        drop(l);
        sleep(Duration::from_secs(3));
    }
}

/// Mensajes del servidor: 0 = portapapeles, 1 = acuse, 2 = salida HID (se ignoran).
fn listen(mut r: TcpStream, tx: &async_channel::Sender<Ev>) {
    let timeout = Some(Duration::from_secs(5));
    let _ = r.set_read_timeout(timeout);
    loop {
        let mut kind = [0u8];
        match r.read(&mut kind) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if !android::session_alive() {
                    return;
                }
                continue;
            }
            Err(_) => return,
        }
        // el resto del mensaje ya viene en camino
        let _ = r.set_read_timeout(None);
        let ok = match kind[0] {
            0 => read_u32(&mut r).is_some_and(|len| {
                let len = len as usize;
                if len > MAX_TEXT {
                    return false;
                }
                let mut buf = vec![0; len];
                r.read_exact(&mut buf).is_ok()
                    && tx.send_blocking(Ev::Text(String::from_utf8_lossy(&buf).into_owned())).is_ok()
            }),
            1 => r.read_exact(&mut [0; 8]).is_ok(),
            2 => {
                let mut head = [0; 4];
                r.read_exact(&mut head).is_ok()
                    && r.read_exact(&mut vec![0; u16::from_be_bytes([head[2], head[3]]) as usize]).is_ok()
            }
            _ => false,
        };
        if !ok {
            return;
        }
        let _ = r.set_read_timeout(timeout);
    }
}

fn read_u32(r: &mut TcpStream) -> Option<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b).ok()?;
    Some(u32::from_be_bytes(b))
}
