//! Acceso a Waydroid: estado de la sesión, arranque, apps y acciones con root.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub type Res<T> = Result<T, String>;

pub const PRIV: &str = "/usr/libexec/playnix/playnix-priv";
pub const PLAY_STORE: &str = "com.android.vending";

fn output(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn quiet(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// (estado de la sesión, estado del contenedor): RUNNING, STOPPED, FROZEN…
pub fn status() -> (String, String) {
    let out = output("waydroid", &["status"]);
    let field = |name: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix(name))
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    };
    (field("Session:"), field("Container:"))
}

pub fn is_installed() -> bool {
    Path::new("/var/lib/waydroid/waydroid.cfg").exists() && Path::new(PRIV).exists()
}

pub fn session_running() -> bool {
    status().0 == "RUNNING"
}

/// Waydroid congela Android cuando no se usa; esto lo despierta (no requiere root).
pub fn unfreeze() {
    if status().1 == "FROZEN" {
        quiet(
            "gdbus",
            &[
                "call", "--system", "--dest", "id.waydro.Container",
                "--object-path", "/ContainerManager",
                "--method", "id.waydro.ContainerManager.Unfreeze",
            ],
        );
    }
}

fn boot_completed() -> bool {
    output("timeout", &["5", "waydroid", "prop", "get", "sys.boot_completed"])
        .lines()
        .last()
        .map(|l| l.trim() == "1")
        .unwrap_or(false)
}

/// Lista para usarse: sesión activa, contenedor despierto y Android terminó de arrancar.
pub fn ready() -> bool {
    let (s, c) = status();
    s == "RUNNING" && c == "RUNNING" && boot_completed()
}

/// Arranca la sesión desacoplada de este proceso (sobrevive si se cierra Playnix).
fn start_detached() {
    let unit = format!("playnix-waydroid-{}", std::process::id());
    quiet(
        "systemd-run",
        &["--user", "--quiet", "--collect", &format!("--unit={unit}"), "waydroid", "session", "start"],
    );
}

pub fn ensure_session(timeout: Duration) -> Res<()> {
    if !session_running() {
        start_detached();
    }
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        unfreeze();
        if ready() {
            return Ok(());
        }
        sleep(Duration::from_millis(700));
    }
    Err("Android no terminó de arrancar".into())
}

pub fn stop_session() {
    if session_running() {
        quiet("waydroid", &["session", "stop"]);
    }
    sleep(Duration::from_secs(2));
}

pub fn restart_session(timeout: Duration) -> Res<()> {
    stop_session();
    ensure_session(timeout)
}

pub fn set_prop(key: &str, value: &str) -> Res<()> {
    if quiet("waydroid", &["prop", "set", key, value]) {
        Ok(())
    } else {
        Err(format!("no se pudo fijar {key}"))
    }
}

/// (nombre, paquete) de las apps instaladas. Requiere sesión activa.
pub fn list_apps() -> Vec<(String, String)> {
    let out = output("waydroid", &["app", "list"]);
    let mut apps = Vec::new();
    let mut name = String::new();
    for line in out.lines() {
        if let Some(n) = line.strip_prefix("Name: ") {
            name = n.trim().to_string();
        } else if let Some(p) = line.strip_prefix("packageName: ") {
            apps.push((name.clone(), p.trim().to_string()));
        }
    }
    apps
}

pub fn has_app(pkg: &str) -> bool {
    list_apps().iter().any(|(_, p)| p == pkg)
}

