//! Ventana principal. Pensada para alguien que solo quiere jugar: un botón para
//! instalar, un paso para Google y listo. Lo técnico queda en "Opciones avanzadas"
//! y en el menú.

use crate::android;
use crate::integrate;
use crate::setup::{self, Ev, Options};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

const REGISTER_URL: &str = "https://www.google.com/android/uncertified";

struct Ui {
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    opts: RefCell<Options>,
    installing: Cell<bool>,
}

impl Ui {
    fn show(&self, name: &str) {
        self.stack.set_visible_child_name(name);
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

fn link(label: &str) -> gtk::Button {
    gtk::Button::builder().label(label).halign(gtk::Align::Center).css_classes(["flat", "dim-label"]).build()
}

fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(spacing).halign(gtk::Align::Center).build()
}

fn page(icon: &str, title: &str, desc: &str, child: &impl IsA<gtk::Widget>) -> adw::StatusPage {
    adw::StatusPage::builder().icon_name(icon).title(title).description(desc).child(child).build()
}

fn play_icon(p: &adw::StatusPage) {
    if let Some(t) = android::icon_path(android::PLAY_STORE).and_then(|f| gdk::Texture::from_filename(f).ok()) {
        p.set_paintable(Some(&t));
    }
}

fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    glib::spawn_future_local(async move {
        if let Ok(v) = gio::spawn_blocking(work).await {
            done(v);
        }
    });
}

pub fn build(app: &adw::Application) {
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(300)
        .vexpand(true)
        .build();
    let header = adw::HeaderBar::builder().show_title(false).build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Playnix")
        .icon_name("playnix")
        .default_width(480)
        .default_height(600)
        .content(&toolbar)
        .build();
    let ui = Rc::new(Ui { window, stack, opts: RefCell::new(Options::default()), installing: Cell::new(false) });

    let installed = android::is_installed();
    welcome_page(&ui, installed);
    progress_page(&ui);
    google_page(&ui);
    done_page(&ui);
    if installed {
        home_page(&ui);
        header.pack_end(&menu_button(&ui));
        ui.show("home");
    } else {
        ui.show("welcome");
    }
    ui.window.present();
}

// --- Bienvenida: un solo botón ---------------------------------------------

fn welcome_page(ui: &Rc<Ui>, installed: bool) {
    let install = pill(if installed { "Reparar" } else { "Instalar" }, true);
    install.set_width_request(200);
    let advanced = link("Opciones avanzadas");
    let b = vbox(12);
    b.append(&install);
    b.append(&advanced);
    let p = page(
        "playnix",
        "Playnix",
        "Juega y usa apps de Android en tu PC.\nCon Google Play, como en tu celular.",
        &b,
    );
    ui.stack.add_named(&p, Some("welcome"));

    let u = ui.clone();
    advanced.connect_clicked(move |_| advanced_dialog(&u));
    let u = ui.clone();
    install.connect_clicked(move |b| {
        b.set_sensitive(false);
        let (u, b) = (u.clone(), b.clone());
        // Las comprobaciones son silenciosas: solo se muestran si algo impide instalar
        background(android::run_checks, move |c| {
            b.set_sensitive(true);
            let problema = if !c.wayland {
                Some("Playnix necesita una sesión Wayland.\nCierra sesión y entra con GNOME o KDE Plasma (Wayland).")
            } else if !c.binder {
                Some("El kernel de tu sistema no es compatible con Android.\nInstala un kernel como linux-zen y vuelve a intentarlo.")
            } else if c.free_gb < 4.0 && !c.already {
                Some("No hay espacio suficiente.\nLibera al menos 4 GB y vuelve a intentarlo.")
            } else {
                None
            };
            match problema {
                Some(m) => show_error(&u, m),
                None => u.show("progress"),
            }
        });
    });
}

