//! Capa transparente encima del juego. El juego ocupa toda la pantalla de Android
//! y su ventana va maximizada (libpnxh.so, data/hwc-fix), así que una ventana
//! maximizada encima coincide pixel por pixel: lo que se toca aquí se manda a
//! Android en el mismo punto.
//!
//! Jugar: las teclas del perfil se convierten en toques; el clic izquierdo
//! arrastra como un dedo; Shift + clic derecho/izquierdo hace el gesto de
//! agrandar/encoger; el resto de teclas pasa a Android. F1 abre el editor y F2
//! muestra u oculta las teclas; el botón ⋮ del borde derecho tiene todo eso.

use super::pointer::PointerLock;
use super::profile::{self, Key, Kind, Profile, Widget};
use super::scrcpy::{self, Link};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Pasos y pausa del arrastre suave del joystick (como waydroid-helper).
const PAD_STEPS: u32 = 6;
const PAD_STEP_MS: u64 = 20;
/// Gesto de pellizco: pasos, pausa entre pasos y pausa antes de repetir.
const PINCH_STEPS: u32 = 12;
const PINCH_STEP_MS: u64 = 16;
const PINCH_REPEAT_MS: u64 = 60;
/// Cuánto se ven las teclas al empezar a jugar.
const HINTS_SECS: u32 = 4;
const ACCENT: (f64, f64, f64) = (0.21, 0.52, 0.89);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Play,
    Edit,
}

struct Pad {
    widget: usize,
    /// arriba, izquierda, abajo, derecha
    held: [bool; 4],
    finger: Option<u64>,
    /// en coordenadas del perfil
    pos: (f64, f64),
    generation: u32,
}

struct Pinch {
    fingers: (u64, u64),
    button: u32,
    held: bool,
    center: (f64, f64),
    zoom_in: bool,
}

struct Aim {
    widget: usize,
    finger: Option<u64>,
    /// en coordenadas de la capa
    pos: (f64, f64),
}

#[derive(Default)]
struct Editor {
    selected: Option<usize>,
    /// desplazamiento del clic dentro del control que se arrastra (perfil)
    grab: Option<(f64, f64)>,
    /// dirección del joystick elegida para asignarle tecla
    pad_dir: Option<usize>,
}

#[derive(Clone, Copy)]
enum Tool {
    Tap,
    Pad,
    Aim,
    Fire,
    Delete,
    Done,
}

pub(super) struct State {
    link: Arc<Link>,
    pkg: String,
    profile: Profile,
    area: gtk::DrawingArea,
    win: glib::WeakRef<gtk::ApplicationWindow>,
    lock: Option<Rc<PointerLock>>,
    /// tamaño de la capa en píxeles lógicos
    view: (f64, f64),
    mode: Mode,
    editor: Editor,
    taps: HashMap<usize, u64>,
    fires: HashMap<usize, u64>,
    pads: Vec<Pad>,
    free: Vec<u64>,
    held: HashSet<Key>,
    /// dedo del clic izquierdo que pasa tal cual (arrastrar en menús, etc.)
    drag: Option<u64>,
    pinch: Option<Pinch>,
    aim: Option<Aim>,
    pointer: (f64, f64),
    hints: bool,
    toast: Option<String>,
    toast_gen: u32,
}

type St = Rc<RefCell<State>>;

impl State {
    fn finger(&mut self) -> Option<u64> {
        self.free.pop()
    }

    fn release_finger(&mut self, id: u64) {
        if !self.free.contains(&id) {
            self.free.push(id);
        }
    }

    /// perfil → capa
    fn to_view(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (x * self.view.0 / self.profile.width, y * self.view.1 / self.profile.height)
    }

    /// capa → perfil
    fn to_profile(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (x * self.profile.width / self.view.0, y * self.profile.height / self.view.1)
    }

    /// capa → Android
    fn to_android(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let (aw, ah) = self.link.size();
        (x * aw as f64 / self.view.0, y * ah as f64 / self.view.1)
    }

    fn touch_profile(&self, action: u8, id: u64, p: (f64, f64)) {
        self.touch_view(action, id, self.to_view(p));
    }

    fn touch_view(&self, action: u8, id: u64, p: (f64, f64)) {
        let (x, y) = self.to_android(p);
        self.link.touch(action, id, x, y);
    }

    /// Rectángulo del control en la capa: (x, y, ancho, alto).
    fn rect(&self, w: &Widget) -> (f64, f64, f64, f64) {
        let (x, y) = self.to_view((w.x, w.y));
        let (x2, y2) = self.to_view((w.x + w.w, w.y + w.h));
        (x, y, x2 - x, y2 - y)
    }

    fn rebuild_pads(&mut self) {
        self.pads = self
            .profile
            .widgets
            .iter()
            .enumerate()
            .filter(|(_, w)| matches!(w.kind, Kind::Pad { .. }))
            .map(|(i, w)| Pad { widget: i, held: [false; 4], finger: None, pos: w.center(), generation: 0 })
            .collect();
    }

    fn pad_target(&self, pad: &Pad) -> (f64, f64) {
        let w = &self.profile.widgets[pad.widget];
        let (cx, cy) = w.center();
        let [up, left, down, right] = pad.held;
        let dx = right as i32 - left as i32;
        let dy = down as i32 - up as i32;
        let r = w.w.min(w.h) / 2.0;
        let k = if dx != 0 && dy != 0 { 0.7071 } else { 1.0 };
        (cx + dx as f64 * r * k, cy + dy as f64 * r * k)
    }

    /// Levanta todos los dedos (al cambiar de modo, al cerrar, al perder el foco).
    fn lift_all(&mut self) {
        let mut ups: Vec<(u64, (f64, f64))> = Vec::new();
        for (i, id) in self.taps.drain().chain(self.fires.drain()).collect::<Vec<_>>() {
            ups.push((id, self.to_view(self.profile.widgets[i].center())));
        }
        for p in 0..self.pads.len() {
            self.pads[p].held = [false; 4];
            self.pads[p].generation += 1;
            if let Some(id) = self.pads[p].finger.take() {
                ups.push((id, self.to_view(self.pads[p].pos)));
            }
        }
        if let Some(id) = self.drag.take() {
            ups.push((id, self.pointer));
        }
        if let Some(p) = self.pinch.take() {
            ups.push((p.fingers.0, p.center));
            ups.push((p.fingers.1, p.center));
        }
        if let Some(a) = self.aim.take() {
            if let Some(id) = a.finger {
                ups.push((id, a.pos));
            }
        }
        if let Some(l) = &self.lock {
            l.unlock();
        }
        for (id, pos) in ups {
            self.touch_view(scrcpy::UP, id, pos);
            self.release_finger(id);
        }
        self.held.clear();
    }
}

