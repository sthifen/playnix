//! Perfiles de controles. Mismo formato JSON que el mapeador de teclas de
//! waydroid-helper (versión 1.2), así se pueden importar los que ya existen.

use gtk::gdk;
use gtk::glib::translate::FromGlib;
use serde_json::Value;

/// Una tecla del teclado o un botón del mouse (numeración de GTK: 1 izq., 3 der.,
/// 8 atrás, 9 adelante).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Kb(gdk::Key),
    Mouse(u32),
}

#[derive(Clone, Debug)]
pub enum Kind {
    /// Toca el centro mientras la tecla está presionada.
    Tap(Vec<Key>),
    /// Joystick: arrastra desde el centro hacia el borde según las teclas.
    Pad { up: Vec<Key>, down: Vec<Key>, left: Vec<Key>, right: Vec<Key>, smooth: bool },
    /// Cámara con el mouse; su tecla entra y sale del modo apuntar.
    Aim { keys: Vec<Key>, sensitivity: f64 },
    /// Disparo: solo mientras se apunta.
    Fire(Vec<Key>),
}

/// Posición y tamaño en las coordenadas del perfil (ver `Profile::width/height`).
#[derive(Clone, Debug)]
pub struct Widget {
    pub kind: Kind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Widget {
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub width: f64,
    pub height: f64,
    pub widgets: Vec<Widget>,
}

impl Profile {
    /// Sin controles: solo los gestos del mouse (se crean con el editor).
    pub fn empty() -> Self {
        Profile { width: 1920.0, height: 1080.0, widgets: Vec::new() }
    }
}

/// Nombres especiales del formato (el resto: una letra, dígito o símbolo).
const NAMED: &[(&str, u32)] = &[
    ("Ctrl_L", 0xFFE3), ("Ctrl_R", 0xFFE4), ("Alt_L", 0xFFE9), ("Alt_R", 0xFFEA),
    ("Shift_L", 0xFFE1), ("Shift_R", 0xFFE2), ("Super_L", 0xFFEB), ("Super_R", 0xFFEC),
    ("Enter", 0xFF0D), ("Escape", 0xFF1B), ("Backspace", 0xFF08), ("Delete", 0xFFFF),
    ("Tab", 0xFF09), ("Home", 0xFF50), ("End", 0xFF57), ("PageUp", 0xFF55),
    ("PageDown", 0xFF56), ("Insert", 0xFF63), ("Left", 0xFF51), ("Right", 0xFF53),
    ("Up", 0xFF52), ("Down", 0xFF54), ("Space", 0x20),
];

pub fn key_from_name(name: &str) -> Option<Key> {
    let mouse = match name {
        "Mouse_Left" => Some(1),
        "Mouse_Middle" => Some(2),
        "Mouse_Right" => Some(3),
        "Mouse_Back" => Some(8),
        "Mouse_Forward" => Some(9),
        _ => None,
    };
    if let Some(b) = mouse {
        return Some(Key::Mouse(b));
    }
    let val = if let Some(&(_, v)) = NAMED.iter().find(|(n, _)| *n == name) {
        v
    } else if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=12).contains(n)) {
        0xFFBD + n
    } else {
        let mut chars = name.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => gdk::unicode_to_keyval(c.to_lowercase().next().unwrap_or(c) as u32),
            _ => return gdk::Key::from_name(name).map(|k| Key::Kb(k.to_lower())),
        }
    };
    Some(Key::Kb(unsafe { gdk::Key::from_glib(val) }.to_lower()))
}

/// Nombre en el formato del perfil (inverso de `key_from_name`).
pub fn key_name(key: Key) -> String {
    match key {
        Key::Mouse(b) => match b {
            1 => "Mouse_Left",
            2 => "Mouse_Middle",
            3 => "Mouse_Right",
            8 => "Mouse_Back",
            9 => "Mouse_Forward",
            _ => "Mouse_Left",
        }
        .into(),
        Key::Kb(k) => {
            use gtk::glib::translate::IntoGlib;
            let v = k.into_glib();
            if let Some(&(n, _)) = NAMED.iter().find(|(_, val)| *val == v) {
                return n.into();
            }
            if (0xFFBE..=0xFFC9).contains(&v) {
                return format!("F{}", v - 0xFFBD);
            }
            match k.to_unicode() {
                Some(c) if !c.is_control() => c.to_uppercase().collect(),
                _ => k.name().map(|n| n.to_string()).unwrap_or_default(),
            }
        }
    }
}

/// Texto corto para mostrar una tecla sobre el juego.
pub fn key_label(key: Key) -> String {
    match key_name(key).as_str() {
        "Mouse_Left" => "Clic".into(),
        "Mouse_Right" => "Clic der.".into(),
        "Mouse_Middle" => "Rueda".into(),
        "Mouse_Back" => "Atrás".into(),
        "Mouse_Forward" => "Adelante".into(),
        "Shift_L" | "Shift_R" => "Shift".into(),
        "Ctrl_L" | "Ctrl_R" => "Ctrl".into(),
        "Alt_L" | "Alt_R" => "Alt".into(),
        "Space" => "Espacio".into(),
        "PageUp" => "RePág".into(),
        "PageDown" => "AvPág".into(),
        "Enter" => "Enter".into(),
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        "Escape" => "Esc".into(),
        other => other.into(),
    }
}