fn advanced_dialog(ui: &Rc<Ui>) {
    let dialog = adw::PreferencesDialog::builder().title("Opciones avanzadas").build();
    let pg = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::builder()
        .description("No hace falta tocar nada: Playnix elige lo mejor para tu PC.")
        .build();
    let o = ui.opts.borrow().clone();
    let gpu = adw::ComboRow::builder()
        .title("Gráficos")
        .model(&gtk::StringList::new(&["Automático", "Aceleración por GPU", "Modo compatible (CPU)"]))
        .selected(match o.gpu.as_str() { "hw" => 1, "sw" => 2, _ => 0 })
        .build();
    let fast = adw::SwitchRow::builder()
        .title("Arranque rápido")
        .subtitle("Las apps abren al instante; usa más memoria")
        .active(o.fast_start)
        .build();
    let widevine = adw::SwitchRow::builder()
        .title("Video protegido")
        .subtitle("Para Netflix, Disney+ y similares")
        .active(o.widevine)
        .build();
    let reinit = adw::SwitchRow::builder()
        .title("Reinstalar desde cero")
        .subtitle("Borra las apps y datos de Android")
        .active(o.reinit)
        .visible(android::run_checks().already)
        .build();
    for r in [gpu.upcast_ref::<gtk::Widget>(), fast.upcast_ref(), widevine.upcast_ref(), reinit.upcast_ref()] {
        g.add(r);
    }
    pg.add(&g);
    dialog.add(&pg);
    let u = ui.clone();
    dialog.connect_closed(move |_| {
        let mut o = u.opts.borrow_mut();
        o.gpu = ["auto", "hw", "sw"][gpu.selected().min(2) as usize].into();
        o.fast_start = fast.is_active();
        o.widevine = widevine.is_active();
        o.reinit = reinit.is_active();
    });
    dialog.present(Some(&ui.window));
}

// --- Instalando: una frase y una barra ------------------------------------------

/// Texto amable para cada paso técnico.
fn friendly(step: &str) -> &'static str {
    match step {
        "deps" => "Preparando tu sistema…",
        "image" => "Descargando Android…",
        "gpu" | "extras" | "helper" => "Configurando…",
        "android" => "Encendiendo Android por primera vez…",
        "desktop" => "Casi listo…",
        _ => "Instalando…",
    }
}

fn progress_page(ui: &Rc<Ui>) {
    let bar = gtk::ProgressBar::builder().width_request(300).build();
    let hint = gtk::Label::builder()
        .label("Te pediremos tu contraseña una vez")
        .css_classes(["dim-label"])
        .build();
    let log = gtk::TextView::builder().editable(false).monospace(true).wrap_mode(gtk::WrapMode::WordChar).build();
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(160)
        .min_content_width(380)
        .child(&log)
        .visible(false)
        .build();
    scroll.add_css_class("card");
    let details = link("Ver detalles");
    let b = vbox(16);
    b.append(&bar);
    b.append(&hint);
    b.append(&details);
    b.append(&scroll);
    let p = page("playnix", "Instalando…", "Esto tarda unos minutos.\nPuedes seguir usando tu PC.", &b);
    ui.stack.add_named(&p, Some("progress"));

    let s = scroll.clone();
    details.connect_clicked(move |d| {
        s.set_visible(!s.is_visible());
        d.set_label(if s.is_visible() { "Ocultar detalles" } else { "Ver detalles" });
    });

    let u = ui.clone();
    ui.stack.connect_visible_child_name_notify(move |st| {
        if st.visible_child_name().as_deref() != Some("progress") || u.installing.replace(true) {
            return;
        }
        u.window.set_deletable(false);
        let (tx, rx) = async_channel::unbounded::<Result<Ev, Option<String>>>();
        let opts = u.opts.borrow().clone();
        std::thread::spawn(move || {
            let t = tx.clone();
            let r = setup::install(&opts, "pkexec", &move |ev| {
                let _ = t.send_blocking(Ok(ev));
            });
            let _ = tx.send_blocking(Err(r.err()));
        });

        let (u, p, bar, hint, log) = (u.clone(), p.clone(), bar.clone(), hint.clone(), log.clone());
        glib::spawn_future_local(async move {
            let determinate = Rc::new(Cell::new(false));
            let pulse = glib::timeout_add_local(Duration::from_millis(120), {
                let (bar, det) = (bar.clone(), determinate.clone());
                move || {
                    if !det.get() {
                        bar.pulse();
                    }
                    glib::ControlFlow::Continue
                }
            });
            let append = |t: &str| {
                let buf = log.buffer();
                buf.insert(&mut buf.end_iter(), &format!("{t}\n"));
            };
            while let Ok(msg) = rx.recv().await {
                match msg {
                    Ok(Ev::Step(id, text)) => {
                        p.set_title(friendly(&id));
                        hint.set_label(if id == "image" { "Solo la primera vez · ~1.4 GB" } else { "" });
                        determinate.set(false);
                        append(&format!("== {text}"));
                    }
                    Ok(Ev::Progress(n)) => {
                        determinate.set(true);
                        bar.set_fraction(n as f64 / 100.0);
                        hint.set_label(&format!("{n}% · solo la primera vez"));
                    }
                    Ok(Ev::Info(t)) | Ok(Ev::Log(t)) => append(&t),
                    Ok(Ev::Warn(t)) => append(&format!("! {t}")),
                    Err(r) => {
                        pulse.remove();
                        u.window.set_deletable(true);
                        u.installing.set(false);
                        match r {
                            None => u.show("google"),
                            Some(e) => {
                                append(&format!("ERROR: {e}"));
                                show_error(&u, &e);
                            }
                        }
                        return;
                    }
                }
            }
        });
    });
}