fn toast(st: &St, text: &str, secs: u32) {
    let generation = {
        let mut s = st.borrow_mut();
        s.toast = Some(text.to_string());
        s.toast_gen += 1;
        s.area.queue_draw();
        s.toast_gen
    };
    let weak = Rc::downgrade(st);
    glib::timeout_add_seconds_local_once(secs, move || {
        if let Some(st) = weak.upgrade() {
            let mut s = st.borrow_mut();
            if s.toast_gen == generation {
                s.toast = None;
                if s.mode == Mode::Play {
                    s.hints = false;
                }
                s.area.queue_draw();
            }
        }
    });
}

// --- Jugar -------------------------------------------------------------------

fn pad_dir(kind: &Kind, key: Key) -> Option<usize> {
    let Kind::Pad { up, down, left, right, .. } = kind else { return None };
    [up, left, down, right].iter().position(|keys| keys.contains(&key))
}

/// Mueve el joystick `p` a donde indican sus teclas; suave al empezar a moverse.
fn pad_update(st: &St, p: usize, smooth: bool) {
    let mut s = st.borrow_mut();
    let target = s.pad_target(&s.pads[p]);
    s.pads[p].generation += 1;
    let Some(id) = s.pads[p].finger else { return };
    if !smooth {
        s.pads[p].pos = target;
        s.touch_profile(scrcpy::MOVE, id, target);
        return;
    }
    let (generation, start) = (s.pads[p].generation, s.pads[p].pos);
    drop(s);
    let weak = Rc::downgrade(st);
    let mut step = 0;
    glib::timeout_add_local(Duration::from_millis(PAD_STEP_MS), move || {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        let mut s = st.borrow_mut();
        let Some(pad) = s.pads.get(p) else { return glib::ControlFlow::Break };
        if pad.generation != generation || pad.finger != Some(id) {
            return glib::ControlFlow::Break;
        }
        step += 1;
        let t = step as f64 / PAD_STEPS as f64;
        let pos = (start.0 + (target.0 - start.0) * t, start.1 + (target.1 - start.1) * t);
        s.pads[p].pos = pos;
        s.touch_profile(scrcpy::MOVE, id, pos);
        if step >= PAD_STEPS { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

fn aim_toggle(st: &St, i: usize) {
    if st.borrow().aim.is_some() {
        aim_exit(st);
        return;
    }
    {
        let mut s = st.borrow_mut();
        let center = s.to_view(s.profile.widgets[i].center());
        s.aim = Some(Aim { widget: i, finger: None, pos: center });
        if s.lock.is_none() {
            s.lock = s.win.upgrade().and_then(|w| PointerLock::new(w.upcast_ref()));
        }
    }
    let lock = st.borrow().lock.clone();
    match lock {
        Some(lock) => {
            let weak = Rc::downgrade(st);
            lock.lock(move |dx, dy| {
                if let Some(st) = weak.upgrade() {
                    aim_move(&st, dx, dy);
                }
            });
        }
        // sin bloqueo, se usa el movimiento normal del cursor (ver `motion`)
        None => toast(st, "Tu escritorio no permite fijar el mouse: apunta moviéndolo", 3),
    }
    if st.borrow().toast.is_none() {
        toast(st, "Apuntando con el mouse · Esc para soltarlo", 2);
    }
}

fn aim_exit(st: &St) {
    let mut s = st.borrow_mut();
    if let Some(l) = &s.lock {
        l.unlock();
    }
    if let Some(a) = s.aim.take() {
        if let Some(id) = a.finger {
            s.touch_view(scrcpy::UP, id, a.pos);
            s.release_finger(id);
        }
    }
    for (i, id) in s.fires.drain().collect::<Vec<_>>() {
        let c = s.profile.widgets[i].center();
        s.touch_profile(scrcpy::UP, id, c);
        s.release_finger(id);
    }
}

/// Movimiento del mouse mientras se apunta: arrastra un dedo dentro del área de
/// apuntar; al salirse, lo levanta y vuelve a apoyarlo en el centro.
fn aim_move(st: &St, dx: f64, dy: f64) {
    let mut s = st.borrow_mut();
    let Some(a) = s.aim.as_ref() else { return };
    let w = s.profile.widgets[a.widget].clone();
    let Kind::Aim { sensitivity, .. } = w.kind else { return };
    let (rx, ry, rw, rh) = s.rect(&w);
    let center = (rx + rw / 2.0, ry + rh / 2.0);
    let k = sensitivity / 50.0;
    let id = match s.aim.as_ref().and_then(|a| a.finger) {
        Some(id) => id,
        None => {
            let Some(id) = s.finger() else { return };
            s.touch_view(scrcpy::DOWN, id, center);
            let a = s.aim.as_mut().unwrap();
            a.finger = Some(id);
            a.pos = center;
            id
        }
    };
    let pos = s.aim.as_ref().unwrap().pos;
    let next = (pos.0 + dx * k, pos.1 + dy * k);
    if next.0 < rx || next.0 > rx + rw || next.1 < ry || next.1 > ry + rh {
        s.touch_view(scrcpy::UP, id, pos);
        s.touch_view(scrcpy::DOWN, id, center);
        s.aim.as_mut().unwrap().pos = center;
    } else {
        s.touch_view(scrcpy::MOVE, id, next);
        s.aim.as_mut().unwrap().pos = next;
    }
}

/// true si la tecla pertenece al perfil (entonces no se manda a Android).
fn press(st: &St, key: Key) -> bool {
    let mut handled = false;
    let n = st.borrow().profile.widgets.len();
    for i in 0..n {
        let kind = st.borrow().profile.widgets[i].kind.clone();
        match &kind {
            Kind::Tap(keys) if keys.contains(&key) => {
                handled = true;
                let mut s = st.borrow_mut();
                if !s.taps.contains_key(&i) {
                    if let Some(id) = s.finger() {
                        s.taps.insert(i, id);
                        let c = s.profile.widgets[i].center();
                        s.touch_profile(scrcpy::DOWN, id, c);
                    }
                }
            }
            Kind::Pad { smooth, .. } => {
                let Some(dir) = pad_dir(&kind, key) else { continue };
                handled = true;
                let Some(p) = st.borrow().pads.iter().position(|pad| pad.widget == i) else { continue };
                let starting = {
                    let mut s = st.borrow_mut();
                    s.pads[p].held[dir] = true;
                    if s.pads[p].finger.is_none() {
                        let Some(id) = s.finger() else { continue };
                        let c = s.profile.widgets[i].center();
                        s.pads[p].finger = Some(id);
                        s.pads[p].pos = c;
                        s.touch_profile(scrcpy::DOWN, id, c);
                        true
                    } else {
                        false
                    }
                };
                pad_update(st, p, starting && *smooth);
            }
            Kind::Aim { keys, .. } if keys.contains(&key) => {
                handled = true;
                aim_toggle(st, i);
            }
            // el disparo solo existe mientras se apunta; si no, el clic es un clic
            Kind::Fire(keys) if keys.contains(&key) && st.borrow().aim.is_some() => {
                handled = true;
                let mut s = st.borrow_mut();
                if !s.fires.contains_key(&i) {
                    if let Some(id) = s.finger() {
                        s.fires.insert(i, id);
                        let c = s.profile.widgets[i].center();
                        s.touch_profile(scrcpy::DOWN, id, c);
                    }
                }
            }
            _ => {}
        }
    }
    handled
}

fn release(st: &St, key: Key) -> bool {
    let mut handled = false;
    let n = st.borrow().profile.widgets.len();
    for i in 0..n {
        let kind = st.borrow().profile.widgets[i].kind.clone();
        match &kind {
            Kind::Tap(keys) if keys.contains(&key) => {
                handled = true;
                let mut s = st.borrow_mut();
                if let Some(id) = s.taps.remove(&i) {
                    let c = s.profile.widgets[i].center();
                    s.touch_profile(scrcpy::UP, id, c);
                    s.release_finger(id);
                }
            }
            Kind::Pad { .. } => {
                let Some(dir) = pad_dir(&kind, key) else { continue };
                handled = true;
                let Some(p) = st.borrow().pads.iter().position(|pad| pad.widget == i) else { continue };
                let any = {
                    let mut s = st.borrow_mut();
                    s.pads[p].held[dir] = false;
                    s.pads[p].held.iter().any(|h| *h)
                };
                if any {
                    pad_update(st, p, false);
                } else {
                    let mut s = st.borrow_mut();
                    s.pads[p].generation += 1;
                    if let Some(id) = s.pads[p].finger.take() {
                        let pos = s.pads[p].pos;
                        s.touch_profile(scrcpy::UP, id, pos);
                        s.release_finger(id);
                    }
                    s.pads[p].pos = s.profile.widgets[i].center();
                }
            }
            Kind::Aim { keys, .. } if keys.contains(&key) => handled = true,
            Kind::Fire(keys) if keys.contains(&key) => {
                let mut s = st.borrow_mut();
                if let Some(id) = s.fires.remove(&i) {
                    handled = true;
                    let c = s.profile.widgets[i].center();
                    s.touch_profile(scrcpy::UP, id, c);
                    s.release_finger(id);
                }
            }
            _ => {}
        }
    }
    handled
}

/// Dos dedos que se separan (agrandar) o se juntan (encoger) alrededor de `center`.
/// Mientras el botón siga presionado, el gesto se repite.
fn pinch_start(st: &St, center: (f64, f64), zoom_in: bool, button: u32) {
    let mut s = st.borrow_mut();
    if s.pinch.is_some() {
        return;
    }
    let Some(a) = s.finger() else { return };
    let Some(b) = s.finger() else {
        s.release_finger(a);
        return;
    };
    s.pinch = Some(Pinch { fingers: (a, b), button, held: true, center, zoom_in });
    drop(s);
    pinch_run(st);
}

fn pinch_run(st: &St) {
    let (center, near, far, zoom_in, (a, b)) = {
        let mut s = st.borrow_mut();
        let Some(p) = s.pinch.as_ref() else { return };
        let (center, zoom_in, fingers) = (p.center, p.zoom_in, p.fingers);
        let near = s.view.1 * 0.04;
        let far = (s.view.1 * 0.28).min(s.view.0 * 0.45);
        // que los dos dedos quepan en la pantalla
        let cx = center.0.clamp(far + 2.0, (s.view.0 - far - 2.0).max(far + 2.0));
        s.pinch.as_mut().unwrap().center = (cx, center.1);
        ((cx, center.1), near, far, zoom_in, fingers)
    };
    let (from, to) = if zoom_in { (near, far) } else { (far, near) };
    {
        let s = st.borrow();
        s.touch_view(scrcpy::DOWN, a, (center.0 - from, center.1));
        s.touch_view(scrcpy::DOWN, b, (center.0 + from, center.1));
    }
    let weak = Rc::downgrade(st);
    let mut step = 0;
    glib::timeout_add_local(Duration::from_millis(PINCH_STEP_MS), move || {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        step += 1;
        let d = from + (to - from) * (step as f64 / PINCH_STEPS as f64);
        {
            let s = st.borrow();
            if s.pinch.as_ref().map(|p| p.fingers) != Some((a, b)) {
                return glib::ControlFlow::Break;
            }
            s.touch_view(scrcpy::MOVE, a, (center.0 - d, center.1));
            s.touch_view(scrcpy::MOVE, b, (center.0 + d, center.1));
        }
        if step < PINCH_STEPS {
            return glib::ControlFlow::Continue;
        }
        let again = {
            let mut s = st.borrow_mut();
            s.touch_view(scrcpy::UP, a, (center.0 - to, center.1));
            s.touch_view(scrcpy::UP, b, (center.0 + to, center.1));
            let held = s.pinch.as_ref().is_some_and(|p| p.held);
            if !held {
                s.pinch = None;
                s.release_finger(a);
                s.release_finger(b);
            }
            held
        };
        if again {
            let weak = Rc::downgrade(&st);
            glib::timeout_add_local_once(Duration::from_millis(PINCH_REPEAT_MS), move || {
                if let Some(st) = weak.upgrade() {
                    pinch_run(&st);
                }
            });
        }
        glib::ControlFlow::Break
    });
}

/// Teclas sueltas → KEYCODE_* de Android (para escribir y navegar menús).
fn android_keycode(k: gdk::Key) -> Option<u32> {
    let c = k.to_lower().to_unicode();
    if let Some(c) = c.filter(|c| c.is_ascii_lowercase()) {
        return Some(29 + (c as u32 - 'a' as u32));
    }
    if let Some(d) = c.and_then(|c| c.to_digit(10)) {
        return Some(7 + d);
    }
    use gdk::Key as K;
    Some(match k {
        K::space => 62,
        K::Return | K::KP_Enter => 66,
        K::BackSpace => 67,
        K::Tab => 61,
        K::Escape => 4, // atrás
        K::Up => 19,
        K::Down => 20,
        K::Left => 21,
        K::Right => 22,
        K::Delete => 112,
        K::Home => 122,
        K::End => 123,
        K::Page_Up => 92,
        K::Page_Down => 93,
        K::Shift_L => 59,
        K::Shift_R => 60,
        K::Control_L => 113,
        K::Control_R => 114,
        K::Alt_L => 57,
        K::comma => 55,
        K::period => 56,
        K::minus => 69,
        K::equal => 70,
        K::slash => 76,
        K::semicolon => 74,
        K::apostrophe => 75,
        _ => return None,
    })
}

fn meta(state: gdk::ModifierType) -> u32 {
    let mut m = 0;
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        m |= 0x41;
    }
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        m |= 0x3000;
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        m |= 0x12;
    }
    m
}

// --- Editor ------------------------------------------------------------------

fn tools() -> [(Tool, &'static str); 6] {
    [
        (Tool::Tap, "+ Botón"),
        (Tool::Pad, "+ Joystick"),
        (Tool::Aim, "+ Apuntar"),
        (Tool::Fire, "+ Disparo"),
        (Tool::Delete, "Borrar"),
        (Tool::Done, "Guardar"),
    ]
}

/// Botones de la barra del editor: (herramienta, texto, rectángulo en la capa).
fn toolbar(view: (f64, f64)) -> Vec<(Tool, &'static str, (f64, f64, f64, f64))> {
    let (bw, bh, gap) = (118.0, 36.0, 8.0);
    let n = tools().len() as f64;
    let x0 = (view.0 - (n * bw + (n - 1.0) * gap)) / 2.0;
    tools()
        .into_iter()
        .enumerate()
        .map(|(i, (t, label))| (t, label, (x0 + i as f64 * (bw + gap), 16.0, bw, bh)))
        .collect()
}

fn inside((x, y): (f64, f64), (rx, ry, rw, rh): (f64, f64, f64, f64)) -> bool {
    x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
}

/// Botón ⋮ en el borde derecho, a media altura (las esquinas suelen tener botones
/// del juego): abre el menú de los controles.
fn menu_rect(view: (f64, f64)) -> (f64, f64, f64, f64) {
    let (w, h) = (22.0, 54.0);
    (view.0 - w, (view.1 - h) / 2.0, w, h)
}

fn open_menu(st: &St) {
    let (area, view, edit) = {
        let s = st.borrow();
        (s.area.clone(), s.view, s.mode == Mode::Edit)
    };
    let menu = gio::Menu::new();
    menu.append(Some(if edit { "Guardar controles" } else { "Editar controles (F1)" }), Some("ctl.edit"));
    if !edit {
        menu.append(Some("Mostrar u ocultar teclas (F2)"), Some("ctl.hints"));
    }
    menu.append(Some("Cómo se usa"), Some("ctl.help"));
    menu.append(Some("Quitar controles"), Some("ctl.close"));
    let pop = gtk::PopoverMenu::from_model(Some(&menu));
    pop.set_parent(&area);
    let (x, y, w, h) = menu_rect(view);
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, w as i32, h as i32)));
    pop.set_position(gtk::PositionType::Left);
    pop.set_has_arrow(false);
    pop.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
}