/// Perfil → JSON del formato de waydroid-helper, en las coordenadas actuales.
pub fn to_json(p: &Profile) -> String {
    use serde_json::json;
    let names = |k: &Vec<Key>| k.iter().map(|k| key_name(*k)).collect::<Vec<_>>();
    let combos = |k: &Vec<Key>| k.iter().map(|k| vec![key_name(*k)]).collect::<Vec<_>>();
    let widgets: Vec<Value> = p
        .widgets
        .iter()
        .map(|w| {
            let mut v = json!({ "x": w.x, "y": w.y, "width": w.w, "height": w.h });
            let o = v.as_object_mut().unwrap();
            match &w.kind {
                Kind::Tap(k) => {
                    o.insert("type".into(), json!("singleclick"));
                    o.insert("text".into(), json!(k.first().map(|k| key_name(*k)).unwrap_or_default()));
                    o.insert("default_keys".into(), json!(combos(k)));
                }
                Kind::Pad { up, down, left, right, smooth } => {
                    o.insert("type".into(), json!("directionalpad"));
                    o.insert(
                        "direction_keys".into(),
                        json!({ "up": names(up), "down": names(down), "left": names(left), "right": names(right) }),
                    );
                    let mode = if *smooth { "smooth" } else { "instant" };
                    o.insert("config".into(), json!({ "movement_mode": { "key": "movement_mode", "value": mode } }));
                }
                Kind::Aim { keys, sensitivity } => {
                    o.insert("type".into(), json!("aim"));
                    o.insert("text".into(), json!(keys.first().map(|k| key_name(*k)).unwrap_or_default()));
                    o.insert("default_keys".into(), json!(combos(keys)));
                    o.insert("config".into(), json!({ "sensitivity": { "key": "sensitivity", "value": sensitivity } }));
                }
                Kind::Fire(k) => {
                    let right = k.first() == Some(&Key::Mouse(3));
                    o.insert("type".into(), json!("fire"));
                    o.insert("text".into(), json!(if right { "Mouse_Right" } else { "Mouse_Left" }));
                    o.insert("default_keys".into(), json!(combos(k)));
                    let b = if right { "right" } else { "left" };
                    o.insert("config".into(), json!({ "mouse_button": { "key": "mouse_button", "value": b } }));
                }
            }
            v
        })
        .collect();
    let out = json!({
        "version": "1.2",
        "screen_resolution": { "width": p.width, "height": p.height },
        "widgets": widgets,
    });
    serde_json::to_string_pretty(&out).unwrap_or_default()
}

/// `[["W"], ["Shift_L", "A"]]` → solo combinaciones de una tecla (las de varias
/// no se usan en los perfiles que circulan).
fn keys(v: Option<&Value>) -> Vec<Key> {
    let mut out = Vec::new();
    for combo in v.and_then(Value::as_array).into_iter().flatten() {
        let names: Vec<&str> = match combo {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            Value::String(s) => vec![s.as_str()],
            _ => continue,
        };
        if let [one] = names[..] {
            out.extend(key_from_name(one));
        }
    }
    out
}

fn config<'a>(w: &'a Value, key: &str) -> Option<&'a Value> {
    w.get("config")?.get(key)?.get("value")
}

pub fn parse(text: &str) -> Result<Profile, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("no es un perfil válido: {e}"))?;
    let res = v.get("screen_resolution");
    let dim = |k: &str| res.and_then(|r| r.get(k)).and_then(Value::as_f64).filter(|n| *n > 0.0);
    let (width, height) = (dim("width").unwrap_or(1920.0), dim("height").unwrap_or(1080.0));
    let list = v.get("widgets").and_then(Value::as_array).ok_or("al perfil le faltan los controles")?;

    let mut widgets = Vec::new();
    for w in list {
        let num = |k: &str| w.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let kind = match w.get("type").and_then(Value::as_str).unwrap_or_default() {
            "singleclick" => Kind::Tap(keys(w.get("default_keys"))),
            "directionalpad" => {
                let dir = |d: &str| {
                    let names = w.get("direction_keys").and_then(|k| k.get(d)).and_then(Value::as_array);
                    names.into_iter().flatten().filter_map(Value::as_str).filter_map(key_from_name).collect()
                };
                let smooth = config(w, "movement_mode").and_then(Value::as_str) != Some("instant");
                Kind::Pad { up: dir("up"), down: dir("down"), left: dir("left"), right: dir("right"), smooth }
            }
            "aim" => Kind::Aim {
                keys: keys(w.get("default_keys")),
                sensitivity: config(w, "sensitivity").and_then(Value::as_f64).unwrap_or(20.0),
            },
            "fire" => {
                let button = if config(w, "mouse_button").and_then(Value::as_str) == Some("right") { 3 } else { 1 };
                Kind::Fire(vec![Key::Mouse(button)])
            }
            other => {
                eprintln!("playnix: control '{other}' todavía no se admite, se omite");
                continue;
            }
        };
        widgets.push(Widget { kind, x: num("x"), y: num("y"), w: num("width"), h: num("height") });
    }
    Ok(Profile { width, height, widgets })
}
