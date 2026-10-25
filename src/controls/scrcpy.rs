//! Canal para inyectar toques y teclas en Android: scrcpy-server (sin video ni
//! audio) sobre adb. El servidor se conecta de vuelta a un puerto local mediante
//! `adb reverse`, y por ese socket van los mensajes de control.

use crate::android::{self, Res};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

/// Versión exacta del jar incluido: el servidor rechaza clientes de otra versión.
const VERSION: &str = "3.3.1";
const SERVER_JAR: &[u8] = include_bytes!("../../data/controls/scrcpy-server");
const DEVICE_JAR: &str = "/data/local/tmp/playnix-scrcpy.jar";

pub const DOWN: u8 = 0;
pub const UP: u8 = 1;
pub const MOVE: u8 = 2;

pub struct Link {
    serial: String,
    socket: String,
    stream: Mutex<TcpStream>,
    server: Child,
    /// Tamaño de la pantalla de Android, en píxeles. Waydroid lo cambia cuando su
    /// ventana cambia de tamaño (p. ej. al pasar a pantalla completa).
    size: Mutex<(u32, u32)>,
}

fn adb(serial: &str, args: &[&str]) -> Option<String> {
    let o = Command::new("adb").arg("-s").arg(serial).args(args).stderr(Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn serial() -> Res<String> {
    let out = Command::new("waydroid").arg("status").output().map_err(|e| e.to_string())?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("IP address:"))
        .map(|ip| format!("{}:5555", ip.trim()))
        .filter(|s| s.len() > 6)
        .ok_or_else(|| "Android no tiene red todavía".into())
}

/// Conecta adb y comprueba que Android lo autorizó; si no, le da la llave de esta
/// PC con el ayudante de root y reintenta.
fn connect_adb(serial: &str) -> Res<()> {
    for intento in 0..2 {
        for _ in 0..5 {
            android::unfreeze();
            let _ = Command::new("adb").args(["connect", serial]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            if adb(serial, &["shell", "echo", "ok"]).is_some_and(|o| o.trim() == "ok") {
                return Ok(());
            }
            sleep(Duration::from_millis(600));
        }
        if intento == 0 {
            ensure_adb_key();
            android::privileged("adb-auth")?;
            let _ = Command::new("adb").args(["disconnect", serial]).status();
        }
    }
    Err("Android no aceptó la conexión de controles".into())
}

/// adb crea su llave la primera vez que arranca el servidor; el ayudante la necesita.
fn ensure_adb_key() {
    let key = android::dirs_home().join(".android/adbkey");
    if !key.exists() {
        let _ = std::fs::create_dir_all(key.parent().unwrap());
        let _ = Command::new("adb").arg("keygen").arg(&key).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

fn screen_size(serial: &str) -> Res<(u32, u32)> {
    let out = adb(serial, &["shell", "wm", "size"]).ok_or("no se pudo leer el tamaño de la pantalla")?;
    // "Physical size: 1920x982" y, si lo cambiaron, "Override size: …" (manda esa)
    let size = out
        .lines()
        .filter_map(|l| l.split_once(':').map(|(_, v)| v.trim()))
        .filter_map(|v| v.split_once('x'))
        .filter_map(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .last();
    size.ok_or_else(|| format!("tamaño de pantalla inesperado: {out}"))
}

impl Link {
    pub fn connect() -> Res<Link> {
        let serial = serial()?;
        connect_adb(&serial)?;
        let size = screen_size(&serial)?;

        let local = std::env::temp_dir().join(format!("playnix-scrcpy-{}.jar", std::process::id()));
        std::fs::write(&local, SERVER_JAR).map_err(|e| e.to_string())?;
        let pushed = adb(&serial, &["push", &local.to_string_lossy(), DEVICE_JAR]);
        let _ = std::fs::remove_file(&local);
        pushed.ok_or("no se pudo copiar el servidor de controles a Android")?;

        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let scid = format!("{:08x}", (nanos as u32 ^ std::process::id()) & 0x7FFF_FFFF);
        let socket = format!("localabstract:scrcpy_{scid}");
        adb(&serial, &["reverse", &socket, &format!("tcp:{port}")]).ok_or("adb reverse falló")?;

        let server = Command::new("adb")
            .args(["-s", &serial, "shell", &format!("CLASSPATH={DEVICE_JAR}"), "app_process", "/"])
            .args(["com.genymobile.scrcpy.Server", VERSION, &format!("scid={scid}"), "log_level=warn"])
            .args(["video=false", "audio=false", "control=true", "cleanup=false"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;

        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let t0 = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(_) if t0.elapsed() < Duration::from_secs(10) => sleep(Duration::from_millis(50)),
                Err(_) => {
                    let _ = adb(&serial, &["reverse", "--remove", &socket]);
                    return Err("el servidor de controles no respondió".into());
                }
            }
        };
        stream.set_nonblocking(false).map_err(|e| e.to_string())?;
        stream.set_nodelay(true).map_err(|e| e.to_string())?;
        // Primero manda el nombre del dispositivo (64 bytes)
        let mut meta = [0u8; 64];
        stream.read_exact(&mut meta).map_err(|e| e.to_string())?;
        Ok(Link { serial, socket, stream: Mutex::new(stream), server, size: Mutex::new(size) })
    }

    fn send(&self, msg: &[u8]) {
        if let Ok(mut s) = self.stream.lock() {
            let _ = s.write_all(msg);
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size.lock().map(|s| *s).unwrap_or((1920, 1080))
    }

    /// Vuelve a leer el tamaño de la pantalla de Android.
    pub fn refresh_size(&self) {
        if let Ok(size) = screen_size(&self.serial) {
            if let Ok(mut s) = self.size.lock() {
                *s = size;
            }
        }
    }

    /// Toque en píxeles de Android. `id` distingue los dedos (multitáctil).
    pub fn touch(&self, action: u8, id: u64, x: f64, y: f64) {
        let (w, h) = self.size();
        let x = x.clamp(0.0, w as f64 - 1.0) as u32;
        let y = y.clamp(0.0, h as f64 - 1.0) as u32;
        let (pressure, buttons) = if action == UP { (0u16, 0u32) } else { (0xFFFF, 1) };
        let mut m = Vec::with_capacity(32);
        m.extend([2, action]);
        m.extend(id.to_be_bytes());
        m.extend(x.to_be_bytes());
        m.extend(y.to_be_bytes());
        m.extend((w as u16).to_be_bytes());
        m.extend((h as u16).to_be_bytes());
        m.extend(pressure.to_be_bytes());
        m.extend(1u32.to_be_bytes()); // action button: primario
        m.extend(buttons.to_be_bytes());
        self.send(&m);
    }

    /// Tecla de Android (KEYCODE_*), con su estado de modificadores (META_*).
    pub fn key(&self, action: u8, keycode: u32, meta: u32) {
        let mut m = Vec::with_capacity(14);
        m.extend([0, action]);
        m.extend(keycode.to_be_bytes());
        m.extend(0u32.to_be_bytes());
        m.extend(meta.to_be_bytes());
        self.send(&m);
    }

    /// Texto tal cual (acentos, ñ, símbolos que no tienen tecla en Android).
    pub fn text(&self, text: &str) {
        let bytes = text.as_bytes();
        let mut m = Vec::with_capacity(5 + bytes.len());
        m.push(1);
        m.extend((bytes.len() as u32).to_be_bytes());
        m.extend(bytes);
        self.send(&m);
    }

    /// Rueda del mouse en un punto (valores de -1 a 1 por paso).
    pub fn scroll(&self, x: f64, y: f64, h: f64, v: f64) {
        let (sw, sh) = self.size();
        let fixed = |f: f64| ((f.clamp(-1.0, 1.0) * 32767.0) as i16).to_be_bytes();
        let mut m = Vec::with_capacity(21);
        m.push(3);
        m.extend((x.max(0.0) as u32).to_be_bytes());
        m.extend((y.max(0.0) as u32).to_be_bytes());
        m.extend((sw as u16).to_be_bytes());
        m.extend((sh as u16).to_be_bytes());
        m.extend(fixed(h));
        m.extend(fixed(v));
        m.extend(0u32.to_be_bytes());
        self.send(&m);
    }

    /// Si el juego está en ventana flotante dentro de Android, lo pasa a pantalla
    /// completa con F11 (atajo de Waydroid): los controles necesitan el juego entero.
    /// Devuelve si el juego quedó en pantalla completa.
    pub fn fullscreen(&self, pkg: &str) -> bool {
        let mode = || {
            let out = adb(&self.serial, &["shell", "dumpsys", "activity", "activities"]).unwrap_or_default();
            let task = format!(":{pkg} ");
            out.lines()
                .find(|l| l.contains("* Task{") && l.contains(&task))
                .and_then(|l| l.split_whitespace().find_map(|w| w.strip_prefix("mode=")).map(str::to_string))
                .unwrap_or_default()
        };
        if mode() != "freeform" {
            return mode() == "fullscreen";
        }
        self.key(DOWN, 141, 0);
        self.key(UP, 141, 0);
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if mode() == "fullscreen" {
                // que Waydroid termine de cambiar su ventana antes de poner la capa
                std::thread::sleep(std::time::Duration::from_millis(500));
                return true;
            }
        }
        false
    }

    /// ¿Sigue corriendo la app? (para cerrar los controles cuando se cierra el juego)
    pub fn running(&self, pkg: &str) -> bool {
        adb(&self.serial, &["shell", "pidof", pkg]).is_some_and(|o| !o.trim().is_empty())
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = adb(&self.serial, &["reverse", "--remove", &self.socket]);
    }
}
