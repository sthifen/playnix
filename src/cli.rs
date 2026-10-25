//! Instalación en modo texto: `playnix --cli` (lo usa el comando de una línea).

use crate::android;
use crate::setup::{self, Ev, Options};
use gtk::glib::ExitCode;
use std::io::Write;
use std::process::Command;

const REGISTER_URL: &str = "https://www.google.com/android/uncertified";

pub fn run(args: &[String]) -> ExitCode {
    let mut opts = Options::default();
    let mut yes = false;
    for a in args {
        match a.as_str() {
            "--yes" | "-y" => yes = true,
            "--widevine" => opts.widevine = true,
            "--reinit" => opts.reinit = true,
            "--fast-start" => opts.fast_start = true,
            s if s.starts_with("--gpu=") => opts.gpu = s[6..].into(),
            s if s.starts_with("--arm=") => opts.arm = s[6..].into(),
            _ => {
                eprintln!("opción desconocida: {a}");
                return ExitCode::from(2);
            }
        }
    }
    if unsafe { libc_geteuid() } == 0 {
        eprintln!("Córrelo como tu usuario normal (sin sudo); Playnix pedirá la contraseña cuando la necesite.");
        return ExitCode::FAILURE;
    }

    println!("\x1b[1mPlaynix\x1b[0m — Google Play y apps de Android integradas en Linux\n");
    let c = android::run_checks();
    let mark = |ok: bool| if ok { "\x1b[32m✓\x1b[0m" } else { "\x1b[31m✗\x1b[0m" };
    println!("  {} Sesión Wayland", mark(c.wayland));
    println!("  {} Kernel con binder", mark(c.binder));
    println!("  {} Espacio libre: {:.0} GB", mark(c.free_gb >= 4.0 || c.already), c.free_gb);
    if c.nvidia_primary {
        println!("  \x1b[33m•\x1b[0m NVIDIA es la GPU principal: se usará el modo compatible");
    }
    if c.already {
        println!("  \x1b[32m✓\x1b[0m Android ya instalado: se conservarán tus apps");
    }
    if !c.wayland || !c.binder {
        eprintln!("\nTu sistema no cumple los requisitos.");
        return ExitCode::FAILURE;
    }
    if !yes && !ask("\n¿Continuar con la instalación?") {
        return ExitCode::SUCCESS;
    }

    let emit = |ev: Ev| match ev {
        Ev::Step(_, t) => println!("\n\x1b[1;36m==> {t}\x1b[0m"),
        Ev::Progress(p) => {
            print!("\r    descargando… {p:3}%");
            let _ = std::io::stdout().flush();
            if p >= 100 {
                println!();
            }
        }
        Ev::Info(t) => println!("    {t}"),
        Ev::Warn(t) => println!("    \x1b[33m! {t}\x1b[0m"),
        Ev::Log(_) => {}
    };
    if let Err(e) = setup::install(&opts, "sudo", &emit) {
        eprintln!("\n\x1b[31m✗ {e}\x1b[0m");
        return ExitCode::FAILURE;
    }

    println!("\n\x1b[1;36m==> Conecta tu cuenta de Google\x1b[0m");
    println!("    Esperando el código de este equipo…");
    match setup::wait_android_id(&emit) {
        Some(id) => {
            let copied = Command::new("sh")
                .args(["-c", &format!("printf %s {id} | wl-copy")])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            println!("\n    Código: \x1b[1m{id}\x1b[0m{}", if copied { "  (copiado)" } else { "" });
            println!("    1. Inicia sesión en la página que se abrirá");
            println!("    2. Pega el código y pulsa «Registrar»");
            let _ = Command::new("xdg-open").arg(REGISTER_URL).status();
            ask("    3. ¿Listo?");
            println!("    Aplicando…");
            let _ = android::restart_session(std::time::Duration::from_secs(120));
        }
        None => println!("    Google no respondió; puedes registrar después desde Playnix."),
    }
    println!("\n\x1b[1;32m✓ ¡Listo!\x1b[0m Abre «Google Play» desde tu menú de aplicaciones.");
    ExitCode::SUCCESS
}

fn ask(q: &str) -> bool {
    print!("{q} [S/n] ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    let s = s.trim().to_lowercase();
    s.is_empty() || s.starts_with('s') || s.starts_with('y')
}

unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}
