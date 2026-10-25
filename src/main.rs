//! Playnix: Google Play y apps de Android integradas en Linux (sobre Waydroid).
//!
//!   playnix                      instalador / panel
//!   playnix --cli [opciones]     instalar desde la terminal
//!   playnix open <paquete>       abrir una app (con ventana de "Abriendo…" si hace falta)
//!   playnix install-apk <apk>    ventana para instalar un APK
//!   playnix uninstall <paquete>  confirmar y desinstalar una app
//!   playnix integrate            reaplicar la integración con el menú

mod android;
mod apk;
mod cli;
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
    app.run_with_args::<&str>(&[])
}

fn usage() -> ! {
    eprintln!(
        "uso: playnix [--cli [--yes] [--gpu=auto|hw|sw] [--widevine] [--reinit] [--fast-start]]\n\
         \x20      playnix open <paquete> | install-apk <archivo.apk> | uninstall <paquete> | integrate"
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
        Some("apk-info") => match apk::read(std::path::Path::new(&arg(1))) {
            Some(i) => {
                println!("paquete: {}\nversión: {}\nnombre:  {}", i.package, i.version, i.label);
                gtk::glib::ExitCode::SUCCESS
            }
            None => {
                eprintln!("playnix: no es un APK válido");
                gtk::glib::ExitCode::FAILURE
            }
        },
        Some("--version") => {
            println!("playnix {}", env!("CARGO_PKG_VERSION"));
            gtk::glib::ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