fn show_help(st: &St) {
    let Some(win) = st.borrow().win.upgrade() else { return };
    let body = "Jugar\n\
        • Clic izquierdo y arrastrar: deslizar un dedo.\n\
        • Shift + clic derecho: agrandar (dos dedos que se separan); mantenlo para seguir.\n\
        • Shift + clic izquierdo: encoger.\n\
        • Las teclas de tus controles tocan la pantalla por ti.\n\
        • Esc suelta el mouse si quedó fijado para apuntar.\n\n\
        Crear o cambiar controles (F1 o ⋮ → Editar)\n\
        • Arriba aparece una barra: elige Botón, Joystick, Apuntar o Disparo.\n\
        • Arrastra el control a su lugar sobre el juego; la rueda del mouse cambia su tamaño.\n\
        • Con el control elegido, toca la tecla que quieres usar.\n\
        • Guardar (o F1) termina. Cada juego guarda sus propios controles.\n\n\
        F2 muestra u oculta las teclas sobre el juego.";
    let dialog = adw::AlertDialog::new(Some("Controles del juego"), Some(body));
    dialog.add_response("ok", "Entendido");
    dialog.present(Some(&win));
}

fn edit_enter(st: &St) {
    {
        let mut s = st.borrow_mut();
        s.lift_all();
        s.mode = Mode::Edit;
        s.hints = true;
        s.editor = Editor::default();
        s.area.queue_draw();
    }
    toast(st, "Editor: arrastra para mover · rueda para el tamaño · elige un control y toca una tecla · F1 guarda", 6);
}

