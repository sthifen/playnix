//! Ventanas pequeñas: abrir una app, instalar un APK, desinstalar.

use crate::android;
use crate::apk;
use crate::controls;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::path::PathBuf;
use std::time::Duration;

fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    glib::spawn_future_local(async move {
        if let Ok(v) = gio::spawn_blocking(work).await {
            done(v);
        }
    });
}

fn small_window(app: &adw::Application, title: &str, content: &impl IsA<gtk::Widget>) -> adw::ApplicationWindow {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
    toolbar.set_content(Some(content));
    adw::ApplicationWindow::builder()
        .application(app)
        .title(title)
        .icon_name("playnix")
        .default_width(420)
        .resizable(false)
        .content(&toolbar)
        .build()
}

fn set_app_icon(page: &adw::StatusPage, pkg: &str) {
    match android::icon_path(pkg).and_then(|p| gdk::Texture::from_filename(p).ok()) {
        Some(t) => page.set_paintable(Some(&t)),
        None => page.set_icon_name(Some("playnix")),
    }
}

fn pill(label: &str, suggested: bool) -> gtk::Button {
    let b = gtk::Button::builder().label(label).halign(gtk::Align::Center).build();
    b.add_css_class("pill");
    if suggested {
        b.add_css_class("suggested-action");
    }
    b
}

// --- Abrir una app ------------------------------------------------------------

/// Si Android ya está encendido abre la app al instante; si no, muestra una ventana
/// "Abriendo…" mientras arranca, para que no parezca que no pasa nada.
pub fn open(app: &adw::Application, pkg: String) {
    // Por si el servicio no arrancó con la sesión (no hace nada si ya corre)
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "start", "--no-block", "playnix-clipboard.service"])
        .stderr(std::process::Stdio::null())
        .status();
    if android::session_running() && android::gpu_mismatch() {
        android::stop_session(); // se vuelve a arrancar abajo, ya en el modo correcto
    }
    if android::session_running() {
        android::unfreeze();
        android::launch(&pkg);
        with_controls(app, pkg);
        return;
    }
    let name = android::app_name(&pkg);
    let spinner = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build();
    let page = adw::StatusPage::builder()
        .title(format!("Abriendo {name}…"))
        .description("Preparando Android, solo tarda la primera vez")
        .child(&spinner)
        .build();
    page.add_css_class("compact");
    set_app_icon(&page, &pkg);
    let win = small_window(app, &name, &page);
    win.set_default_height(320);
    win.present();

    let (w, a, p) = (win.clone(), app.clone(), pkg.clone());
    background(
        move || {
            android::ensure_session(Duration::from_secs(150))?;
            android::launch(&pkg);
            // dar tiempo a que aparezca la ventana de la app antes de cerrar esta
            std::thread::sleep(Duration::from_secs(3));
            Ok::<_, String>(())
        },
        move |r| match r {
            Ok(()) => {
                with_controls(&a, p);
                w.close();
            }
            Err(e) => {
                page.set_title("No se pudo abrir");
                page.set_description(Some(&format!("{e}. Inténtalo de nuevo o abre Playnix para repararlo.")));
                page.set_child(None::<&gtk::Widget>);
            }
        },
    );
}

/// Pone la capa de controles encima de los juegos (con sus teclas si tienen perfil;
/// sin perfil, los gestos del mouse) y de cualquier app con perfil.
fn with_controls(app: &adw::Application, pkg: String) {
    // `playnix open` termina cuando no queda nada pendiente: que espere a esto
    let hold_screen = app.hold();
    background(
        {
            let pkg = pkg.clone();
            move || controls::keep_on_screen(&pkg)
        },
        move |()| drop(hold_screen),
    );
    let profile = controls::load(&pkg);
    let hold = app.hold();
    let a = app.clone();
    background(
        {
            let pkg = pkg.clone();
            move || profile.or_else(|| controls::is_game(&pkg).then(controls::profile::Profile::empty))
        },
        move |profile| {
            drop(hold);
            if let Some(profile) = profile {
                start_controls(&a, pkg, profile);
            }
        },
    );
}

fn start_controls(app: &adw::Application, pkg: String, profile: controls::profile::Profile) {
    let a = app.clone();
    controls::start(app, pkg.clone(), profile, move |e| {
        let page = adw::StatusPage::builder()
            .icon_name("input-keyboard-symbolic")
            .title("Los controles de teclado no se activaron")
            .description(format!("{e}. El juego sigue abierto; puedes jugar con el mouse."))
            .build();
        page.add_css_class("compact");
        let win = small_window(&a, &android::app_name(&pkg), &page);
        win.present();
    });
}

