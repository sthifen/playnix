//! Integración con el escritorio: que Google Play y las apps parezcan programas
//! normales y que Android no se note.

use crate::android::{dirs_home, PLAY_STORE};
use gtk::glib;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Apps del sistema Android que no tienen sentido en el menú de un PC.
const HIDDEN: &[&str] = &[
    "com.android.settings",
    "com.android.documentsui",
    "com.android.contacts",
    "com.google.android.contacts",
    "com.android.gallery3d",
    "com.android.deskclock",
    "com.android.calculator2",
    "com.android.camera2",
    "com.android.dialer",
    "com.android.messaging",
    "com.google.android.apps.messaging",
    "com.google.android.googlequicksearchbox",
    "org.lineageos.jelly",
    "org.lineageos.eleven",
    "org.lineageos.etar",
    "org.lineageos.aperture",
    "org.lineageos.recorder",
];

const ICON_SVG: &str = include_str!("../data/playnix.svg");

fn apps_dir() -> PathBuf {
    dirs_home().join(".local/share/applications")
}

pub fn bin_path() -> PathBuf {
    dirs_home().join(".local/bin/playnix")
}

/// Copia Playnix (el AppImage o el binario) a ~/.local/bin/playnix para que los
/// accesos del menú sigan funcionando aunque se borre el archivo descargado.
pub fn install_self() -> Result<(), String> {
    let src = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .ok_or("no encuentro el ejecutable de Playnix")?;
    let dst = bin_path();
    if src != dst {
        std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
        let tmp = dst.with_extension("new");
        std::fs::copy(&src, &tmp).map_err(|e| format!("copiando Playnix: {e}"))?;
        std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    }
    let icon = dirs_home().join(".local/share/icons/hicolor/scalable/apps/playnix.svg");
    std::fs::create_dir_all(icon.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(icon, ICON_SVG).map_err(|e| e.to_string())?;
    Ok(())
}

fn write_if_changed(path: &Path, content: &str) -> bool {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return false;
    }
    std::fs::write(path, content).is_ok()
}

/// Ajusta un .desktop generado por Waydroid. Devuelve true si lo modificó.
fn fix_app_entry(path: &Path, pkg: &str, bin: &str) -> bool {
    let kf = glib::KeyFile::new();
    let flags = glib::KeyFileFlags::KEEP_COMMENTS | glib::KeyFileFlags::KEEP_TRANSLATIONS;
    if kf.load_from_file(path, flags).is_err() {
        return false;
    }
    const G: &str = "Desktop Entry";
    if HIDDEN.contains(&pkg) {
        kf.set_boolean(G, "NoDisplay", true);
    } else {
        kf.set_string(G, "Exec", &format!("{bin} open {pkg}"));
        kf.set_string(G, "StartupNotify", "false");
        let mut actions: Vec<String> = kf
            .string_list(G, "Actions")
            .map(|v| v.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();
        if !actions.iter().any(|a| a == "uninstall") {
            actions.push("uninstall".into());
            kf.set_string(G, "Actions", &format!("{};", actions.join(";")));
        }
        const A: &str = "Desktop Action uninstall";
        kf.set_string(A, "Name", "Uninstall");
        kf.set_locale_string(A, "Name", "es", "Desinstalar");
        kf.set_string(A, "Exec", &format!("{bin} uninstall {pkg}"));
        kf.set_string(A, "Icon", "user-trash-symbolic");
        if pkg == PLAY_STORE {
            kf.set_string(G, "Name", "Google Play");
            kf.set_string(G, "Comment", "Apps and games for Android");
            kf.set_locale_string(G, "Comment", "es", "Apps y juegos de Android");
            kf.set_string(G, "Keywords", "play;store;tienda;android;apps;juegos;");
        }
        let _ = kf.remove_key(G, "NoDisplay");
    }
    write_if_changed(path, &kf.to_data())
}

/// Aplica la integración a todas las apps. La llama el path unit cada vez que
/// Waydroid regenera sus .desktop (lo hace al arrancar y al instalar apps).
pub fn integrate() -> Result<usize, String> {
    let dir = apps_dir();
    let bin = bin_path();
    let bin = bin.to_string_lossy();
    let mut changed = 0;
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(pkg) = name.strip_prefix("waydroid.").and_then(|n| n.strip_suffix(".desktop")) {
            if fix_app_entry(&e.path(), pkg, &bin) {
                changed += 1;
            }
        }
    }
    // Ocultar las entradas genéricas de Waydroid (el usuario no tiene por qué verlas)
    for (file, name, exec) in [
        ("Waydroid.desktop", "Waydroid", "waydroid first-launch"),
        ("waydroid.market.desktop", "Waydroid Market", "waydroid app intent android.intent.action.VIEW %u"),
    ] {
        let content = format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nNoDisplay=true\n");
        write_if_changed(&dir.join(file), &content);
    }
    Ok(changed)
}