fn edit_exit(st: &St) {
    let saved = {
        let mut s = st.borrow_mut();
        s.mode = Mode::Play;
        s.hints = true;
        s.editor = Editor::default();
        s.rebuild_pads();
        super::save(&s.pkg, &s.profile)
    };
    match saved {
        Ok(()) => toast(st, "Controles guardados · F2 muestra las teclas cuando quieras", HINTS_SECS),
        Err(e) => toast(st, &format!("No se pudieron guardar los controles: {e}"), 4),
    }
}

fn edit_add(st: &St, tool: Tool) {
    let mut s = st.borrow_mut();
    let (cx, cy) = s.to_profile((s.view.0 / 2.0, s.view.1 / 2.0));
    // tamaños pensados en píxeles de la capa
    let k = s.profile.width / s.view.0;
    let keys = |n: &str| profile::key_from_name(n).into_iter().collect::<Vec<_>>();
    let (kind, w, h) = match tool {
        Tool::Tap => (Kind::Tap(Vec::new()), 64.0 * k, 64.0 * k),
        Tool::Pad => (
            Kind::Pad { up: keys("W"), down: keys("S"), left: keys("A"), right: keys("D"), smooth: true },
            220.0 * k,
            220.0 * k,
        ),
        Tool::Aim => (Kind::Aim { keys: keys("Tab"), sensitivity: 20.0 }, 300.0 * k, 200.0 * k),
        Tool::Fire => (Kind::Fire(vec![Key::Mouse(1)]), 72.0 * k, 72.0 * k),
        Tool::Delete | Tool::Done => return,
    };
    s.profile.widgets.push(Widget { kind, x: cx - w / 2.0, y: cy - h / 2.0, w, h });
    s.editor.selected = Some(s.profile.widgets.len() - 1);
    s.editor.pad_dir = None;
    s.area.queue_draw();
}

fn edit_delete(st: &St) {
    let mut s = st.borrow_mut();
    if let Some(i) = s.editor.selected.take() {
        s.profile.widgets.remove(i);
        s.area.queue_draw();
    }
}

/// Qué dirección del joystick hay bajo el punto (perfil), si alguna.
fn pad_region(w: &Widget, (x, y): (f64, f64)) -> Option<usize> {
    let (cx, cy) = w.center();
    let (dx, dy) = (x - cx, y - cy);
    let r = w.w.min(w.h) / 2.0;
    if (dx * dx + dy * dy).sqrt() < r * 0.3 {
        return None;
    }
    // arriba, izquierda, abajo, derecha
    Some(if dy.abs() > dx.abs() { if dy < 0.0 { 0 } else { 2 } } else if dx < 0.0 { 1 } else { 3 })
}