// --- Instalar un APK ------------------------------------------------------------

pub fn install_apk(app: &adw::Application, files: Vec<PathBuf>) {
    let Some(file) = files.into_iter().next() else { return };
    let bundle = apk::is_bundle(&file);
    let info = if bundle { apk::read_bundle(&file) } else { apk::read(&file) };
    let name = info
        .as_ref()
        .filter(|i| !i.label.is_empty())
        .map(|i| i.label.clone())
        .unwrap_or_else(|| apk::pretty_filename(&file));
    let size = std::fs::metadata(&file).map(|m| m.len() as f64 / 1e6).unwrap_or(0.0);

    let install = pill("Instalar", true);
    let page = adw::StatusPage::builder().icon_name("playnix").title(&name).child(&install).build();
    let win = small_window(app, "Instalar app de Android", &page);
    win.set_default_height(460);

    let Some(info) = info else {
        page.set_icon_name(Some("dialog-error-symbolic"));
        page.set_description(Some("Este archivo no es una app de Android válida."));
        page.set_child(None::<&gtk::Widget>);
        win.present();
        return;
    };
    let mut desc = String::new();
    if !info.version.is_empty() {
        desc.push_str(&format!("Versión {} · ", info.version));
    }
    desc.push_str(&format!("{size:.1} MB\n{}", info.package));
    page.set_description(Some(&desc));
    win.present();

    let pkg = info.package.clone();
    let (p, w) = (page.clone(), win.clone());
    install.connect_clicked(move |_| {
        let spinner = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build();
        p.set_child(Some(&spinner));
        p.set_description(Some("Instalando…"));
        let (file, pkg, p, w) = (file.clone(), pkg.clone(), p.clone(), w.clone());
        background(
            {
                let pkg = pkg.clone();
                move || {
                    android::ensure_session(Duration::from_secs(150))?;
                    if bundle {
                        android::install_bundle(&file, &pkg)
                    } else {
                        android::install_apk(&file, &pkg)
                    }
                }
            },
            move |r| match r {
                Ok(updated) => {
                    let shown = android::app_name(&pkg);
                    if shown != pkg {
                        p.set_title(&shown);
                    }
                    set_app_icon(&p, &pkg);
                    p.set_description(Some(if updated {
                        "Actualizada. Ya está en tu menú de aplicaciones."
                    } else {
                        "Instalada. Ya aparece en tu menú de aplicaciones."
                    }));
                    let row = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
                    let close = pill("Cerrar", false);
                    let open = pill("Abrir", true);
                    row.append(&close);
                    row.append(&open);
                    p.set_child(Some(&row));
                    let w2 = w.clone();
                    close.connect_clicked(move |_| w2.close());
                    open.connect_clicked(move |_| {
                        android::launch(&pkg);
                        w.close();
                    });
                }
                Err(e) => {
                    p.set_icon_name(Some("dialog-error-symbolic"));
                    p.set_title("No se pudo instalar");
                    p.set_description(Some(&e));
                    p.set_child(None::<&gtk::Widget>);
                }
            },
        );
    });
}

// --- Desinstalar ----------------------------------------------------------------

pub fn uninstall(app: &adw::Application, pkg: String) {
    let name = android::app_name(&pkg);
    let dialog = adw::AlertDialog::new(
        Some(&format!("¿Desinstalar {name}?")),
        Some("Se borrarán la app y sus datos."),
    );
    dialog.add_response("cancel", "Cancelar");
    dialog.add_response("remove", "Desinstalar");
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    // AlertDialog necesita una ventana padre; una invisible basta
    let host = adw::ApplicationWindow::builder().application(app).title(&name).default_width(1).default_height(1).build();
    host.set_opacity(0.0);
    host.present();
    let h = host.clone();
    dialog.connect_response(None, move |_, resp| {
        if resp != "remove" {
            h.close();
            return;
        }
        let (pkg, h) = (pkg.clone(), h.clone());
        background(
            move || {
                android::ensure_session(Duration::from_secs(150))?;
                android::remove_app(&pkg)
            },
            move |_| h.close(),
        );
    });
    dialog.present(Some(&host));
}