fn show_error(ui: &Rc<Ui>, msg: &str) {
    let retry = pill("Volver a intentar", true);
    let p = page("dialog-warning-symbolic", "No se pudo completar", msg, &retry);
    if let Some(old) = ui.stack.child_by_name("error") {
        ui.stack.remove(&old);
    }
    ui.stack.add_named(&p, Some("error"));
    ui.show("error");
    let u = ui.clone();
    retry.connect_clicked(move |_| u.show("welcome"));
}

// --- Google: un botón ----------------------------------------------------------

fn google_page(ui: &Rc<Ui>) {
    let go = pill("Continuar con Google", true);
    go.set_width_request(240);
    go.set_sensitive(false);
    let waiting = gtk::Spinner::builder().spinning(true).build();
    let instructions = gtk::Label::builder()
        .label("En la página, <b>pega este código</b> y pulsa <b>Registrar</b>:")
        .use_markup(true)
        .justify(gtk::Justification::Center)
        .visible(false)
        .build();
    let code_text = gtk::Label::builder().selectable(true).css_classes(["title-4", "monospace"]).build();
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copiar código")
        .css_classes(["circular", "flat"])
        .valign(gtk::Align::Center)
        .build();
    let code = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).visible(false).build();
    code.add_css_class("card");
    code_text.set_margin_start(14);
    code_text.set_margin_top(6);
    code_text.set_margin_bottom(6);
    copy.set_margin_end(6);
    code.append(&code_text);
    code.append(&copy);
    let toast_overlay = adw::ToastOverlay::new();
    let finished = pill("Ya terminé", false);
    finished.set_visible(false);
    let skip = link("Hacerlo después");
    let b = vbox(14);
    b.append(&waiting);
    b.append(&go);
    b.append(&instructions);
    b.append(&code);
    b.append(&finished);
    b.append(&skip);
    let p = page(
        "playnix",
        "Último paso",
        "Conecta tu cuenta de Google para poder descargar apps y juegos.",
        &b,
    );
    play_icon(&p);
    toast_overlay.set_child(Some(&p));
    ui.stack.add_named(&toast_overlay, Some("google"));

    let id = Rc::new(RefCell::new(String::new()));
    let (i, t) = (id.clone(), toast_overlay.clone());
    copy.connect_clicked(move |b| {
        b.clipboard().set_text(&i.borrow());
        t.add_toast(adw::Toast::builder().title("Código copiado").timeout(2).build());
    });
    let w = ui.window.clone();
    let (i, ins, cd, ct, fin) = (id.clone(), instructions.clone(), code.clone(), code_text.clone(), finished.clone());
    go.connect_clicked(move |g| {
        g.clipboard().set_text(&i.borrow());
        gtk::UriLauncher::new(REGISTER_URL).launch(Some(&w), gio::Cancellable::NONE, |_| {});
        g.remove_css_class("suggested-action");
        ins.set_visible(true);
        ct.set_label(&i.borrow());
        cd.set_visible(true);
        fin.add_css_class("suggested-action");
        fin.set_visible(true);
    });
    let u = ui.clone();
    finished.connect_clicked(move |b| {
        b.set_sensitive(false);
        b.set_label("Aplicando…");
        let u = u.clone();
        background(|| android::restart_session(Duration::from_secs(120)), move |_| u.show("done"));
    });
    let u = ui.clone();
    skip.connect_clicked(move |_| u.show("done"));

    let started = Rc::new(Cell::new(false));
    let p2 = p.clone();
    ui.stack.connect_visible_child_name_notify(move |s| {
        if s.visible_child_name().as_deref() != Some("google") || started.replace(true) {
            return;
        }
        play_icon(&p2); // el icono de Play Store existe recién tras el primer arranque
        let (go, waiting, id, p2, started) = (go.clone(), waiting.clone(), id.clone(), p2.clone(), started.clone());
        background(|| setup::wait_android_id(&|_| {}), move |r| {
            waiting.set_visible(false);
            match r {
                Some(v) => {
                    id.replace(v);
                    go.set_sensitive(true);
                }
                None => {
                    started.set(false);
                    p2.set_description(Some("Google no responde todavía. Revisa tu internet; puedes hacerlo después desde Playnix."));
                }
            }
        });
    });
}