fn edit_button(st: &St, button: u32, pressed: bool, pos: (f64, f64)) {
    if !pressed {
        st.borrow_mut().editor.grab = None;
        return;
    }
    if button == 1 {
        let tool = toolbar(st.borrow().view).into_iter().find(|(_, _, r)| inside(pos, *r)).map(|(t, _, _)| t);
        match tool {
            Some(Tool::Done) => return edit_exit(st),
            Some(Tool::Delete) => return edit_delete(st),
            Some(t) => return edit_add(st, t),
            None => {}
        }
    }
    let mut s = st.borrow_mut();
    let p = s.to_profile(pos);
    let hit = (0..s.profile.widgets.len()).rev().find(|&i| {
        let w = &s.profile.widgets[i];
        p.0 >= w.x && p.0 <= w.x + w.w && p.1 >= w.y && p.1 <= w.y + w.h
    });
    match (button, hit) {
        (1, Some(i)) => {
            let w = &s.profile.widgets[i];
            let dir = if matches!(w.kind, Kind::Pad { .. }) { pad_region(w, p) } else { None };
            let grab = (p.0 - w.x, p.1 - w.y);
            s.editor.grab = Some(grab);
            s.editor.selected = Some(i);
            s.editor.pad_dir = dir;
        }
        (1, None) => s.editor.selected = None,
        // botones del mouse sobre un botón: asignarlos (el izquierdo es para editar)
        (b, Some(i)) if b != 1 && s.editor.selected == Some(i) => {
            if let Kind::Tap(keys) = &mut s.profile.widgets[i].kind {
                *keys = vec![Key::Mouse(b)];
            }
        }
        _ => {}
    }
    s.area.queue_draw();
}

fn edit_motion(st: &St, pos: (f64, f64)) {
    let mut s = st.borrow_mut();
    let (Some(i), Some((gx, gy))) = (s.editor.selected, s.editor.grab) else { return };
    let p = s.to_profile(pos);
    let (pw, ph) = (s.profile.width, s.profile.height);
    let w = &mut s.profile.widgets[i];
    w.x = (p.0 - gx).clamp(0.0, (pw - w.w).max(0.0));
    w.y = (p.1 - gy).clamp(0.0, (ph - w.h).max(0.0));
    s.editor.pad_dir = None;
    s.area.queue_draw();
}

fn edit_scroll(st: &St, dy: f64) {
    let mut s = st.borrow_mut();
    let Some(i) = s.editor.selected else { return };
    let min = 24.0 * s.profile.width / s.view.0;
    let f = if dy < 0.0 { 1.1 } else { 1.0 / 1.1 };
    let w = &mut s.profile.widgets[i];
    let (cx, cy) = w.center();
    w.w = (w.w * f).max(min);
    w.h = (w.h * f).max(min);
    w.x = cx - w.w / 2.0;
    w.y = cy - w.h / 2.0;
    s.area.queue_draw();
}

fn edit_key(st: &St, keyval: gdk::Key) {
    if matches!(keyval, gdk::Key::Escape) {
        return edit_exit(st);
    }
    if matches!(keyval, gdk::Key::Delete | gdk::Key::BackSpace) {
        return edit_delete(st);
    }
    let key = Key::Kb(keyval.to_lower());
    let mut s = st.borrow_mut();
    let Some(i) = s.editor.selected else { return };
    let dir = s.editor.pad_dir;
    match &mut s.profile.widgets[i].kind {
        Kind::Tap(keys) => *keys = vec![key],
        Kind::Aim { keys, .. } => *keys = vec![key],
        Kind::Pad { up, left, down, right, .. } => {
            if let Some(d) = dir {
                *[up, left, down, right][d] = vec![key];
            }
        }
        Kind::Fire(_) => {}
    }
    s.area.queue_draw();
}

// --- Entrada (eventos de GTK o del guion de pruebas) --------------------------

fn on_key(st: &St, keyval: gdk::Key, pressed: bool, mods: gdk::ModifierType) {
    if keyval == gdk::Key::F1 {
        if pressed {
            if st.borrow().mode == Mode::Play { edit_enter(st) } else { edit_exit(st) }
        }
        return;
    }
    if st.borrow().mode == Mode::Edit {
        if pressed {
            edit_key(st, keyval);
        }
        return;
    }
    if keyval == gdk::Key::F2 {
        if pressed {
            let mut s = st.borrow_mut();
            s.hints = !s.hints;
            s.toast_gen += 1; // que no las vuelva a esconder un aviso anterior
            s.area.queue_draw();
        }
        return;
    }
    // Esc siempre suelta el mouse si está fijado para apuntar
    if keyval == gdk::Key::Escape && st.borrow().aim.is_some() {
        if pressed {
            aim_exit(st);
            st.borrow().area.queue_draw();
        }
        return;
    }
    let key = Key::Kb(keyval.to_lower());
    if pressed {
        // ignorar la repetición automática del teclado
        if !st.borrow_mut().held.insert(key) {
            return;
        }
        if !press(st, key) {
            if let Some(code) = android_keycode(keyval) {
                st.borrow().link.key(scrcpy::DOWN, code, meta(mods));
            } else if let Some(c) = keyval.to_unicode().filter(|c| !c.is_control()) {
                // sin tecla equivalente en Android (á, ñ, ¿…): se escribe como texto
                if !mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
                    st.borrow().link.text(&c.to_string());
                }
            }
        }
    } else {
        st.borrow_mut().held.remove(&key);
        if !release(st, key) {
            if let Some(code) = android_keycode(keyval) {
                st.borrow().link.key(scrcpy::UP, code, meta(mods));
            }
        }
    }
    st.borrow().area.queue_draw();
}

fn on_button(st: &St, button: u32, pressed: bool, pos: (f64, f64), mods: gdk::ModifierType) {
    st.borrow_mut().pointer = pos;
    if button == 1 && inside(pos, menu_rect(st.borrow().view)) && st.borrow().drag.is_none() {
        if pressed {
            open_menu(st);
        }
        return;
    }
    if st.borrow().mode == Mode::Edit {
        return edit_button(st, button, pressed, pos);
    }
    let aiming = st.borrow().aim.is_some();
    if pressed && !aiming && mods.contains(gdk::ModifierType::SHIFT_MASK) && (button == 1 || button == 3) {
        // Shift + clic derecho agranda, Shift + clic izquierdo encoge
        return pinch_start(st, pos, button == 3, button);
    }
    if !pressed {
        let mut s = st.borrow_mut();
        if let Some(p) = s.pinch.as_mut().filter(|p| p.button == button) {
            p.held = false;
            return;
        }
    }
    let key = Key::Mouse(button);
    if pressed {
        if !press(st, key) && button == 1 && !aiming {
            let mut s = st.borrow_mut();
            if s.drag.is_none() {
                if let Some(id) = s.finger() {
                    s.drag = Some(id);
                    s.touch_view(scrcpy::DOWN, id, pos);
                }
            }
        }
    } else if !release(st, key) && button == 1 {
        let mut s = st.borrow_mut();
        if let Some(id) = s.drag.take() {
            s.touch_view(scrcpy::UP, id, pos);
            s.release_finger(id);
        }
    }
    st.borrow().area.queue_draw();
}

