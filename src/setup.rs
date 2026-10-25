//! Orquesta la instalación completa. La usan tanto la ventana como `--cli`:
//! emite eventos con un callback para que cada interfaz los muestre a su manera.

use crate::android::{self, Res};
use crate::integrate;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

const ROOT_SETUP: &str = include_str!("../data/root-setup.sh");
const PRIV_HELPER: &str = include_str!("../data/playnix-priv");
const POLICY: &str = include_str!("../data/io.github.sthifen.playnix.policy");
const HWC_SHIM: &[u8] = include_bytes!("../data/hwc-fix/libpnxh.so");
const HWC_PATCH: &str = include_str!("../data/hwc-fix/hwc-patch.py");

#[derive(Debug, Clone)]
pub enum Ev {
    Step(String, String),
    Progress(u32),
    Info(String),
    Warn(String),
    Log(String),
}

#[derive(Debug, Clone)]
pub struct Options {
    pub gpu: String,
    pub arm: String,
    pub widevine: bool,
    pub reinit: bool,
    pub fast_start: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { gpu: "auto".into(), arm: "auto".into(), widevine: false, reinit: false, fast_start: false }
    }
}

/// Escribe los scripts embebidos en un directorio privado del usuario.
fn stage_scripts() -> Res<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("playnix-setup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    for (name, content, mode) in [
        ("root-setup.sh", ROOT_SETUP.as_bytes(), 0o755),
        ("playnix-priv", PRIV_HELPER.as_bytes(), 0o755),
        ("io.github.sthifen.playnix.policy", POLICY.as_bytes(), 0o644),
        ("libpnxh.so", HWC_SHIM, 0o644),
        ("hwc-patch.py", HWC_PATCH.as_bytes(), 0o644),
    ] {
        let p = dir.join(name);
        std::fs::write(&p, content).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// Parte con root. `elevate` es "pkexec" (gráfico, pide la contraseña en una ventana)
/// o "sudo" (terminal).
fn run_root(opts: &Options, elevate: &str, emit: &dyn Fn(Ev)) -> Res<()> {
    let dir = stage_scripts()?;
    let user = std::env::var("USER").map_err(|_| "no sé quién es el usuario")?;
    let mut cmd = Command::new(elevate);
    cmd.arg("/bin/bash")
        .arg(dir.join("root-setup.sh"))
        .args(["--user", &user, "--gpu", &opts.gpu, "--arm", &opts.arm]);
    if opts.widevine {
        cmd.arg("--widevine");
    }
    if opts.reinit {
        cmd.arg("--reinit");
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let mut error = None;
    let mut done = false;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let (tag, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match tag {
            "@@STEP" => {
                let (id, text) = rest.split_once(' ').unwrap_or((rest, ""));
                emit(Ev::Step(id.into(), text.into()));
            }
            "@@PROGRESS" => emit(Ev::Progress(rest.trim().parse().unwrap_or(0))),
            "@@INFO" => emit(Ev::Info(rest.into())),
            "@@WARN" => emit(Ev::Warn(rest.into())),
            "@@ERROR" => error = Some(rest.to_string()),
            "@@DONE" => done = true,
            "@@LOG" => emit(Ev::Log(rest.into())),
            _ => emit(Ev::Log(line.clone())),
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    if let Some(e) = error {
        return Err(e);
    }
    // pkexec devuelve 126/127 si se canceló la ventana de contraseña
    if !done {
        return Err(match status.code() {
            Some(126) | Some(127) => "Se canceló la autorización.".into(),
            _ => "La instalación se interrumpió.".into(),
        });
    }
    Ok(())
}

/// Instalación completa. Al terminar Android queda encendido, listo para certificar.
pub fn install(opts: &Options, elevate: &str, emit: &dyn Fn(Ev)) -> Res<()> {
    run_root(opts, elevate, emit)?;

    emit(Ev::Step("android".into(), "Arrancando Android por primera vez".into()));
    android::ensure_session(Duration::from_secs(240))?;
    android::set_prop("persist.waydroid.multi_windows", "true")?;
    emit(Ev::Info("Cada app se abrirá en su propia ventana".into()));
    android::privileged("setup-android")?;
    emit(Ev::Info("Teclado de la PC configurado".into()));
    android::restart_session(Duration::from_secs(180))?;

    emit(Ev::Step("desktop".into(), "Añadiendo Google Play a tu menú".into()));
    integrate::install_self()?;
    integrate::install_desktop(opts.fast_start)?;
    emit(Ev::Info("Accesos creados".into()));
    Ok(())
}

/// Espera a que Google genere el ID del dispositivo. Si tarda, abre Play Store
/// (eso fuerza el primer contacto con los servidores de Google).
pub fn wait_android_id(emit: &dyn Fn(Ev)) -> Option<String> {
    let _ = android::ensure_session(Duration::from_secs(120));
    for i in 0..60 {
        if let Some(id) = android::android_id() {
            return Some(id);
        }
        if i == 10 {
            emit(Ev::Info("Abriendo Google Play para conectar con Google…".into()));
            android::launch(android::PLAY_STORE);
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    None
}