/// Accesos propios, manejador de .apk y servicios de usuario.
pub fn install_desktop(fast_start: bool) -> Result<(), String> {
    let dir = apps_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bin = bin_path();
    let bin = bin.to_string_lossy();
    write_if_changed(
        &dir.join("io.github.sthifen.Playnix.desktop"),
        &format!(
            "[Desktop Entry]\nType=Application\nName=Playnix\nComment=Android and Google Play settings\n\
             Comment[es]=Ajustes de Android y Google Play\nIcon=playnix\nExec={bin}\n\
             Categories=System;Settings;\nKeywords=android;waydroid;google;play;\n"
        ),
    );
    write_if_changed(
        &dir.join("io.github.sthifen.Playnix.InstallApk.desktop"),
        &format!(
            "[Desktop Entry]\nType=Application\nName=Install Android app\nName[es]=Instalar app de Android\n\
             Icon=playnix\nExec={bin} install-apk %F\nMimeType=application/vnd.android.package-archive;\n\
             NoDisplay=true\n"
        ),
    );
    // Restos de la versión anterior basada en scripts
    let _ = std::fs::remove_file(dir.join("wd-install-apk.desktop"));
    let _ = Command::new("update-desktop-database").arg(&dir).status();
    let _ = Command::new("xdg-mime")
        .args(["default", "io.github.sthifen.Playnix.InstallApk.desktop", "application/vnd.android.package-archive"])
        .status();

    let units = dirs_home().join(".config/systemd/user");
    std::fs::create_dir_all(&units).map_err(|e| e.to_string())?;
    write_if_changed(
        &units.join("playnix-integrate.path"),
        "[Unit]\nDescription=Playnix: watch Android app shortcuts\n\n[Path]\n\
         PathChanged=%h/.local/share/applications\n\n[Install]\nWantedBy=default.target\n",
    );
    write_if_changed(
        &units.join("playnix-integrate.service"),
        &format!("[Unit]\nDescription=Playnix: integrate Android apps\n\n[Service]\nType=oneshot\nExecStart={bin} integrate\n"),
    );
    write_if_changed(
        &units.join("playnix-session.service"),
        "[Unit]\nDescription=Playnix: start Android in the background\nAfter=graphical-session.target\n\
         PartOf=graphical-session.target\n\n[Service]\nExecStart=/usr/bin/waydroid session start\n\
         ExecStop=/usr/bin/waydroid session stop\n\n[Install]\nWantedBy=graphical-session.target\n",
    );
    let sc = |args: &[&str]| {
        let _ = Command::new("systemctl").arg("--user").args(args).status();
    };
    sc(&["daemon-reload"]);
    sc(&["enable", "--now", "playnix-integrate.path"]);
    set_fast_start(fast_start);
    integrate()?;
    Ok(())
}

pub fn fast_start_enabled() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-enabled", "-q", "playnix-session.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn set_fast_start(on: bool) {
    let _ = Command::new("systemctl")
        .args(["--user", if on { "enable" } else { "disable" }, "playnix-session.service"])
        .status();
}
