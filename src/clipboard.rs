//! Portapapeles compartido entre Linux y Android (`playnix clipboard`, servicio de
//! usuario): texto e imágenes. Del lado de Android corre un servidor pequeño como
//! shell (data/clip, el mismo APK que trae el proveedor de imágenes) que avisa de
//! cada cambio y recibe lo que se copia en Linux. Del lado de Linux se usa el
//! portapapeles de X11 (XWayland): GNOME no deja leer el de Wayland a un programa sin
//! una ventana con foco, pero lo mantiene sincronizado con el de X11, y ése sí se
//! puede leer y vigilar siempre.

use crate::android;
use crate::controls::scrcpy::{adb, connect_adb, serial};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::Cell;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::sleep;
use std::time::Duration;

const APK: &[u8] = include_bytes!("../data/clip/playnix-clip.apk");
const PACKAGE: &str = "io.github.sthifen.playnix.clip";
const DEVICE_APK: &str = "/data/local/tmp/playnix-clip.apk";
const MAGIC: &[u8] = b"PNXCLIP1";
const TEXT: u8 = 0;
const IMAGE: u8 = 1;
/// Lo mismo que acepta el servidor
const MAX: usize = 32 << 20;

enum Ev {
    /// Recién conectado a Android: se le manda lo que haya copiado en Linux.
    Connected,
    /// Se copió algo en Android.
    Text(String),
    Image(Vec<u8>),
}

/// Mensajes para Android; los escribe un hilo aparte (una imagen grande tarda).
type Shared = Arc<Mutex<Option<mpsc::Sender<(u8, Vec<u8>)>>>>;