// --- Final, inicio y menú ---------------------------------------------------------

fn open_play(ui: &Rc<Ui>) -> gtk::Button {
    let b = pill("Abrir Google Play", true);
    b.set_width_request(240);
    let w = ui.window.clone();
    b.connect_clicked(move |_| {
        android::launch(android::PLAY_STORE);
        w.close();
    });
    b
}

fn done_page(ui: &Rc<Ui>) {
    let p = page(
        "playnix",
        "¡Listo para jugar!",
        "Busca «Google Play» en tus aplicaciones cuando quieras.\nLo que instales aparecerá ahí, como cualquier programa.",
        &open_play(ui),
    );
    ui.stack.add_named(&p, Some("done"));
    let p2 = p.clone();
    ui.stack.connect_visible_child_name_notify(move |s| {
        if s.visible_child_name().as_deref() == Some("done") {
            play_icon(&p2);
        }
    });
}

fn home_page(ui: &Rc<Ui>) {
    let p = page("playnix", "Google Play está listo", "Tus apps están en el menú de aplicaciones.", &open_play(ui));
    play_icon(&p);
    ui.stack.add_named(&p, Some("home"));
}

fn menu_button(ui: &Rc<Ui>) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    menu.append(Some("Arranque rápido"), Some("win.fast-start"));
    menu.append(Some("Volver a conectar Google"), Some("win.google"));
    menu.append(Some("Ajustes de Android"), Some("win.android-settings"));
    menu.append(Some("Reparar instalación"), Some("win.repair"));

    let fast = gio::SimpleAction::new_stateful("fast-start", None, &integrate::fast_start_enabled().to_variant());
    fast.connect_activate(|a, _| {
        let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
        integrate::set_fast_start(on);
        a.set_state(&on.to_variant());
    });
    let u = ui.clone();
    let google = gio::SimpleAction::new("google", None);
    google.connect_activate(move |_, _| u.show("google"));
    let settings = gio::SimpleAction::new("android-settings", None);
    settings.connect_activate(|_, _| android::launch("com.android.settings"));
    let u = ui.clone();
    let repair = gio::SimpleAction::new("repair", None);
    repair.connect_activate(move |_, _| u.show("welcome"));
    for a in [&fast, &google, &settings, &repair] {
        ui.window.add_action(a);
    }
    gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).build()
}
