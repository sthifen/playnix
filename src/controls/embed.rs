//! Controles dentro de la ventana del juego: libpnxh.so (data/hwc-fix), en el
//! hwcomposer de Waydroid, pone una capa encima de la app y le manda a Playnix el
//! mouse sobre ella y las teclas que pida. Hablan por un socket Unix en la carpeta de
//! datos de Android, que Waydroid monta desde Linux. Formato en pnxhwc.c.

use crate::android;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const HELLO: u32 = 1;
const FRAME: u32 = 2;
const KEYS: u32 = 3;
const LOCK: u32 = 4;

pub enum Ev {
    /// tamaño de la capa en píxeles lógicos
    Size(i32, i32),
    /// posición en la capa
    Motion(f64, f64),
    /// botón de Linux (BTN_LEFT…) y si está presionado
    Button(u32, bool),
    /// eje (0 vertical, 1 horizontal) y valor
    Axis(u32, f64),
    /// código evdev y si está presionada
    Key(u32, bool),
    /// modificadores de xkb: presionados, fijados y bloqueados
    Mods(u32, u32, u32),
    /// movimiento relativo con el mouse fijado
    Rel(f64, f64),
    Leave,
    Enter(f64, f64),
    Focus(bool),
    /// la ventana del juego se cerró (puede volver a abrirse)
    Gone,
}

pub struct Embed {
    out: Mutex<UnixStream>,
}

fn socket_path() -> std::path::PathBuf {
    android::dirs_home().join(".local/share/waydroid/data/playnix/controls.sock")
}

fn fixed(v: u32) -> f64 {
    v as i32 as f64 / 256.0
}

impl Embed {
    /// Se conecta y espera a que la capa esté en la ventana del juego (`timeout`).
    /// None si libpnxh.so no está (Waydroid sin el arreglo) o la ventana no aparece:
    /// entonces los controles van en una ventana aparte, como antes.
    pub fn connect(pkg: &str, timeout: Duration) -> Option<(Embed, (i32, i32), async_channel::Receiver<Ev>)> {
        let mut stream = UnixStream::connect(socket_path()).ok()?;
        let id = format!("waydroid.{pkg}");
        let mut msg = Vec::new();
        msg.extend(HELLO.to_ne_bytes());
        msg.extend((id.len() as u32).to_ne_bytes());
        msg.extend(id.as_bytes());
        stream.write_all(&msg).ok()?;

        // primero llega el tamaño, cuando la ventana del juego existe
        let t0 = Instant::now();
        let size = loop {
            let left = timeout.checked_sub(t0.elapsed())?;
            stream.set_read_timeout(Some(left.max(Duration::from_millis(10)))).ok()?;
            match read_ev(&mut stream)? {
                Ev::Size(w, h) if w > 0 && h > 0 => break (w, h),
                _ => {}
            }
        };
        stream.set_read_timeout(None).ok()?;

        let (tx, rx) = async_channel::unbounded();
        let mut reader = stream.try_clone().ok()?;
        std::thread::spawn(move || {
            while let Some(ev) = read_ev(&mut reader) {
                if tx.send_blocking(ev).is_err() {
                    break;
                }
            }
        });
        Some((Embed { out: Mutex::new(stream) }, size, rx))
    }

    fn send(&self, kind: u32, parts: &[&[u8]]) {
        let len: usize = parts.iter().map(|p| p.len()).sum();
        if let Ok(mut s) = self.out.lock() {
            let _ = s.write_all(&kind.to_ne_bytes());
            let _ = s.write_all(&(len as u32).to_ne_bytes());
            for p in parts {
                let _ = s.write_all(p);
            }
        }
    }

    /// Imagen de la capa: ARGB8888 premultiplicado (como cairo), sin relleno entre filas.
    pub fn frame(&self, w: u32, h: u32, pixels: &[u8]) {
        let mut head = Vec::with_capacity(8);
        head.extend(w.to_ne_bytes());
        head.extend(h.to_ne_bytes());
        self.send(FRAME, &[&head, pixels]);
    }

    /// Teclas (códigos evdev) que van a Playnix y no a Android; `all` las pide todas.
    pub fn keys(&self, all: bool, codes: &[u32]) {
        let mut v = Vec::with_capacity(8 + codes.len() * 4);
        v.extend((all as u32).to_ne_bytes());
        v.extend((codes.len() as u32).to_ne_bytes());
        for c in codes {
            v.extend(c.to_ne_bytes());
        }
        self.send(KEYS, &[&v]);
    }

    /// Quita la capa: el hwcomposer la borra al cerrarse la conexión.
    pub fn close(&self) {
        if let Ok(s) = self.out.lock() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }

    /// Fija el mouse (llega movimiento relativo) o lo suelta.
    pub fn lock(&self, on: bool) {
        self.send(LOCK, &[&(on as u32).to_ne_bytes()]);
    }
}

fn read_ev(r: &mut UnixStream) -> Option<Ev> {
    loop {
        let mut head = [0u8; 8];
        r.read_exact(&mut head).ok()?;
        let kind = u32::from_ne_bytes(head[0..4].try_into().ok()?);
        let len = u32::from_ne_bytes(head[4..8].try_into().ok()?) as usize;
        if len > 64 {
            return None;
        }
        let mut body = vec![0u8; len];
        r.read_exact(&mut body).ok()?;
        let v = |i: usize| body.get(i * 4..i * 4 + 4).map(|b| u32::from_ne_bytes(b.try_into().unwrap())).unwrap_or(0);
        return Some(match kind {
            1 => Ev::Size(v(0) as i32, v(1) as i32),
            2 => Ev::Motion(fixed(v(0)), fixed(v(1))),
            3 => Ev::Button(v(0), v(1) != 0),
            4 => Ev::Axis(v(0), fixed(v(1))),
            5 => Ev::Key(v(0), v(1) != 0),
            6 => Ev::Mods(v(0), v(1), v(2)),
            7 => Ev::Rel(fixed(v(0)), fixed(v(1))),
            8 => Ev::Leave,
            9 => Ev::Enter(fixed(v(0)), fixed(v(1))),
            10 => Ev::Focus(v(0) != 0),
            11 => Ev::Gone,
            _ => continue,
        });
    }
}
