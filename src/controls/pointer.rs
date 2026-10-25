//! Bloqueo del puntero para apuntar con el mouse: el cursor queda fijo y llega el
//! movimiento relativo (como en un juego de PC). Protocolos pointer-constraints y
//! relative-pointer de Wayland, sobre la misma conexión que usa GTK.

use gtk::gdk;
use gtk::glib;
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::pointer_constraints::zv1::client::{
    zwp_locked_pointer_v1::{self, ZwpLockedPointerV1},
    zwp_pointer_constraints_v1::{self, ZwpPointerConstraintsV1},
};
use wayland_protocols::wp::relative_pointer::zv1::client::{
    zwp_relative_pointer_manager_v1::{self, ZwpRelativePointerManagerV1},
    zwp_relative_pointer_v1::{self, ZwpRelativePointerV1},
};

unsafe extern "C" {
    fn gdk_wayland_display_get_wl_display(display: *mut gdk::ffi::GdkDisplay) -> *mut std::ffi::c_void;
    fn gdk_wayland_surface_get_wl_surface(surface: *mut gdk::ffi::GdkSurface) -> *mut std::ffi::c_void;
}

/// Movimiento acumulado entre despachos (sin aceleración del sistema).
#[derive(Default)]
struct Events {
    dx: f64,
    dy: f64,
    locked: bool,
}

pub struct PointerLock {
    conn: Connection,
    queue: RefCell<EventQueue<Events>>,
    events: RefCell<Events>,
    qh: QueueHandle<Events>,
    surface: WlSurface,
    pointer: wl_pointer::WlPointer,
    constraints: ZwpPointerConstraintsV1,
    relative: ZwpRelativePointerManagerV1,
    active: RefCell<Option<(ZwpLockedPointerV1, ZwpRelativePointerV1, glib::SourceId)>>,
}

impl PointerLock {
    /// None si la sesión no es Wayland o el compositor no admite los protocolos.
    pub fn new(win: &gtk::Window) -> Option<Rc<PointerLock>> {
        let display = WidgetExt::display(win);
        if display.type_().name() != "GdkWaylandDisplay" {
            return None;
        }
        let surface = win.surface()?;
        let (wl_display, wl_surface) = unsafe {
            (
                gdk_wayland_display_get_wl_display(display.to_glib_none().0),
                gdk_wayland_surface_get_wl_surface(surface.to_glib_none().0),
            )
        };
        if wl_display.is_null() || wl_surface.is_null() {
            return None;
        }
        let backend = unsafe { Backend::from_foreign_display(wl_display.cast()) };
        let conn = Connection::from_backend(backend);
        let sid = unsafe { ObjectId::from_ptr(WlSurface::interface(), wl_surface.cast()) }.ok()?;
        let surface = WlSurface::from_id(&conn, sid).ok()?;
        let (globals, queue) = registry_queue_init::<Events>(&conn).ok()?;
        let qh = queue.handle();
        let constraints: ZwpPointerConstraintsV1 = globals.bind(&qh, 1..=1, ()).ok()?;
        let relative: ZwpRelativePointerManagerV1 = globals.bind(&qh, 1..=1, ()).ok()?;
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).ok()?;
        let pointer = seat.get_pointer(&qh, ());
        let _ = conn.flush();
        Some(Rc::new(PointerLock {
            conn,
            queue: RefCell::new(queue),
            events: RefCell::new(Events::default()),
            qh,
            surface,
            pointer,
            constraints,
            relative,
            active: RefCell::new(None),
        }))
    }

    /// Fija el cursor y llama `motion(dx, dy)` con el movimiento hasta `unlock`.
    pub fn lock(self: &Rc<Self>, motion: impl Fn(f64, f64) + 'static) {
        if self.active.borrow().is_some() {
            return;
        }
        let locked = self.constraints.lock_pointer(
            &self.surface,
            &self.pointer,
            None,
            zwp_pointer_constraints_v1::Lifetime::Persistent,
            &self.qh,
            (),
        );
        let rel = self.relative.get_relative_pointer(&self.pointer, &self.qh, ());
        let _ = self.conn.flush();
        // GTK lee el socket; aquí solo se despachan los eventos de esta cola
        let weak = Rc::downgrade(self);
        let tick = glib::timeout_add_local(std::time::Duration::from_millis(4), move || {
            let Some(me) = weak.upgrade() else { return glib::ControlFlow::Break };
            let _ = me.queue.borrow_mut().dispatch_pending(&mut me.events.borrow_mut());
            let (dx, dy) = {
                let mut e = me.events.borrow_mut();
                (std::mem::take(&mut e.dx), std::mem::take(&mut e.dy))
            };
            if dx != 0.0 || dy != 0.0 {
                motion(dx, dy);
            }
            glib::ControlFlow::Continue
        });
        *self.active.borrow_mut() = Some((locked, rel, tick));
    }

    pub fn unlock(&self) {
        if let Some((locked, rel, tick)) = self.active.borrow_mut().take() {
            tick.remove();
            locked.destroy();
            rel.destroy();
            let _ = self.conn.flush();
            *self.events.borrow_mut() = Events::default();
        }
    }

    pub fn is_locked(&self) -> bool {
        self.active.borrow().is_some()
    }
}

impl Drop for PointerLock {
    fn drop(&mut self) {
        self.unlock();
        self.pointer.release();
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Events {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_seat::WlSeat, ()> for Events {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_pointer::WlPointer, ()> for Events {
    fn event(_: &mut Self, _: &wl_pointer::WlPointer, _: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpPointerConstraintsV1, ()> for Events {
    fn event(_: &mut Self, _: &ZwpPointerConstraintsV1, _: zwp_pointer_constraints_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpLockedPointerV1, ()> for Events {
    fn event(s: &mut Self, _: &ZwpLockedPointerV1, ev: zwp_locked_pointer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            zwp_locked_pointer_v1::Event::Locked => s.locked = true,
            zwp_locked_pointer_v1::Event::Unlocked => s.locked = false,
            _ => {}
        }
    }
}

impl Dispatch<ZwpRelativePointerManagerV1, ()> for Events {
    fn event(_: &mut Self, _: &ZwpRelativePointerManagerV1, _: zwp_relative_pointer_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpRelativePointerV1, ()> for Events {
    fn event(s: &mut Self, _: &ZwpRelativePointerV1, ev: zwp_relative_pointer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zwp_relative_pointer_v1::Event::RelativeMotion { dx_unaccel, dy_unaccel, .. } = ev {
            s.dx += dx_unaccel;
            s.dy += dy_unaccel;
        }
    }
}