fn on_motion(st: &St, pos: (f64, f64)) {
    if st.borrow().mode == Mode::Edit {
        st.borrow_mut().pointer = pos;
        return edit_motion(st, pos);
    }
    let (last, aiming, locked) = {
        let s = st.borrow();
        (s.pointer, s.aim.is_some(), s.lock.as_ref().is_some_and(|l| l.is_locked()))
    };
    st.borrow_mut().pointer = pos;
    if aiming && !locked {
        return aim_move(st, pos.0 - last.0, pos.1 - last.1);
    }
    let s = st.borrow();
    if let Some(id) = s.drag {
        s.touch_view(scrcpy::MOVE, id, pos);
    }
}

fn on_scroll(st: &St, dx: f64, dy: f64, mods: gdk::ModifierType) {
    if st.borrow().mode == Mode::Edit {
        return edit_scroll(st, dy);
    }
    // Ctrl + rueda: agrandar/encoger donde está el mouse (un gesto por paso)
    if mods.contains(gdk::ModifierType::CONTROL_MASK) && dy != 0.0 {
        let pos = st.borrow().pointer;
        pinch_start(st, pos, dy < 0.0, 0);
        if let Some(p) = st.borrow_mut().pinch.as_mut() {
            p.held = false;
        }
        return;
    }
    let s = st.borrow();
    let (x, y) = s.to_android(s.pointer);
    // en Android la rueda hacia abajo es negativa
    s.link.scroll(x, y, -dx, -dy);
}

fn on_event(st: &St, ev: &gdk::Event) -> glib::Propagation {
    use gdk::EventType as E;
    // el menú ⋮ y la ayuda tienen su propia superficie: sus eventos son para ellos
    let own = st.borrow().win.upgrade().and_then(|w| w.surface());
    if ev.surface().is_some() && ev.surface() != own {
        return glib::Propagation::Proceed;
    }
    match ev.event_type() {
        E::KeyPress | E::KeyRelease => {
            let Some(k) = ev.downcast_ref::<gdk::KeyEvent>() else { return glib::Propagation::Proceed };
            on_key(st, k.keyval(), ev.event_type() == E::KeyPress, ev.modifier_state());
        }
        E::ButtonPress | E::ButtonRelease => {
            let Some(b) = ev.downcast_ref::<gdk::ButtonEvent>() else { return glib::Propagation::Proceed };
            let pos = ev.position().unwrap_or(st.borrow().pointer);
            on_button(st, b.button(), ev.event_type() == E::ButtonPress, pos, ev.modifier_state());
        }
        E::MotionNotify => {
            if let Some(pos) = ev.position() {
                on_motion(st, pos);
            }
        }
        E::Scroll => {
            let Some(sev) = ev.downcast_ref::<gdk::ScrollEvent>() else { return glib::Propagation::Proceed };
            let (dx, dy) = match sev.direction() {
                gdk::ScrollDirection::Up => (0.0, -1.0),
                gdk::ScrollDirection::Down => (0.0, 1.0),
                gdk::ScrollDirection::Left => (-1.0, 0.0),
                gdk::ScrollDirection::Right => (1.0, 0.0),
                _ => sev.deltas(),
            };
            on_scroll(st, dx, dy, ev.modifier_state());
        }
        // al perder el foco (Alt+Tab) no deben quedar dedos apoyados
        E::FocusChange => st.borrow_mut().lift_all(),
        _ => return glib::Propagation::Proceed,
    }
    glib::Propagation::Stop
}

// --- Dibujo --------------------------------------------------------------------

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}

/// Texto con Pango (como GTK): busca en otras fuentes lo que falte (flechas, ñ…).
fn layout(cr: &gtk::cairo::Context, s: &str, size: f64) -> pangocairo::pango::Layout {
    use pangocairo::pango;
    let l = pangocairo::functions::create_layout(cr);
    let mut fd = pango::FontDescription::from_string("Sans Bold");
    fd.set_absolute_size(size * pango::SCALE as f64);
    l.set_font_description(Some(&fd));
    l.set_text(s);
    l
}

fn text_size(cr: &gtk::cairo::Context, s: &str, size: f64) -> (f64, f64) {
    let (w, h) = layout(cr, s, size).pixel_size();
    (w as f64, h as f64)
}

