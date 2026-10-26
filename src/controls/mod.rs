//! Controles de teclado y mouse para juegos: un perfil por app convierte teclas en
//! toques sobre la pantalla. `playnix open` pone la capa en todas las apps (con o
//! sin perfil: los gestos del mouse sirven siempre); los juegos van en pantalla completa.

mod embed;
mod overlay;
mod pointer;
pub mod profile;
pub(crate) mod scrcpy;

use crate::android::{self, Res};
use std::path::{Path, PathBuf};

pub use overlay::start;
pub use scrcpy::keep_on_screen;

fn dir() -> PathBuf {
    android::dirs_home().join(".local/share/playnix/controles")
}

fn path(pkg: &str) -> PathBuf {
    dir().join(format!("{pkg}.json"))
}

pub fn load(pkg: &str) -> Option<profile::Profile> {
    let text = std::fs::read_to_string(path(pkg)).ok()?;
    match profile::parse(&text) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("playnix: perfil de {pkg}: {e}");
            None
        }
    }
}

/// Lo que guarda el editor (mismo formato que se importa).
pub(crate) fn save(pkg: &str, p: &profile::Profile) -> Res<()> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let tmp = path(pkg).with_extension("json.new");
    std::fs::write(&tmp, profile::to_json(p)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path(pkg)).map_err(|e| e.to_string())
}

/// Copia un perfil (formato de waydroid-helper) como el de `pkg`. Devuelve cuántos
/// controles tiene.
pub fn import(pkg: &str, file: &Path) -> Res<usize> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("no se pudo leer {}: {e}", file.display()))?;
    let n = profile::parse(&text)?.widgets.len();
    if n == 0 {
        return Err("el perfil no tiene controles que Playnix entienda".into());
    }
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    std::fs::write(path(pkg), text).map_err(|e| e.to_string())?;
    Ok(n)
}

/// Activa la capa (gestos del mouse) en una app que no es juego: un perfil vacío,
/// que después se puede llenar con el editor. No toca un perfil que ya exista.
pub fn enable_gestures(pkg: &str) -> Res<()> {
    if path(pkg).exists() {
        return Ok(());
    }
    save(pkg, &profile::Profile::empty())
}

pub fn remove(pkg: &str) -> bool {
    std::fs::remove_file(path(pkg)).is_ok()
}

/// ¿Es un juego? Lo dice el manifiesto de su APK, que Waydroid guarda en la carpeta
/// de datos del usuario (se lee sin pasar por Android).
pub fn is_game(pkg: &str) -> bool {
    let apps = android::dirs_home().join(".local/share/waydroid/data/app");
    let prefix = format!("{pkg}-");
    let base = std::fs::read_dir(&apps).into_iter().flatten().flatten().find_map(|dir| {
        // /data/app/~~xxx==/<paquete>-yyy==/base.apk
        std::fs::read_dir(dir.path()).into_iter().flatten().flatten().find_map(|d| {
            d.file_name().to_string_lossy().starts_with(&prefix).then(|| d.path().join("base.apk"))
        })
    });
    base.and_then(|p| crate::apk::read(&p)).is_some_and(|i| i.game)
}