pub fn launch(pkg: &str) {
    // `waydroid app launch` arranca la sesión si hace falta y se queda vivo mientras;
    // por eso se lanza desacoplado.
    let _ = Command::new("setsid")
        .args(["-f", "waydroid", "app", "launch", pkg])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn install_apk(path: &Path, pkg: &str) -> Res<bool> {
    let previo = has_app(pkg);
    let ok = Command::new("waydroid")
        .args(["app", "install"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("Waydroid rechazó el archivo".into());
    }
    // installApp es asíncrono dentro de Android
    for _ in 0..60 {
        if has_app(pkg) {
            return Ok(previo);
        }
        sleep(Duration::from_secs(1));
    }
    Err("La app no apareció después de instalar. Puede que no sea compatible con este Android.".into())
}

/// Instala una app partida (.xapk/.apks): extrae sus partes en la carpeta temporal
/// que Android ve como /data/waydroid_tmp y las instala en una sola sesión.
pub fn install_bundle(path: &Path, pkg: &str) -> Res<bool> {
    let previo = has_app(pkg);
    let dir = dirs_home().join(".local/share/waydroid/data/waydroid_tmp/playnix-split");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let r = crate::apk::extract_bundle(path, &dir).and_then(|_| privileged("install-split"));
    let _ = std::fs::remove_dir_all(&dir);
    r.map_err(|_| "Android rechazó la app. Puede que no sea compatible con este equipo.".to_string())?;
    for _ in 0..30 {
        if has_app(pkg) {
            return Ok(previo);
        }
        sleep(Duration::from_secs(1));
    }
    Err("La app no apareció después de instalar.".into())
}

pub fn remove_app(pkg: &str) -> Res<()> {
    if quiet("waydroid", &["app", "remove", pkg]) {
        Ok(())
    } else {
        Err("No se pudo desinstalar".into())
    }
}

/// Acción fija del ayudante con root (polkit la permite sin contraseña).
pub fn privileged(action: &str) -> Res<String> {
    let o = Command::new("pkexec")
        .args([PRIV, action])
        .stderr(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(format!("pkexec {action} falló"))
    }
}

pub fn android_id() -> Option<String> {
    privileged("android-id").ok().filter(|id| !id.is_empty())
}

// ---------------------------------------------------------------------------
// Datos de apps para mostrar

pub fn icon_path(pkg: &str) -> Option<PathBuf> {
    let p = dirs_home().join(".local/share/waydroid/data/icons").join(format!("{pkg}.png"));
    p.exists().then_some(p)
}

/// Nombre visible de una app según su .desktop de Waydroid.
pub fn app_name(pkg: &str) -> String {
    let f = dirs_home().join(format!(".local/share/applications/waydroid.{pkg}.desktop"));
    std::fs::read_to_string(f)
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("Name=").map(str::to_string)))
        .unwrap_or_else(|| if pkg == PLAY_STORE { "Google Play".into() } else { pkg.into() })
}

pub fn dirs_home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

// ---------------------------------------------------------------------------
// Comprobaciones del equipo

pub struct Checks {
    pub wayland: bool,
    pub binder: bool,
    pub nvidia_primary: bool,
    pub free_gb: f64,
    pub already: bool,
}

pub fn run_checks() -> Checks {
    let binder = Path::new("/dev/binderfs/binder").exists()
        || Path::new("/dev/binder").exists()
        || std::fs::read_to_string("/proc/filesystems").map(|s| s.contains("binder")).unwrap_or(false)
        || quiet("modinfo", &["binder_linux"]);
    let free_gb = output("df", &["--output=avail", "-B1", "/var/lib"])
        .lines()
        .nth(1)
        .and_then(|l| l.trim().parse::<f64>().ok())
        .map(|b| b / 1e9)
        .unwrap_or(0.0);
    let cfg = std::fs::read_to_string("/var/lib/waydroid/waydroid.cfg").unwrap_or_default();
    Checks {
        wayland: std::env::var_os("WAYLAND_DISPLAY").is_some(),
        binder,
        nvidia_primary: nvidia_primary(),
        free_gb,
        already: cfg.contains("GAPPS") && cfg.contains("ota.waydro.id"),
    }
}

/// Misma regla que root-setup.sh: sin GPU Mesa, o con NVIDIA como GPU principal del
/// compositor, Android no puede compartir buffers y hay que dibujar por CPU.
fn nvidia_primary() -> bool {
    let driver = |p: PathBuf| {
        std::fs::read_link(p.join("device/driver"))
            .ok()
            .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
    };
    let mut mesa = false;
    let mut nv = false;
    if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let drv = driver(e.path());
            if name.starts_with("renderD")
                && ["i915", "xe", "amdgpu", "radeon", "nouveau"].contains(&drv.as_str())
            {
                mesa = true;
            }
            if name.starts_with("card") && !name.contains('-') && drv == "nvidia" {
                let boot = std::fs::read_to_string(e.path().join("device/boot_vga")).unwrap_or_default();
                nv |= boot.trim() == "1";
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir("/etc/udev/rules.d") {
        for e in rd.flatten() {
            let s = std::fs::read_to_string(e.path()).unwrap_or_default();
            nv |= s.contains("mutter-device-preferred-primary") && s.contains("0x10de");
        }
    }
    !mesa || nv
}