fn centered_text(cr: &gtk::cairo::Context, s: &str, x: f64, y: f64, size: f64, alpha: f64) {
    let l = layout(cr, s, size);
    let (w, h) = l.pixel_size();
    cr.move_to(x - w as f64 / 2.0, y - h as f64 / 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    pangocairo::functions::show_layout(cr, &l);
    cr.new_path();
}

/// Etiqueta pequeña (una tecla) centrada en un punto.
fn pill(cr: &gtk::cairo::Context, label: &str, x: f64, y: f64, strong: bool) {
    let size = 12.0;
    let (tw, th) = text_size(cr, label, size);
    let (w, h) = ((tw + 14.0).max(24.0), th + 6.0);
    rounded(cr, x - w / 2.0, y - h / 2.0, w, h, h / 2.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, if strong { 0.62 } else { 0.42 });
    let _ = cr.fill_preserve();
    cr.set_source_rgba(1.0, 1.0, 1.0, if strong { 0.55 } else { 0.22 });
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    centered_text(cr, label, x, y, size, 0.92);
}

fn label_of(keys: &[Key]) -> String {
    if keys.is_empty() {
        return "?".into();
    }
    keys.iter().take(2).map(|k| profile::key_label(*k)).collect::<Vec<_>>().join(" / ")
}

fn draw(s: &State, cr: &gtk::cairo::Context) {
    let edit = s.mode == Mode::Edit;
    if edit {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.38);
        let _ = cr.paint();
    }
    // botón ⋮: discreto, pegado al borde derecho
    let (mx, my, mw, mh) = menu_rect(s.view);
    rounded(cr, mx, my, mw + 10.0, mh, 9.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
    let _ = cr.fill();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.85);
    for k in 0..3 {
        cr.arc(mx + mw / 2.0, my + mh / 2.0 + (k as f64 - 1.0) * 9.0, 2.2, 0.0, 2.0 * std::f64::consts::PI);
        let _ = cr.fill();
    }
    if s.hints || edit {
        for (i, w) in s.profile.widgets.iter().enumerate() {
            // que ningún trazo arranque desde donde terminó el texto anterior
            cr.new_path();
            let (x, y, rw, rh) = s.rect(w);
            let (cx, cy) = (x + rw / 2.0, y + rh / 2.0);
            let selected = edit && s.editor.selected == Some(i);
            if edit {
                // en el editor se ve el área real de cada control
                match &w.kind {
                    Kind::Pad { .. } => cr.arc(cx, cy, rw.min(rh) / 2.0, 0.0, std::f64::consts::TAU),
                    _ => rounded(cr, x, y, rw, rh, 10.0),
                }
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
                let _ = cr.fill_preserve();
                if selected {
                    cr.set_source_rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.95);
                    cr.set_line_width(3.0);
                } else {
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
                    cr.set_line_width(1.5);
                }
                let _ = cr.stroke();
            }
            match &w.kind {
                Kind::Pad { up, down, left, right, .. } => {
                    let r = rw.min(rh) / 2.0;
                    if !edit {
                        cr.new_path();
                        cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
                        cr.set_line_width(1.5);
                        let _ = cr.stroke();
                    }
                    let dirs = [(up, 0.0, -1.0), (left, -1.0, 0.0), (down, 0.0, 1.0), (right, 1.0, 0.0)];
                    for (d, (keys, ox, oy)) in dirs.iter().enumerate() {
                        let strong = selected && s.editor.pad_dir == Some(d);
                        pill(cr, &label_of(keys), cx + ox * r * 0.62, cy + oy * r * 0.62, strong);
                    }
                }
                Kind::Aim { keys, .. } => {
                    if !edit {
                        cr.set_dash(&[5.0, 5.0], 0.0);
                        rounded(cr, x, y, rw, rh, 10.0);
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.22);
                        cr.set_line_width(1.2);
                        let _ = cr.stroke();
                        cr.set_dash(&[], 0.0);
                    }
                    pill(cr, &format!("Apuntar: {}", label_of(keys)), cx, cy, s.aim.is_some());
                }
                Kind::Tap(keys) => pill(cr, &label_of(keys), cx, cy, s.taps.contains_key(&i)),
                Kind::Fire(keys) => pill(cr, &format!("Disparo: {}", label_of(keys)), cx, cy, s.fires.contains_key(&i)),
            }
        }
    }
    if edit {
        for (tool, label, (x, y, w, h)) in toolbar(s.view) {
            rounded(cr, x, y, w, h, 8.0);
            let delete = matches!(tool, Tool::Delete) && s.editor.selected.is_none();
            match tool {
                Tool::Done => cr.set_source_rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.95),
                _ => cr.set_source_rgba(0.14, 0.14, 0.14, if delete { 0.55 } else { 0.92 }),
            }
            let _ = cr.fill();
            centered_text(cr, label, x + w / 2.0, y + h / 2.0, 13.0, if delete { 0.4 } else { 0.95 });
        }
    }
    if let Some(t) = &s.toast {
        let size = 13.0;
        let (tw, th) = text_size(cr, t, size);
        let (w, h) = (tw + 32.0, th + 16.0);
        let y = if edit { 64.0 } else { 18.0 };
        rounded(cr, (s.view.0 - w) / 2.0, y, w, h, h / 2.0);
        cr.set_source_rgba(0.08, 0.08, 0.08, 0.82);
        let _ = cr.fill();
        centered_text(cr, t, s.view.0 / 2.0, y + h / 2.0, size, 0.95);
    }
}

/// Dibuja la capa en un PNG (para revisarla sin mirar la pantalla).
fn snapshot(s: &State, path: &str) -> Result<(), String> {
    let (w, h) = (s.view.0.max(1.0) as i32, s.view.1.max(1.0) as i32);
    let mut surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h).map_err(|e| e.to_string())?;
    {
        let cr = gtk::cairo::Context::new(&surface).map_err(|e| e.to_string())?;
        draw(s, &cr);
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().map_err(|e| e.to_string())?.to_vec();
    let tex = gdk::MemoryTexture::new(w, h, gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from_owned(data), stride);
    tex.save_to_png(path).map_err(|e| e.to_string())
}

// --- Guion de pruebas (PLAYNIX_CONTROLS_SCRIPT) ---------------------------------

/// Una orden por línea: `wait MS`, `key NOMBRE down|up|tap`, `button N down|up X Y
/// [shift]`, `click N X Y [shift]`, `move X Y`, `drag X1 Y1 X2 Y2`, `scroll DY [ctrl]`, `text …`,
/// `rel DX DY` (movimiento al apuntar), `snapshot ARCHIVO`, `quit`.
async fn run_script(st: St, path: String) {
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let wait = |ms: u64| glib::timeout_future(Duration::from_millis(ms));
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let shift = if f.last() == Some(&"shift") { gdk::ModifierType::SHIFT_MASK } else { gdk::ModifierType::empty() };
        eprintln!("playnix-guion: {line}");
        match f[0] {
            "wait" => wait(num(1) as u64).await,
            "key" => {
                let Some(Key::Kb(k)) = f.get(1).and_then(|n| profile::key_from_name(n)) else { continue };
                match f.get(2).copied().unwrap_or("tap") {
                    "down" => on_key(&st, k, true, gdk::ModifierType::empty()),
                    "up" => on_key(&st, k, false, gdk::ModifierType::empty()),
                    _ => {
                        on_key(&st, k, true, gdk::ModifierType::empty());
                        wait(80).await;
                        on_key(&st, k, false, gdk::ModifierType::empty());
                    }
                }
            }
            "button" => on_button(&st, num(1) as u32, f.get(2) == Some(&"down"), (num(3), num(4)), shift),
            "click" => {
                on_button(&st, num(1) as u32, true, (num(2), num(3)), shift);
                wait(80).await;
                on_button(&st, num(1) as u32, false, (num(2), num(3)), shift);
            }
            "move" => on_motion(&st, (num(1), num(2))),
            "drag" => {
                let (a, b) = ((num(1), num(2)), (num(3), num(4)));
                on_button(&st, 1, true, a, gdk::ModifierType::empty());
                for i in 1..=20 {
                    let t = i as f64 / 20.0;
                    wait(16).await;
                    on_motion(&st, (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
                }
                on_button(&st, 1, false, b, gdk::ModifierType::empty());
            }
            "scroll" => {
                let ctrl = if f.last() == Some(&"ctrl") { gdk::ModifierType::CONTROL_MASK } else { gdk::ModifierType::empty() };
                on_scroll(&st, 0.0, num(1), ctrl)
            }
            "text" => st.borrow().link.text(&f[1..].join(" ")),
            "rel" => aim_move(&st, num(1), num(2)),
            "snapshot" => {
                if let Err(e) = snapshot(&st.borrow(), f.get(1).copied().unwrap_or("/tmp/playnix-capa.png")) {
                    eprintln!("playnix-guion: snapshot: {e}");
                }
            }
            "quit" => {
                st.borrow_mut().lift_all();
                let win = st.borrow().win.upgrade();
                if let Some(w) = win {
                    w.close();
                }
                return;
            }
            other => eprintln!("playnix-guion: orden desconocida {other}"),
        }
    }
}

// --- Ventana -------------------------------------------------------------------

fn install_css() {
    let css = gtk::CssProvider::new();
    // Por encima de la prioridad de usuario: un tema en ~/.config/gtk-4.0 le pondría
    // fondo a la ventana y taparía el juego
    css.load_from_string(
        "window.playnix-controles, window.playnix-controles > * { background: none; background-color: transparent; box-shadow: none; }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
    }
}

/// Marca de "este juego ya tiene su capa" (al abrirlo otra vez desde el menú no
/// debe aparecer una segunda). Contiene el PID dueño; si ese proceso ya no existe,
/// la marca es vieja y se reemplaza.
fn claim(pkg: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("playnix-controles-{pkg}.pid"));
    if let Ok(pid) = std::fs::read_to_string(&path) {
        let alive = std::fs::read(format!("/proc/{}/cmdline", pid.trim()))
            .is_ok_and(|c| String::from_utf8_lossy(&c).contains("playnix"));
        if alive {
            return None;
        }
    }
    std::fs::write(&path, std::process::id().to_string()).ok()?;
    Some(path)
}

