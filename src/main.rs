//! Playnix: Google Play y apps de Android integradas en Linux (sobre Waydroid).
//!
//!   playnix                      instalador / panel
//!   playnix --cli [opciones]     instalar desde la terminal
//!   playnix open <paquete>       abrir una app (con ventana de "Abriendo…" si hace falta)
//!   playnix install-apk <apk>    ventana para instalar un APK
//!   playnix uninstall <paquete>  confirmar y desinstalar una app
//!   playnix integrate            reaplicar la integración con el menú
//!   playnix watchdog             reinicia Android si su pantalla se cae (servicio)
//!   playnix clipboard            portapapeles compartido con Android (servicio)
//!   playnix diagnose             resumen para reportar un problema
//!   playnix controls <paquete> <perfil.json> | --gestos | --quitar
//!                                controles de teclado para un juego

mod android;
mod apk;
mod cli;
mod clipboard;
mod controls;
mod integrate;
mod setup;
mod ui_apps;
mod ui_installer;

use adw::prelude::*;
use gtk::gio;
use std::path::PathBuf;

const APP_ID: &str = "io.github.sthifen.Playnix";

enum Mode {
    Installer,
    Open(String),
    InstallApk(Vec<PathBuf>),
    Uninstall(String),
}

fn run_gui(mode: Mode) -> gtk::glib::ExitCode {
    // NON_UNIQUE: se pueden abrir varias ventanas pequeñas a la vez (instalar dos APK, etc.)
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let mode = std::cell::RefCell::new(Some(mode));
    app.connect_activate(move |app| {
        let Some(m) = mode.borrow_mut().take() else { return };
        if let Some(theme) = gtk::gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d)) {
            theme.add_search_path(android::dirs_home().join(".local/share/icons"));
        }
        match m {
            Mode::Installer => ui_installer::build(app),
            Mode::Open(pkg) => ui_apps::open(app, pkg),
            Mode::InstallApk(files) => ui_apps::install_apk(app, files),
            Mode::Uninstall(pkg) => ui_apps::uninstall(app, pkg),
        }
    });
    // Depuración: PLAYNIX_SNAPSHOT=/ruta/base guarda cada ventana como PNG a los 3 s
    if let Some(base) = std::env::var_os("PLAYNIX_SNAPSHOT") {
        let base = PathBuf::from(base);
        app.connect_startup(move |app| {
            let (app, base) = (app.clone(), base.clone());
            gtk::glib::timeout_add_seconds_local_once(3, move || {
                for (i, w) in app.windows().iter().enumerate() {
                    let path = base.with_file_name(format!(
                        "{}-{i}.png",
                        base.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    let paintable = gtk::WidgetPaintable::new(Some(w));
                    let snap = gtk::Snapshot::new();
                    paintable.snapshot(&snap, w.width() as f64, w.height() as f64);
                    let renderer = w.native().and_then(|n| n.renderer());
                    if let (Some(node), Some(r)) = (snap.to_node(), renderer) {
                        let _ = r.render_texture(&node, None).save_to_png(&path);
                        eprintln!("playnix: ventana guardada en {}", path.display());
                    }
                }
            });
        });
    }
    app.run_with_args::<&str>(&[])
}

fn usage() -> ! {
    eprintln!(
        "uso: playnix [--cli [--yes] [--gpu=auto|hw|sw] [--widevine] [--reinit] [--fast-start]]\n\
         \x20      playnix open <paquete> | install-apk <archivo.apk> | uninstall <paquete> | integrate\n\
         \x20      playnix controls <paquete> <perfil.json> | --gestos | --quitar"
    );
    std::process::exit(2)
}

fn main() -> gtk::glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_else(|| usage());
    match args.first().map(String::as_str) {
        None => run_gui(Mode::Installer),
        Some("--cli") => cli::run(&args[1..]),
        Some("open") => run_gui(Mode::Open(arg(1))),
        Some("install-apk") if args.len() > 1 => {
            run_gui(Mode::InstallApk(args[1..].iter().map(PathBuf::from).collect()))
        }
        Some("uninstall") => run_gui(Mode::Uninstall(arg(1))),
        Some("integrate") => match integrate::integrate() {
            Ok(_) => gtk::glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("playnix: {e}");
                gtk::glib::ExitCode::FAILURE
            }
        },
        Some("apk-info") => match {
            let p = PathBuf::from(arg(1));
            if apk::is_bundle(&p) { apk::read_bundle(&p) } else { apk::read(&p) }
        } {
            Some(i) => {
                println!("paquete: {}\nversión: {}\nnombre:  {}\njuego:   {}", i.package, i.version, i.label, if i.game { "sí" } else { "no" });
                gtk::glib::ExitCode::SUCCESS
            }
            None => {
                eprintln!("playnix: no es un APK válido");
                gtk::glib::ExitCode::FAILURE
            }
        },
        Some("controls") => {
            let pkg = arg(1);
            match arg(2).as_str() {
                "--gestos" => match controls::enable_gestures(&pkg) {
                    Ok(()) => {
                        println!("Gestos del mouse activos en {pkg} (F1 dentro de la app para agregar controles)");
                        gtk::glib::ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("playnix: {e}");
                        gtk::glib::ExitCode::FAILURE
                    }
                },
                "--quitar" => {
                    if controls::remove(&pkg) {
                        println!("Controles de {pkg} quitados");
                    }
                    gtk::glib::ExitCode::SUCCESS
                }
                file => match controls::import(&pkg, std::path::Path::new(file)) {
                    Ok(n) => {
                        println!("{n} controles listos para {pkg}: se activan al abrir el juego");
                        gtk::glib::ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("playnix: {e}");
                        gtk::glib::ExitCode::FAILURE
                    }
                },
            }
        }
        Some("diagnose") => {
            print!("{}", android::diagnostics());
            gtk::glib::ExitCode::SUCCESS
        }
        Some("watchdog") => integrate::watchdog(),
        Some("clipboard") => clipboard::run(),
        Some("--version") => {
            println!("playnix {}", env!("CARGO_PKG_VERSION"));
            gtk::glib::ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