fn digest(kind: u8, data: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    kind.hash(&mut h);
    data.hash(&mut h);
    h.finish()
}

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
    let last: Rc<Cell<u64>> = Rc::default();
    let send = {
        let (link, last) = (link.clone(), last.clone());
        move |kind: u8, data: Vec<u8>| {
            if data.is_empty() || data.len() > MAX {
                return;
            }
            let d = digest(kind, &data);
            if last.get() == d {
                return;
            }
            if let Some(l) = link.lock().ok().and_then(|l| l.clone()) {
                let _ = l.send((kind, data));
            }
            last.set(d);
        }
    };

    // Linux → Android: texto si lo hay (copiar celdas o texto con formato también trae
    // una imagen); si no, la imagen (capturas de pantalla, "Copiar imagen").
    let read_linux = {
        let send = send.clone();
        move |b: gdk::Clipboard| {
            let send = send.clone();
            glib::spawn_future_local(async move {
                if let Ok(Some(text)) = b.read_text_future().await {
                    send(TEXT, text.as_bytes().to_vec());
                } else if let Ok(Some(tex)) = b.read_texture_future().await {
                    send(IMAGE, tex.save_to_png_bytes().to_vec());
                }
            });
        }
    };
    board.connect_changed({
        let read_linux = read_linux.clone();
        move |b| {
            if !b.is_local() {
                read_linux(b.clone()); // si es local, lo pusimos nosotros
            }
        }
    });

    // Android → Linux
    glib::spawn_future_local(async move {
        while let Ok(ev) = rx.recv().await {
            match ev {
                Ev::Connected => {
                    last.set(0);
                    if !board.is_local() {
                        read_linux(board.clone());
                    }
                }
                Ev::Text(text) => {
                    let d = digest(TEXT, text.as_bytes());
                    if !text.is_empty() && last.get() != d {
                        board.set_text(&text);
                        last.set(d);
                    }
                }
                Ev::Image(bytes) => {
                    let d = digest(IMAGE, &bytes);
                    if last.get() == d {
                        continue;
                    }
                    match gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) {
                        Ok(tex) => {
                            board.set_texture(&tex);
                            last.set(d);
                        }
                        Err(e) => eprintln!("playnix: portapapeles: imagen de Android ilegible: {e}"),
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
        let mut server = match start_server() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("playnix: portapapeles: {e}");
                sleep(Duration::from_secs(60));
                continue;
            }
        };
        let (Some(stdin), Some(mut stdout)) = (server.stdin.take(), server.stdout.take()) else { continue };
        let server = Arc::new(Mutex::new(server));
        // Si Android se apaga, adb puede quedarse esperando: se corta desde aquí
        let watch = {
            let server = server.clone();
            std::thread::spawn(move || {
                while server.lock().is_ok_and(|mut s| s.try_wait().is_ok_and(|r| r.is_none())) {
                    if !android::session_alive() {
                        if let Ok(mut s) = server.lock() {
                            let _ = s.kill();
                        }
                    }
                    sleep(Duration::from_secs(5));
                }
            })
        };
        if wait_magic(&mut stdout) {
            let (out_tx, out_rx) = mpsc::channel();
            std::thread::spawn(move || writer(stdin, &out_rx));
            if let Ok(mut l) = link.lock() {
                *l = Some(out_tx);
            }
            let _ = tx.send_blocking(Ev::Connected);
            listen(&mut stdout, tx);
            if let Ok(mut l) = link.lock() {
                *l = None; // al soltar el Sender termina el hilo que escribe
            }
        }
        if let Ok(mut s) = server.lock() {
            let _ = s.kill();
            let _ = s.wait();
        }
        let _ = watch.join();
        sleep(Duration::from_secs(3));
    }
}

/// Instala o actualiza el APK si hace falta y arranca el servidor como shell.
fn start_server() -> android::Res<Child> {
    let serial = serial()?;
    connect_adb(&serial)?;
    let local = std::env::temp_dir().join(format!("playnix-clip-{}.apk", std::process::id()));
    std::fs::write(&local, APK).map_err(|e| e.to_string())?;
    let pushed = adb(&serial, &["push", &local.to_string_lossy(), DEVICE_APK]);
    let _ = std::fs::remove_file(&local);
    pushed.ok_or("no se pudo copiar el portapapeles a Android")?;

    // El proveedor de imágenes tiene que estar instalado y ser esta versión
    let installed = adb(&serial, &["shell", "pm", "path", PACKAGE])
        .and_then(|o| o.lines().find_map(|l| l.strip_prefix("package:")).map(|p| p.trim().to_string()));
    let same = installed.is_some_and(|p| adb(&serial, &["shell", "cmp", "-s", DEVICE_APK, &p]).is_some());
    if !same {
        let ok = |o: Option<String>| o.is_some_and(|o| o.contains("Success"));
        if !ok(adb(&serial, &["shell", "pm", "install", "-r", DEVICE_APK])) {
            // firmado con otra llave: se reinstala
            let _ = adb(&serial, &["shell", "pm", "uninstall", PACKAGE]);
            if !ok(adb(&serial, &["shell", "pm", "install", DEVICE_APK])) {
                eprintln!("playnix: portapapeles: no se pudo instalar el proveedor de imágenes (solo texto)");
            }
        }
    }

    Command::new("adb")
        .args(["-s", &serial, "shell", "-T", &format!("CLASSPATH={DEVICE_APK}"), "app_process", "/"])
        .arg("io.github.sthifen.playnix.clip.Server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())
}

/// Salta lo que haya antes de MAGIC (avisos de Android, si los hay).
fn wait_magic(r: &mut impl Read) -> bool {
    let mut seen = Vec::new();
    let mut b = [0u8];
    while r.read_exact(&mut b).is_ok() {
        seen.push(b[0]);
        if seen.ends_with(MAGIC) {
            return true;
        }
        if seen.len() > 1 << 16 {
            return false;
        }
    }
    false
}

fn writer(mut w: ChildStdin, rx: &mpsc::Receiver<(u8, Vec<u8>)>) {
    while let Ok((kind, data)) = rx.recv() {
        let mut head = vec![kind];
        head.extend((data.len() as u32).to_be_bytes());
        if w.write_all(&head).and_then(|_| w.write_all(&data)).and_then(|_| w.flush()).is_err() {
            return;
        }
    }
}

/// Mensajes del servidor: [tipo u8][largo u32][datos].
fn listen(r: &mut impl Read, tx: &async_channel::Sender<Ev>) {
    loop {
        let mut head = [0u8; 5];
        if r.read_exact(&mut head).is_err() {
            return;
        }
        let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        if len > MAX {
            return;
        }
        let mut data = vec![0; len];
        if r.read_exact(&mut data).is_err() {
            return;
        }
        let ev = match head[0] {
            TEXT => Ev::Text(String::from_utf8_lossy(&data).into_owned()),
            IMAGE => Ev::Image(data),
            _ => continue,
        };
        if tx.send_blocking(ev).is_err() {
            return;
        }
    }
}