/// Activa los controles sobre la app `pkg` (ya lanzada). `failed` recibe el error
/// si no se pudo conectar; el juego sigue abierto igual.
pub fn start(app: &adw::Application, pkg: String, profile: Profile, failed: impl FnOnce(String) + 'static) {
    let Some(mark) = claim(&pkg) else { return };
    let hold = app.hold();
    let app = app.clone();
    glib::spawn_future_local(async move {
        let p = pkg.clone();
        let link = gio::spawn_blocking(move || {
            let link = Link::connect()?;
            // esperar a que el juego tenga ventana para quedar encima de ella
            for _ in 0..60 {
                if link.running(&p) {
                    std::thread::sleep(Duration::from_millis(1500));
                    let full = link.fullscreen(&p);
                    return Ok((link, full));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            Err("el juego no arrancó".to_string())
        })
        .await
        .unwrap_or_else(|_| Err("error interno".into()));
        let (link, full) = match link {
            Ok((l, full)) => (Arc::new(l), full),
            Err(e) => {
                let _ = std::fs::remove_file(&mark);
                failed(e);
                drop(hold);
                return;
            }
        };

        install_css();
        let area = gtk::DrawingArea::new();
        let win = gtk::ApplicationWindow::builder()
            .application(&app)
            .title(format!("{} · controles", crate::android::app_name(&pkg)))
            .icon_name("playnix")
            .decorated(false)
            .child(&area)
            .build();
        win.add_css_class("playnix-controles");
        win.remove_css_class("background");
        let empty = profile.widgets.is_empty();
        let st: St = Rc::new(RefCell::new(State {
            link: link.clone(),
            pkg: pkg.clone(),
            profile,
            area: area.clone(),
            win: win.downgrade(),
            lock: None,
            view: (1.0, 1.0),
            mode: Mode::Play,
            editor: Editor::default(),
            taps: HashMap::new(),
            fires: HashMap::new(),
            pads: Vec::new(),
            free: (0..10).rev().collect(),
            held: HashSet::new(),
            drag: None,
            pinch: None,
            aim: None,
            pointer: (0.0, 0.0),
            hints: !empty,
            toast: None,
            toast_gen: 0,
        }));
        st.borrow_mut().rebuild_pads();

        let s = st.clone();
        area.set_draw_func(move |_, cr, _, _| draw(&s.borrow(), cr));
        let s = st.clone();
        area.connect_resize(move |a, w, h| {
            let mut s = s.borrow_mut();
            s.view = (w.max(1) as f64, h.max(1) as f64);
            // un perfil nuevo se guarda en las coordenadas de esta pantalla
            if s.profile.widgets.is_empty() {
                (s.profile.width, s.profile.height) = s.view;
            }
            a.queue_draw();
        });
        let events = gtk::EventControllerLegacy::new();
        events.set_propagation_phase(gtk::PropagationPhase::Capture);
        let s = st.clone();
        events.connect_event(move |_, ev| on_event(&s, ev));
        win.add_controller(events);
        let s = st.clone();
        win.connect_close_request(move |_| {
            s.borrow_mut().lift_all();
            let _ = std::fs::remove_file(&mark);
            glib::Propagation::Proceed
        });
        let actions = gio::SimpleActionGroup::new();
        let action = |name: &str, f: Box<dyn Fn(&St)>| {
            let a = gio::SimpleAction::new(name, None);
            let s = st.clone();
            a.connect_activate(move |_, _| f(&s));
            actions.add_action(&a);
        };
        action("edit", Box::new(|s| if s.borrow().mode == Mode::Play { edit_enter(s) } else { edit_exit(s) }));
        action("hints", Box::new(|s| on_key(s, gdk::Key::F2, true, gdk::ModifierType::empty())));
        action("help", Box::new(show_help));
        action("close", Box::new(|s| {
            if let Some(w) = s.borrow().win.upgrade() {
                w.close();
            }
        }));
        win.insert_action_group("ctl", Some(&actions));
        // La capa ocupa lo mismo que la ventana del juego, que va maximizada
        if !full {
            eprintln!("playnix: el juego no quedó en pantalla completa en Android");
        }
        win.maximize();
        win.present();
        drop(hold);

        if empty {
            toast(&st, "Arrastra con el mouse · Shift + clic der. agranda · Shift + clic izq. encoge · ⋮ a la derecha: crear controles y ayuda", 7);
        } else {
            toast(&st, "Controles activos · ⋮ a la derecha: editar, ver teclas y ayuda", HINTS_SECS);
        }
        if let Ok(path) = std::env::var("PLAYNIX_CONTROLS_SCRIPT") {
            glib::spawn_future_local(run_script(st.clone(), path));
        }

        // Al cerrarse el juego se cierran los controles (y con ellos el servidor)
        let w = win.downgrade();
        glib::spawn_future_local(async move {
            let mut misses = 0;
            loop {
                glib::timeout_future_seconds(2).await;
                let Some(win) = w.upgrade() else { break };
                let (l, p) = (link.clone(), pkg.clone());
                let alive = gio::spawn_blocking(move || {
                    l.refresh_size();
                    l.running(&p)
                })
                .await
                .unwrap_or(true);
                misses = if alive { 0 } else { misses + 1 };
                if misses >= 2 {
                    win.close();
                    break;
                }
            }
        });
    });
}
