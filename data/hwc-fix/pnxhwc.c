// Reemplazo de split_layer_names_helper::split_layer_name para hwcomposer.waydroid.
//
// El hwcomposer de Waydroid solo asigna una capa a una ventana si su nombre empieza
// con "TID:<tarea>#". Waydroid añade ese prefijo a las SurfaceView solo cuando su
// contexto es directamente una Activity; si no (p. ej. Clash Royale), la capa del
// juego se descarta y la ventana queda transparente. Aquí, si una capa sin prefijo
// pertenece a un paquete que ya tiene ventana, se le antepone el TID de esa ventana
// y se llama a la función original.
//
// Sin cabeceras de bionic: se declara a mano lo poco que se usa.

typedef unsigned long size_t;
typedef long ssize_t;
typedef unsigned long uintptr_t;

#define SPLIT_SYM "_ZN24split_layer_names_helper16split_layer_nameERKNSt3__112basic_stringIcNS0_11char_traitsIcEENS0_9allocatorIcEEEE"

// std::__1::string (libc++, x86_64): bit 0 del primer byte indica cadena larga.
typedef struct { unsigned char raw[24]; } lstr;
extern void lstr_init(lstr *, const char *, size_t)
    __asm__("_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEE6__initEPKcm");
extern void lstr_dtor(lstr *)
    __asm__("_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEED1Ev");

static const char *lstr_data(const lstr *s, size_t *n) {
    if (s->raw[0] & 1) {
        *n = *(const size_t *)(s->raw + 8);
        return *(const char *const *)(s->raw + 16);
    }
    *n = s->raw[0] >> 1;
    return (const char *)s->raw + 1;
}

// bionic
extern int open(const char *, int, ...);
extern ssize_t pread(int, void *, size_t, long);
extern int close(int);
extern void abort(void);
extern int __android_log_print(int, const char *, const char *, ...);

typedef struct { unsigned int p_type, p_flags; unsigned long p_offset, p_vaddr, p_paddr, p_filesz, p_memsz, p_align; } Phdr;
struct dl_phdr_info { uintptr_t dlpi_addr; const char *dlpi_name; const Phdr *dlpi_phdr; unsigned short dlpi_phnum; };
extern int dl_iterate_phdr(int (*)(struct dl_phdr_info *, size_t, void *), void *);

typedef struct { unsigned char e_ident[16]; unsigned short e_type, e_machine; unsigned int e_version;
    unsigned long e_entry, e_phoff, e_shoff; unsigned int e_flags; unsigned short e_ehsize, e_phentsize,
    e_phnum, e_shentsize, e_shnum, e_shstrndx; } Ehdr;
typedef struct { unsigned int sh_name, sh_type; unsigned long sh_flags, sh_addr, sh_offset, sh_size;
    unsigned int sh_link, sh_info; unsigned long sh_addralign, sh_entsize; } Shdr;
typedef struct { unsigned int st_name; unsigned char st_info, st_other; unsigned short st_shndx;
    unsigned long st_value, st_size; } Sym;

#define TAG "playnix-hwc"

static size_t slen(const char *s) { size_t n = 0; while (s[n]) n++; return n; }
static int seq(const char *a, size_t n, const char *b) {
    for (size_t i = 0; i < n; i++) if (a[i] != b[i]) return 0;
    return b[n] == 0;
}
static const char *mchr(const char *s, size_t n, char c) {
    for (size_t i = 0; i < n; i++) if (s[i] == c) return s + i;
    return 0;
}

typedef void *(*split_fn)(void *, const lstr *);
static split_fn orig;

struct find { uintptr_t addr, bias; const char *name; };
static int find_lib(struct dl_phdr_info *info, size_t sz, void *data) {
    (void)sz;
    struct find *f = data;
    for (int i = 0; i < info->dlpi_phnum; i++) {
        const Phdr *p = &info->dlpi_phdr[i];
        if (p->p_type == 1 && f->addr >= info->dlpi_addr + p->p_vaddr
            && f->addr < info->dlpi_addr + p->p_vaddr + p->p_memsz) {
            f->bias = info->dlpi_addr;
            f->name = info->dlpi_name;
            return 1;
        }
    }
    return 0;
}

// Las definiciones originales siguen en el hwcomposer (con enlace local): se buscan
// en su .dynsym en disco y se suman a la base donde quedó cargado.
static const char *hwc_name;
static uintptr_t hwc_bias;

static void *resolve(uintptr_t caller, const char *sym) {
    if (!hwc_name) {
        struct find f = { caller, 0, 0 };
        if (!dl_iterate_phdr(find_lib, &f) || !f.name) return 0;
        hwc_name = f.name;
        hwc_bias = f.bias;
    }
    int fd = open(hwc_name, 0x80000 /* O_RDONLY|O_CLOEXEC */);
    if (fd < 0) return 0;
    void *fn = 0;
    Ehdr eh;
    Shdr sh[128];
    if (pread(fd, &eh, sizeof eh, 0) != sizeof eh || eh.e_shnum > 128
        || pread(fd, sh, eh.e_shnum * sizeof(Shdr), eh.e_shoff) != (ssize_t)(eh.e_shnum * sizeof(Shdr)))
        goto out;
    size_t want = slen(sym) + 1;
    if (want > 160) goto out;
    for (int i = 0; i < eh.e_shnum && !fn; i++) {
        if (sh[i].sh_type != 11 /* SHT_DYNSYM */) continue;
        const Shdr *str = &sh[sh[i].sh_link];
        char name[160];
        for (unsigned long off = 0; off < sh[i].sh_size; off += sizeof(Sym)) {
            Sym s;
            if (pread(fd, &s, sizeof s, sh[i].sh_offset + off) != sizeof s) break;
            if (s.st_shndx == 0 || s.st_name >= str->sh_size) continue;
            if (pread(fd, name, want, str->sh_offset + s.st_name) != (ssize_t)want) continue;
            if (!name[want - 1] && seq(name, want - 1, sym)) { fn = (void *)(hwc_bias + s.st_value); break; }
        }
    }
out:
    close(fd);
    return fn;
}

// paquete → TID de su ventana, aprendido de las capas que sí traen prefijo.
#define SLOTS 64
static struct { char pkg[128]; char tid[16]; } known[SLOTS];
static unsigned next_slot;

static void remember(const char *pkg, size_t pn, const char *tid, size_t tn) {
    if (pn >= sizeof known[0].pkg || tn >= sizeof known[0].tid) return;
    unsigned slot = SLOTS;
    for (unsigned i = 0; i < SLOTS; i++)
        if (seq(pkg, pn, known[i].pkg)) { slot = i; break; }
    if (slot == SLOTS) slot = next_slot++ % SLOTS;
    for (size_t i = 0; i < pn; i++) known[slot].pkg[i] = pkg[i];
    known[slot].pkg[pn] = 0;
    for (size_t i = 0; i < tn; i++) known[slot].tid[i] = tid[i];
    known[slot].tid[tn] = 0;
}

static const char *lookup(const char *pkg, size_t pn) {
    for (unsigned i = 0; i < SLOTS; i++)
        if (known[i].pkg[0] && seq(pkg, pn, known[i].pkg)) return known[i].tid;
    return 0;
}

__attribute__((visibility("default")))
void *split_layer_name(void *ret, const lstr *name) __asm__(SPLIT_SYM);

void *split_layer_name(void *ret, const lstr *name) {
    if (!orig) {
        orig = (split_fn)resolve((uintptr_t)__builtin_return_address(0), SPLIT_SYM);
        if (!orig) {
            __android_log_print(6, TAG, "no se encontró split_layer_name original");
            abort();
        }
    }
    size_t n;
    const char *d = lstr_data(name, &n);

    if (n > 4 && seq(d, 4, "TID:")) {
        const char *hash = mchr(d, n, '#');
        const char *slash = hash ? mchr(hash, n - (hash - d), '/') : 0;
        if (slash) remember(hash + 1, slash - hash - 1, d + 4, hash - d - 4);
        return orig(ret, name);
    }

    const char *p = d;
    size_t pn = n;
    if (pn > 12 && seq(p, 12, "SurfaceView[")) { p += 12; pn -= 12; }
    const char *slash = mchr(p, pn, '/');
    const char *hash = mchr(p, pn, '#');
    const char *tid = (slash && (!hash || slash < hash)) ? lookup(p, slash - p) : 0;
    if (!tid) return orig(ret, name);

    char buf[512];
    size_t tn = slen(tid), len = 4 + tn + 1 + pn;
    if (len > sizeof buf) return orig(ret, name);
    size_t k = 0;
    buf[k++] = 'T'; buf[k++] = 'I'; buf[k++] = 'D'; buf[k++] = ':';
    for (size_t i = 0; i < tn; i++) buf[k++] = tid[i];
    buf[k++] = '#';
    for (size_t i = 0; i < pn; i++) buf[k++] = p[i];

    lstr fixed;
    lstr_init(&fixed, buf, len);
    orig(ret, &fixed);
    lstr_dtor(&fixed);
    return ret;
}

// ---------------------------------------------------------------------------------
// Mover la ventana desde la barra de título de Android.
//
// En modo multiventana cada tarea de Android (freeform) se muestra en su propia
// ventana de Wayland, pero la barra de título (←, _, □, ✕) la dibuja Android. Al
// arrastrarla, Android movía la tarea dentro de su pantalla virtual y la ventana de
// Linux se quedaba quieta. Aquí, un arrastre que empieza en esa barra se convierte
// en xdg_toplevel.move: el escritorio mueve la ventana como la de cualquier programa.
// Un clic sin arrastre llega a Android igual que antes (los botones siguen sirviendo).
//
// Para que el escritorio la mueva bien, la ventana tiene que medir lo que mide la app:
// Waydroid hace cada ventana del tamaño de toda la pantalla de Android (transparente,
// con la app dibujada en su posición), y GNOME arrastraba ese rectángulo invisible:
// chocaba con los bordes, iba a saltos y ofrecía acomodarla en mosaico. Aquí la
// geometría de la ventana (xdg_surface.set_window_geometry) es el rectángulo de la app,
// el mismo que Waydroid calcula para su región de entrada, y la ventana tiene tamaño
// fijo (mínimo = máximo) salvo al maximizarse o pasar a pantalla completa. Además, el
// tamaño que el escritorio propone para una ventana normal ya no cambia el de la
// pantalla de Android (Waydroid lo aplicaba a toda la pantalla).
//
// Se interceptan wl_proxy_add_listener y wl_proxy_marshal (enlace local en el
// hwcomposer, ver hwc-patch.py): la primera para envolver los listeners del puntero,
// el teclado y las ventanas; la segunda para ver y ajustar sus peticiones.

#define ADD_LISTENER_SYM "wl_proxy_add_listener"
#define BTN_LEFT 0x110
#define MARSHAL_SYM "wl_proxy_marshal"
// Opcodes de las peticiones (protocolos wayland y xdg-shell)
#define WL_SURFACE_SET_INPUT_REGION 5
#define WL_REGION_DESTROY 0
#define WL_REGION_ADD 1
#define WL_REGION_SUBTRACT 2
#define XDG_SURFACE_DESTROY 0
#define XDG_SURFACE_SET_WINDOW_GEOMETRY 3
#define XDG_TOPLEVEL_DESTROY 0
#define XDG_TOPLEVEL_SET_APP_ID 3
#define XDG_TOPLEVEL_MOVE 5
#define XDG_TOPLEVEL_SET_MAX_SIZE 7
#define XDG_TOPLEVEL_SET_MIN_SIZE 8
#define XDG_TOPLEVEL_SET_MAXIMIZED 9
#define XDG_TOPLEVEL_UNSET_MAXIMIZED 10
#define XDG_TOPLEVEL_SET_FULLSCREEN 11
// Margen que Waydroid añade alrededor de la app en la región de entrada
// (WINDOW_DECORATION_OUTSET en hwcomposer.cpp)
#define INPUT_OUTSET 15
#define STATE_MAXIMIZED 1
#define STATE_FULLSCREEN 2
// Altura de la barra de título de Android a 180 dpi, en píxeles de Android
#define CAPTION_PX_180DPI 48
// Píxeles que hay que arrastrar antes de mover (menos es un clic)
#define DRAG_THRESHOLD 4
// Se copian de más: los que no se envuelven quedan igual que en el original
#define LISTENER_SLOTS 12

struct wl_proxy;
struct wl_array { size_t size, alloc; void *data; };
typedef void (*fnptr)(void);
typedef int (*add_listener_fn)(struct wl_proxy *, fnptr *, void *);
typedef const char *(*get_class_fn)(struct wl_proxy *);
typedef void *(*get_user_data_fn)(struct wl_proxy *);
typedef void (*marshal_fn)(struct wl_proxy *, unsigned, ...);
extern int __system_property_get(const char *, char *);

static add_listener_fn real_add_listener;
static get_class_fn get_class;
static get_user_data_fn get_user_data;
static marshal_fn marshal;

static struct wl_proxy *seat;
// Ventanas: datos del listener (la `window` del hwcomposer, que también es el
// user_data de su wl_surface principal) → su xdg_toplevel y sus estados.
#define WINDOWS 64
static struct {
    void *window;
    struct wl_proxy *xdg_surface, *toplevel;
    int maximized, fullscreen;
    int gx, gy, gw, gh;  // geometría enviada (gw = 0: todavía no se conoce)
    int fixed;           // tamaño mínimo y máximo puestos
    int full;            // la app ocupa toda la pantalla de Android (-1: no se sabe)
    struct wl_proxy *surface;  // su wl_surface principal
    char app_id[160];          // "waydroid.<paquete>"
} wins[WINDOWS];
static unsigned next_win;
// Ventana con el foco del teclado: la que se acaba de clicar
static void *focused;

static int slot_of_window(void *window) {
    for (int i = 0; i < WINDOWS; i++) if (wins[i].window == window && wins[i].toplevel) return i;
    return -1;
}
static int slot_of_toplevel(struct wl_proxy *t) {
    for (int i = 0; i < WINDOWS; i++) if (wins[i].toplevel == t) return i;
    return -1;
}
static int slot_of_xdg_surface(struct wl_proxy *x) {
    for (int i = 0; i < WINDOWS; i++) if (wins[i].xdg_surface == x) return i;
    return -1;
}

// Regiones de entrada: rectángulo que abarca todo lo añadido desde que se vaciaron
#define REGIONS 64
static struct { struct wl_proxy *region; int x0, y0, x1, y1, valid; } regions[REGIONS];
static unsigned next_region;

static int slot_of_region(struct wl_proxy *r, int create) {
    for (int i = 0; i < REGIONS; i++) if (regions[i].region == r) return i;
    if (!create) return -1;
    int i = next_region++ % REGIONS;
    regions[i].region = r;
    regions[i].valid = 0;
    return i;
}

// Controles dentro de la ventana (al final del archivo); eventos para Playnix
#define EV_SIZE 1
#define EV_MOTION 2
#define EV_BUTTON 3
#define EV_AXIS 4
#define EV_KEY 5
#define EV_MODS 6
#define EV_REL 7
#define EV_LEAVE 8
#define EV_ENTER 9
#define EV_FOCUS 10
#define EV_GONE 11
struct ctl;
static struct ctl *pointer_ctl;
static struct ctl *ctl_of_surface(struct wl_proxy *surface);
static void ctl_pointer(struct ctl *c, unsigned type, const int *v, unsigned n);
static int ctl_key(unsigned key, unsigned state);
static void ctl_mods(const int *v);
static void ctl_focus(void *window, int on);
static void ctl_window_updated(int i);
static void ctl_window_gone(void *window);
static void ctl_start(uintptr_t caller);
static void ctl_cursor(unsigned serial);
static unsigned cursor_serial;
static void registry_global(void *, struct wl_proxy *, unsigned, const char *, unsigned);
static fnptr registry_orig[LISTENER_SLOTS], registry_wrap[LISTENER_SLOTS];
static struct wl_proxy *pointer_proxy;

static int prop_int(const char *name, int def) {
    char v[92];
    if (__system_property_get(name, v) <= 0) return def;
    int n = 0;
    for (int i = 0; v[i] >= '0' && v[i] <= '9'; i++) n = n * 10 + v[i] - '0';
    return n > 0 ? n : def;
}

// Píxeles de Android → coordenadas de Wayland (lógicas): escala 1 en la mayoría de
// escritorios; si hay escalado, waydroid.display_scale trae el factor ("1.250000").
static int to_logical(int px) {
    char v[92];
    if (__system_property_get("waydroid.display_scale", v) > 0) {
        int whole = 0, frac = 0, div = 1, i = 0;
        for (; v[i] >= '0' && v[i] <= '9'; i++) whole = whole * 10 + v[i] - '0';
        if (v[i] == '.')
            for (i++; v[i] >= '0' && v[i] <= '9' && div < 1000; i++) { frac = frac * 10 + v[i] - '0'; div *= 10; }
        int scale1000 = whole * 1000 + frac * 1000 / div;
        if (scale1000 > 0) px = px * 1000 / scale1000;
    }
    return px;
}

// Alto de la barra de título de Android
static int caption_height(void) {
    return to_logical(CAPTION_PX_180DPI * prop_int("ro.sf.lcd_density", 180) / 180);
}

// ¿La app ocupa toda la pantalla de Android? (maximizada con su botón □, o juego en
// pantalla completa): entonces no hay ventana que mover.
static int fills_display(int i) {
    int w = to_logical(prop_int("waydroid.display_width", 0));
    int h = to_logical(prop_int("waydroid.display_height", 0));
    return w > 0 && h > 0 && wins[i].gw >= w - 2 && wins[i].gh >= h - 2;
}

// --- xdg_toplevel: configure(data, toplevel, w, h, states) ---
static fnptr toplevel_orig[LISTENER_SLOTS], toplevel_wrap[LISTENER_SLOTS];

static void toplevel_configure(void *data, struct wl_proxy *t, int w, int h, struct wl_array *states) {
    int i = slot_of_toplevel(t);
    if (i >= 0) {
        wins[i].maximized = wins[i].fullscreen = 0;
        unsigned *s = states->data;
        for (size_t k = 0; k < states->size / sizeof *s; k++) {
            if (s[k] == STATE_MAXIMIZED) wins[i].maximized = 1;
            if (s[k] == STATE_FULLSCREEN) wins[i].fullscreen = 1;
        }
        // Waydroid usaría el tamaño de una ventana normal para toda la pantalla de
        // Android. Con 0 x 0 lo ignora (la primera ventana, con la que mide la
        // pantalla, todavía no tiene geometría y pasa igual).
        if (wins[i].gw && !wins[i].maximized && !wins[i].fullscreen) w = h = 0;
    }
    ((void (*)(void *, struct wl_proxy *, int, int, struct wl_array *))toplevel_orig[0])(data, t, w, h, states);
}

// --- wl_keyboard: enter(data, kb, serial, surface, keys) · leave(data, kb, serial, surface) ---
static fnptr keyboard_orig[LISTENER_SLOTS], keyboard_wrap[LISTENER_SLOTS];

static void keyboard_enter(void *data, struct wl_proxy *kb, unsigned serial, struct wl_proxy *surface, struct wl_array *keys) {
    focused = surface ? get_user_data(surface) : 0;
    ctl_focus(focused, 1);
    ((void (*)(void *, struct wl_proxy *, unsigned, struct wl_proxy *, struct wl_array *))keyboard_orig[1])(data, kb, serial, surface, keys);
}

static void keyboard_leave(void *data, struct wl_proxy *kb, unsigned serial, struct wl_proxy *surface) {
    ctl_focus(focused, 0);
    focused = 0;
    ((void (*)(void *, struct wl_proxy *, unsigned, struct wl_proxy *))keyboard_orig[2])(data, kb, serial, surface);
}

// key(data, kb, serial, time, tecla, estado) · modifiers(data, kb, serial, dep, lat, lock, grupo)
static void keyboard_key(void *data, struct wl_proxy *kb, unsigned serial, unsigned time, unsigned key, unsigned state) {
    if (ctl_key(key, state)) return; // la pidió Playnix
    ((void (*)(void *, struct wl_proxy *, unsigned, unsigned, unsigned, unsigned))keyboard_orig[3])(data, kb, serial, time, key, state);
}

static void keyboard_modifiers(void *data, struct wl_proxy *kb, unsigned serial, unsigned dep, unsigned lat, unsigned lck, unsigned group) {
    int v[4] = { (int)dep, (int)lat, (int)lck, (int)group };
    ctl_mods(v);
    ((void (*)(void *, struct wl_proxy *, unsigned, unsigned, unsigned, unsigned, unsigned))keyboard_orig[4])(data, kb, serial, dep, lat, lck, group);
}

// --- wl_pointer: enter · leave · motion · button · axis ---
// Sobre la capa de controles, todo va a Playnix (el cursor lo sigue poniendo Waydroid)
typedef void (*button_fn)(void *, struct wl_proxy *, unsigned, unsigned, unsigned, unsigned);
static fnptr pointer_orig[LISTENER_SLOTS], pointer_wrap[LISTENER_SLOTS];
static int px, py;  // posición del puntero dentro de la superficie (lógica, entera)
enum { IDLE, HELD, MOVING };
static int drag;
static int drag_x, drag_y;
static unsigned held_serial, held_time;

static void pointer_enter(void *data, struct wl_proxy *p, unsigned serial, struct wl_proxy *surface, int sx, int sy) {
    drag = IDLE; // p. ej. al terminar de mover la ventana
    px = sx / 256;
    py = sy / 256;
    pointer_ctl = ctl_of_surface(surface);
    if (pointer_ctl) {
        int v[2] = { sx, sy };
        ctl_pointer(pointer_ctl, EV_ENTER, v, 2);
        ((void (*)(void *, struct wl_proxy *, unsigned, struct wl_proxy *, int, int))pointer_orig[0])(data, p, serial, surface, sx, sy);
        // Android oculta su puntero al recibir toques: sobre la capa va uno propio
        cursor_serial = serial;
        ctl_cursor(serial);
        return;
    }
    ((void (*)(void *, struct wl_proxy *, unsigned, struct wl_proxy *, int, int))pointer_orig[0])(data, p, serial, surface, sx, sy);
}

// Si se soltó sin arrastrar, Android recibe el clic que se retuvo
static void release_held(void *data, struct wl_proxy *p) {
    if (drag == HELD) ((button_fn)pointer_orig[3])(data, p, held_serial, held_time, BTN_LEFT, 1);
    drag = IDLE;
}

static void pointer_leave(void *data, struct wl_proxy *p, unsigned serial, struct wl_proxy *surface) {
    // Android nunca vio el clic retenido: se descarta
    drag = IDLE;
    if (pointer_ctl) ctl_pointer(pointer_ctl, EV_LEAVE, 0, 0);
    pointer_ctl = 0;
    ((void (*)(void *, struct wl_proxy *, unsigned, struct wl_proxy *))pointer_orig[1])(data, p, serial, surface);
}

static void pointer_motion(void *data, struct wl_proxy *p, unsigned time, int sx, int sy) {
    if (pointer_ctl) {
        int v[2] = { sx, sy };
        return ctl_pointer(pointer_ctl, EV_MOTION, v, 2);
    }
    px = sx / 256;
    py = sy / 256;
    if (drag == HELD) {
        int dx = px - drag_x, dy = py - drag_y;
        int i = slot_of_window(focused);
        if (dx * dx + dy * dy >= DRAG_THRESHOLD * DRAG_THRESHOLD && i >= 0 && seat && !fills_display(i)) {
            drag = MOVING;
            marshal(wins[i].toplevel, XDG_TOPLEVEL_MOVE, seat, held_serial);
            return;
        }
    }
    if (drag == MOVING) return;
    ((void (*)(void *, struct wl_proxy *, unsigned, int, int))pointer_orig[2])(data, p, time, sx, sy);
}

static void pointer_button(void *data, struct wl_proxy *p, unsigned serial, unsigned time, unsigned button, unsigned state) {
    if (pointer_ctl) {
        int v[2] = { (int)button, (int)state };
        return ctl_pointer(pointer_ctl, EV_BUTTON, v, 2);
    }
    if (button == BTN_LEFT) {
        if (state == 1 && drag == IDLE && py >= 0 && py < caption_height()) {
            // Si la app ocupa toda la pantalla no hay nada que mover. No sirve el estado
            // de la ventana en el escritorio: Waydroid las maximiza todas al crearlas.
            // El foco del teclado puede llegar justo después del clic: se revisa al arrastrar.
            int i = slot_of_window(focused);
            if (i < 0 || (!wins[i].fullscreen && !fills_display(i))) {
                drag = HELD;
                drag_x = px;
                drag_y = py;
                held_serial = serial;
                held_time = time;
                return;
            }
        }
        if (state == 0 && drag != IDLE) {
            int was = drag;
            release_held(data, p);
            if (was == MOVING) return;
        }
    }
    ((button_fn)pointer_orig[3])(data, p, serial, time, button, state);
}

static void pointer_axis(void *data, struct wl_proxy *p, unsigned time, unsigned axis, int value) {
    if (pointer_ctl) {
        int v[2] = { (int)axis, value };
        return ctl_pointer(pointer_ctl, EV_AXIS, v, 2);
    }
    ((void (*)(void *, struct wl_proxy *, unsigned, unsigned, int))pointer_orig[4])(data, p, time, axis, value);
}

static void wrap(fnptr *orig, fnptr *wrapped, const fnptr *impl) {
    for (int i = 0; i < LISTENER_SLOTS; i++) orig[i] = wrapped[i] = impl[i];
}

static int wl_ready;
// Waydroid ya midió la pantalla con su primera ventana
static int measured;

static void wl_init(uintptr_t caller) {
    if (wl_ready) return;
    wl_ready = 1;
    real_add_listener = (add_listener_fn)resolve(caller, ADD_LISTENER_SYM);
    get_class = (get_class_fn)resolve(caller, "wl_proxy_get_class");
    get_user_data = (get_user_data_fn)resolve(caller, "wl_proxy_get_user_data");
    marshal = (marshal_fn)resolve(caller, MARSHAL_SYM);
    if (!real_add_listener || !marshal) {
        __android_log_print(6, TAG, "no se encontraron las funciones originales de Wayland");
        abort();
    }
    if (!get_class || !get_user_data)
        __android_log_print(5, TAG, "sin funciones de Wayland: no se moverán ventanas");
    else
        ctl_start(caller);
}

__attribute__((visibility("default")))
int wl_proxy_add_listener(struct wl_proxy *proxy, fnptr *impl, void *data) {
    wl_init((uintptr_t)__builtin_return_address(0));
    if (!get_class || !get_user_data) return real_add_listener(proxy, impl, data);

    const char *cls = get_class(proxy);
    size_t n = slen(cls);
    if (seq(cls, n, "wl_registry")) {
        wrap(registry_orig, registry_wrap, impl);
        registry_wrap[0] = (fnptr)registry_global;
        impl = registry_wrap;
    } else if (seq(cls, n, "wl_seat")) {
        seat = proxy;
    } else if (seq(cls, n, "xdg_surface")) {
        // ventana nueva (Waydroid crea su xdg_surface antes que su xdg_toplevel); una
        // anterior en la misma dirección de memoria ya no existe
        for (int k = 0; k < WINDOWS; k++)
            if (wins[k].window == data) wins[k].window = wins[k].xdg_surface = wins[k].toplevel = 0;
        int i = next_win++ % WINDOWS;
        wins[i].window = data;
        wins[i].xdg_surface = proxy;
        wins[i].toplevel = 0;
        wins[i].maximized = wins[i].fullscreen = wins[i].fixed = 0;
        wins[i].gx = wins[i].gy = wins[i].gw = wins[i].gh = 0;
        wins[i].full = -1;
        wins[i].surface = 0;
        wins[i].app_id[0] = 0;
    } else if (seq(cls, n, "xdg_toplevel")) {
        int i = -1;
        for (int k = 0; k < WINDOWS; k++) if (wins[k].window == data && !wins[k].toplevel) { i = k; break; }
        if (i < 0) {
            i = next_win++ % WINDOWS;
            wins[i].window = data;
            wins[i].xdg_surface = 0;
            wins[i].fixed = wins[i].gw = 0;
            wins[i].full = -1;
        }
        wins[i].toplevel = proxy;
        wins[i].maximized = wins[i].fullscreen = 0;
        if (!toplevel_orig[0]) {
            wrap(toplevel_orig, toplevel_wrap, impl);
            toplevel_wrap[0] = (fnptr)toplevel_configure;
        }
        if (impl[0] == toplevel_orig[0]) impl = toplevel_wrap;
    } else if (seq(cls, n, "wl_keyboard")) {
        wrap(keyboard_orig, keyboard_wrap, impl);
        keyboard_wrap[1] = (fnptr)keyboard_enter;
        keyboard_wrap[2] = (fnptr)keyboard_leave;
        keyboard_wrap[3] = (fnptr)keyboard_key;
        keyboard_wrap[4] = (fnptr)keyboard_modifiers;
        impl = keyboard_wrap;
    } else if (seq(cls, n, "wl_pointer")) {
        wrap(pointer_orig, pointer_wrap, impl);
        pointer_wrap[0] = (fnptr)pointer_enter;
        pointer_wrap[1] = (fnptr)pointer_leave;
        pointer_wrap[2] = (fnptr)pointer_motion;
        pointer_wrap[3] = (fnptr)pointer_button;
        pointer_wrap[4] = (fnptr)pointer_axis;
        pointer_proxy = proxy;
        impl = pointer_wrap;
    }
    return real_add_listener(proxy, impl, data);
}

// Tamaño fijo para una ventana normal: así el escritorio no la acomoda en mosaico ni
// la maximiza al soltarla en un borde (Android no cambiaría su tamaño).
static void set_fixed(int i, int on) {
    if (!wins[i].toplevel || wins[i].fixed == on) return;
    int w = on ? wins[i].gw : 0, h = on ? wins[i].gh : 0;
    marshal(wins[i].toplevel, XDG_TOPLEVEL_SET_MIN_SIZE, w, h);
    marshal(wins[i].toplevel, XDG_TOPLEVEL_SET_MAX_SIZE, w, h);
    wins[i].fixed = on;
}

// La región de entrada de una ventana acaba de cambiar: su rectángulo (sin el margen)
// es el de la app. Se aplica con el commit que Waydroid hace justo después.
static void update_geometry(struct wl_proxy *surface, struct wl_proxy *region) {
    int r = slot_of_region(region, 0);
    int i = slot_of_window(get_user_data(surface));
    if (i >= 0) wins[i].surface = surface;
    if (r < 0 || i < 0 || !regions[r].valid || !wins[i].xdg_surface) return;
    int x = regions[r].x0 + INPUT_OUTSET, y = regions[r].y0 + INPUT_OUTSET;
    int w = regions[r].x1 - regions[r].x0 - 2 * INPUT_OUTSET;
    int h = regions[r].y1 - regions[r].y0 - 2 * INPUT_OUTSET;
    if (w <= 0 || h <= 0) return;
    if (x != wins[i].gx || y != wins[i].gy || w != wins[i].gw || h != wins[i].gh) {
        wins[i].gx = x;
        wins[i].gy = y;
        wins[i].gw = w;
        wins[i].gh = h;
        marshal(wins[i].xdg_surface, XDG_SURFACE_SET_WINDOW_GEOMETRY, x, y, w, h);
        wins[i].fixed = -1; // hay que volver a poner el tamaño
    }
    // Si la app ocupa toda la pantalla de Android (un juego, o el botón □), la ventana
    // se maximiza, como hacía Waydroid con todas: así la pantalla de Android coincide
    // con la ventana (los controles de juegos dependen de eso). Si no, mide lo que
    // la app. Waydroid no maximiza ni restaura por su cuenta.
    int full = fills_display(i);
    if (wins[i].toplevel && full != wins[i].full) {
        wins[i].full = full;
        if (full) {
            set_fixed(i, 0);
            if (!wins[i].maximized && !wins[i].fullscreen) marshal(wins[i].toplevel, XDG_TOPLEVEL_SET_MAXIMIZED);
        } else if (wins[i].maximized) {
            marshal(wins[i].toplevel, XDG_TOPLEVEL_UNSET_MAXIMIZED);
        }
    }
    set_fixed(i, !full && !wins[i].maximized && !wins[i].fullscreen);
    ctl_window_updated(i);
}

// Todas las peticiones de Waydroid pasan por aquí. Sus argumentos son enteros o
// punteros (ninguno de coma flotante), así que se reenvían como 8 palabras: la
// original lee solo los que su firma indica.
__attribute__((visibility("default")))
void wl_proxy_marshal(struct wl_proxy *p, unsigned opcode, ...) {
    wl_init((uintptr_t)__builtin_return_address(0));
    __builtin_va_list ap;
    __builtin_va_start(ap, opcode);
    uintptr_t a[8];
    for (int k = 0; k < 8; k++) a[k] = __builtin_va_arg(ap, uintptr_t);
    __builtin_va_end(ap);

    const char *cls = get_class && get_user_data ? get_class(p) : "";
    size_t n = slen(cls);
    if (seq(cls, n, "wl_region")) {
        int r = slot_of_region(p, opcode == WL_REGION_ADD);
        if (r >= 0 && opcode == WL_REGION_ADD) {
            int x = (int)a[0], y = (int)a[1], x1 = x + (int)a[2], y1 = y + (int)a[3];
            if (!regions[r].valid) {
                regions[r].x0 = x; regions[r].y0 = y; regions[r].x1 = x1; regions[r].y1 = y1;
                regions[r].valid = 1;
            } else {
                if (x < regions[r].x0) regions[r].x0 = x;
                if (y < regions[r].y0) regions[r].y0 = y;
                if (x1 > regions[r].x1) regions[r].x1 = x1;
                if (y1 > regions[r].y1) regions[r].y1 = y1;
            }
        } else if (r >= 0 && opcode == WL_REGION_SUBTRACT) {
            regions[r].valid = 0; // Waydroid la vacía entera en cada cuadro
        } else if (r >= 0 && opcode == WL_REGION_DESTROY) {
            regions[r].region = 0;
        }
    } else if (seq(cls, n, "wl_surface") && opcode == WL_SURFACE_SET_INPUT_REGION && a[0]) {
        marshal(p, opcode, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
        update_geometry(p, (struct wl_proxy *)a[0]);
        return;
    } else if (seq(cls, n, "wl_pointer") && opcode == 0 /* set_cursor */ && pointer_ctl) {
        return; // sobre la capa manda el cursor propio (ctl_cursor)
    } else if (seq(cls, n, "xdg_toplevel") && opcode == XDG_TOPLEVEL_DESTROY) {
        // ventana cerrada: no se debe volver a usar
        int i = slot_of_toplevel(p);
        if (i >= 0) {
            ctl_window_gone(wins[i].window);
            wins[i].window = wins[i].toplevel = 0;
        }
    } else if (seq(cls, n, "xdg_toplevel") && opcode == XDG_TOPLEVEL_SET_APP_ID && a[0]) {
        int i = slot_of_toplevel(p);
        const char *id = (const char *)a[0];
        if (i >= 0) {
            size_t k = 0;
            for (; id[k] && k + 1 < sizeof wins[i].app_id; k++) wins[i].app_id[k] = id[k];
            wins[i].app_id[k] = 0;
        }
    } else if (seq(cls, n, "xdg_surface") && opcode == XDG_SURFACE_DESTROY) {
        int i = slot_of_xdg_surface(p);
        if (i >= 0) wins[i].window = wins[i].xdg_surface = wins[i].toplevel = 0;
    } else if (seq(cls, n, "xdg_surface") && opcode == XDG_SURFACE_SET_WINDOW_GEOMETRY) {
        // Waydroid la pone del tamaño de toda la pantalla: se mantiene la de la app
        int i = slot_of_xdg_surface(p);
        if (i >= 0 && wins[i].gw) {
            a[0] = wins[i].gx; a[1] = wins[i].gy; a[2] = wins[i].gw; a[3] = wins[i].gh;
        }
    } else if (seq(cls, n, "xdg_toplevel")
               && (opcode == XDG_TOPLEVEL_SET_MAXIMIZED || opcode == XDG_TOPLEVEL_SET_FULLSCREEN)) {
        int i = slot_of_toplevel(p);
        // Waydroid maximiza cada ventana nueva: sin eso mide lo que mide su app. La
        // primera de todas sí, porque con ella mide la pantalla del escritorio.
        if (opcode == XDG_TOPLEVEL_SET_MAXIMIZED && i >= 0 && !wins[i].gw) {
            if (measured) return;
            measured = 1;
        }
        // con tamaño fijo el escritorio no la dejaría crecer
        if (i >= 0) set_fixed(i, 0);
    }
    marshal(p, opcode, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
}

// ---------------------------------------------------------------------------------
// Portapapeles: el hwcomposer registra IWaydroidClipboard (texto con el portapapeles
// de Wayland, solo con una ventana de Android enfocada). Mientras existe, Android
// responde a toda app que lee el portapapeles con el texto de Linux, y lo que se
// copia como imagen (en Android o desde Linux) nunca se puede pegar. Playnix comparte
// texto e imágenes con su propio servicio (`playnix clipboard`), así que aquí se
// omite el registro. Se responde OK: si fallara, el hwcomposer dejaría de atender
// también la pantalla y las ventanas.
//
// El hwcomposer importa esta función de vendor.waydroid.clipboard@1.0.so, que en su
// lista de dependencias va después de libpnxh.so: el linker usa esta.

#define CLIP_REGISTER_SYM "_ZN6vendor8waydroid9clipboard4V1_018IWaydroidClipboard17registerAsServiceERKNSt3__112basic_stringIcNS4_11char_traitsIcEENS4_9allocatorIcEEEE"

__attribute__((visibility("default")))
int clipboard_register(void *self, const lstr *name) __asm__(CLIP_REGISTER_SYM);

int clipboard_register(void *self, const lstr *name) {
    (void)self;
    (void)name;
    __android_log_print(4, TAG, "portapapeles de Waydroid omitido: lo comparte Playnix");
    return 0;
}

// ---------------------------------------------------------------------------------
// Controles de juegos dentro de la ventana de la app.
//
// Playnix (en Linux) convierte teclado y mouse en toques. Antes lo hacía con su
// propia ventana transparente encima del juego: salía aparte en Alt+Tab y se podía
// mover sola. Ahora los controles viven dentro de la ventana de la app: una
// subsuperficie encima de ella muestra la imagen que dibuja Playnix, y el mouse sobre
// la app y las teclas que Playnix pide se le mandan a él en vez de a Android.
//
// Comunicación: socket Unix en /data/playnix/controls.sock. /data es la carpeta de
// datos de Android que Waydroid monta desde Linux (~/.local/share/waydroid/data), así
// que Playnix lo ve. Mensajes [tipo u32][largo u32][datos], enteros en el orden de
// la máquina:
//   Playnix → aquí: 1 HELLO app_id · 2 FRAME ancho alto píxeles (ARGB8888) ·
//                   3 KEYS todas n teclas… (códigos evdev) · 4 LOCK sí/no
//   aquí → Playnix: 1 SIZE ancho alto · 2 MOTION x y · 3 BUTTON botón estado ·
//                   4 AXIS eje valor · 5 KEY tecla estado · 6 MODS dep lat lock grupo ·
//                   7 REL dx dy · 8 LEAVE · 9 ENTER x y · 10 FOCUS sí/no · 11 GONE
//   (x, y, valor, dx y dy en punto fijo 24.8 de Wayland; posiciones en la capa)
//
// Los objetos de Wayland propios (superficie, memoria compartida, bloqueo del mouse)
// se crean con interfaces definidas aquí: el hwcomposer trae libwayland compilada
// adentro y no exporta las suyas; el compositor solo ve los nombres.

#define SOCK_DIR "/data/playnix"
#define SOCK_PATH SOCK_DIR "/controls.sock"
#define MSG_HELLO 1
#define MSG_FRAME 2
#define MSG_KEYS 3
#define MSG_LOCK 4
#define MAX_FRAME (64 << 20)

// bionic
struct sockaddr_un { unsigned short family; char path[108]; };
struct pollfd { int fd; short events, revents; };
extern int socket(int, int, int);
extern int bind(int, const void *, unsigned);
extern int listen(int, int);
extern int accept4(int, void *, unsigned *, int);
extern ssize_t recv(int, void *, size_t, int);
extern ssize_t send(int, const void *, size_t, int);
extern int poll(struct pollfd *, unsigned long, int);
extern int unlink(const char *);
extern int mkdir(const char *, unsigned);
extern int memfd_create(const char *, unsigned);
extern int ftruncate(int, long);
extern void *mmap(void *, size_t, int, int, int, long);
extern int munmap(void *, size_t);
extern int pthread_create(unsigned long *, const void *, void *(*)(void *), void *);
extern int pthread_mutexattr_init(long *);
extern int pthread_mutexattr_settype(long *, int);
extern int pthread_mutex_init(void *, const long *);
extern int pthread_mutex_lock(void *);
extern int pthread_mutex_unlock(void *);
#define AF_UNIX 1
#define SOCK_STREAM 1
#define SOCK_CLOEXEC 0x80000
#define MSG_DONTWAIT 0x40
#define MSG_WAITALL 0x100
#define MSG_NOSIGNAL 0x4000
#define POLLIN 1
#define MFD_CLOEXEC 1
#define PROT_RW 3
#define MAP_SHARED 1
#define PTHREAD_MUTEX_RECURSIVE 1

// --- Interfaces de Wayland (las del protocolo, con los mismos nombres) ---
struct wl_message { const char *name; const char *signature; const struct wl_interface **types; };
struct wl_interface { const char *name; int version; int method_count; const struct wl_message *methods;
                      int event_count; const struct wl_message *events; };

static const struct wl_interface i_surface, i_region, i_buffer, i_callback, i_output, i_pointer,
    i_subsurface, i_shm_pool, i_locked, i_confined, i_relative;
static const struct wl_interface *t_none[8];
static const struct wl_interface *t_surface[] = { &i_surface };
static const struct wl_interface *t_region[] = { &i_region };
static const struct wl_interface *t_buffer[] = { &i_buffer, 0, 0 };
static const struct wl_interface *t_callback[] = { &i_callback };
static const struct wl_interface *t_output[] = { &i_output };
static const struct wl_interface *t_subsurface[] = { &i_subsurface, &i_surface, &i_surface };
static const struct wl_interface *t_pool[] = { &i_shm_pool, 0, 0 };
static const struct wl_interface *t_new_buffer[] = { &i_buffer, 0, 0, 0, 0, 0 };
static const struct wl_interface *t_lock[] = { &i_locked, &i_surface, &i_pointer, &i_region, 0 };
static const struct wl_interface *t_confine[] = { &i_confined, &i_surface, &i_pointer, &i_region, 0 };
static const struct wl_interface *t_relative[] = { &i_relative, &i_pointer };

static const struct wl_message m_compositor[] = {
    { "create_surface", "n", t_surface }, { "create_region", "n", t_region } };
static const struct wl_interface i_compositor = { "wl_compositor", 4, 2, m_compositor, 0, 0 };
static const struct wl_message m_surface[] = {
    { "destroy", "", t_none }, { "attach", "?oii", t_buffer }, { "damage", "iiii", t_none },
    { "frame", "n", t_callback }, { "set_opaque_region", "?o", t_region },
    { "set_input_region", "?o", t_region }, { "commit", "", t_none },
    { "set_buffer_transform", "2i", t_none }, { "set_buffer_scale", "3i", t_none },
    { "damage_buffer", "4iiii", t_none } };
static const struct wl_message e_surface[] = { { "enter", "o", t_output }, { "leave", "o", t_output } };
static const struct wl_interface i_surface = { "wl_surface", 4, 10, m_surface, 2, e_surface };
static const struct wl_message m_region[] = {
    { "destroy", "", t_none }, { "add", "iiii", t_none }, { "subtract", "iiii", t_none } };
static const struct wl_interface i_region = { "wl_region", 1, 3, m_region, 0, 0 };
static const struct wl_message e_callback[] = { { "done", "u", t_none } };
static const struct wl_interface i_callback = { "wl_callback", 1, 0, 0, 1, e_callback };
static const struct wl_interface i_output = { "wl_output", 1, 0, 0, 0, 0 };
static const struct wl_interface i_pointer = { "wl_pointer", 1, 0, 0, 0, 0 };
static const struct wl_message m_subcompositor[] = {
    { "destroy", "", t_none }, { "get_subsurface", "noo", t_subsurface } };
static const struct wl_interface i_subcompositor = { "wl_subcompositor", 1, 2, m_subcompositor, 0, 0 };
static const struct wl_message m_subsurface[] = {
    { "destroy", "", t_none }, { "set_position", "ii", t_none }, { "place_above", "o", t_surface },
    { "place_below", "o", t_surface }, { "set_sync", "", t_none }, { "set_desync", "", t_none } };
static const struct wl_interface i_subsurface = { "wl_subsurface", 1, 6, m_subsurface, 0, 0 };
static const struct wl_message m_shm[] = { { "create_pool", "nhi", t_pool } };
static const struct wl_message e_shm[] = { { "format", "u", t_none } };
static const struct wl_interface i_shm = { "wl_shm", 1, 1, m_shm, 1, e_shm };
static const struct wl_message m_pool[] = {
    { "create_buffer", "niiiiu", t_new_buffer }, { "destroy", "", t_none }, { "resize", "i", t_none } };
static const struct wl_interface i_shm_pool = { "wl_shm_pool", 1, 3, m_pool, 0, 0 };
static const struct wl_message m_buffer[] = { { "destroy", "", t_none } };
static const struct wl_message e_buffer[] = { { "release", "", t_none } };
static const struct wl_interface i_buffer = { "wl_buffer", 1, 1, m_buffer, 1, e_buffer };
static const struct wl_message m_constraints[] = {
    { "destroy", "", t_none }, { "lock_pointer", "noo?ou", t_lock }, { "confine_pointer", "noo?ou", t_confine } };
static const struct wl_interface i_constraints = { "zwp_pointer_constraints_v1", 1, 3, m_constraints, 0, 0 };
static const struct wl_message m_locked[] = {
    { "destroy", "", t_none }, { "set_cursor_position_hint", "ff", t_none }, { "set_region", "?o", t_region } };
static const struct wl_message e_locked[] = { { "locked", "", t_none }, { "unlocked", "", t_none } };
static const struct wl_interface i_locked = { "zwp_locked_pointer_v1", 1, 3, m_locked, 2, e_locked };
static const struct wl_interface i_confined = { "zwp_confined_pointer_v1", 1, 0, 0, 0, 0 };
static const struct wl_message m_relmgr[] = {
    { "destroy", "", t_none }, { "get_relative_pointer", "no", t_relative } };
static const struct wl_interface i_relmgr = { "zwp_relative_pointer_manager_v1", 1, 2, m_relmgr, 0, 0 };
static const struct wl_message m_relative[] = { { "destroy", "", t_none } };
static const struct wl_message e_relative[] = { { "relative_motion", "uuffff", t_none } };
static const struct wl_interface i_relative = { "zwp_relative_pointer_v1", 1, 1, m_relative, 1, e_relative };

typedef struct wl_proxy *(*constructor_fn)(struct wl_proxy *, unsigned, const struct wl_interface *, ...);
typedef struct wl_proxy *(*constructor_ver_fn)(struct wl_proxy *, unsigned, const struct wl_interface *, unsigned, ...);
static constructor_fn construct;
static constructor_ver_fn construct_ver;
static void (*proxy_destroy)(struct wl_proxy *);
static unsigned (*proxy_version)(struct wl_proxy *);
static int (*display_flush)(void *);

static struct wl_proxy *compositor, *subcompositor, *shm, *constraints, *relmgr;
static void *wl_display_ptr;
// Recursivo: los ganchos pueden volver a entrar desde el mismo hilo
static long big_lock[8];

static void lock(void) { pthread_mutex_lock(big_lock); }
static void unlock(void) { pthread_mutex_unlock(big_lock); }

// La wl_display está en la propia wl_proxy (después de su wl_object)
static void *display_of(struct wl_proxy *p) { return *(void **)((char *)p + 24); }
static void flush(void) { if (wl_display_ptr) display_flush(wl_display_ptr); }

static void destroy(struct wl_proxy **p, unsigned destroy_opcode) {
    if (!*p) return;
    marshal(*p, destroy_opcode);
    proxy_destroy(*p);
    *p = 0;
}

// --- Clientes (una capa por app) ---
#define CLIENTS 8
static struct ctl {
    int fd;                         // -1: libre
    char app_id[160];               // "waydroid.<paquete>"
    void *window;                   // ventana con la capa (0: todavía no aparece)
    struct wl_proxy *surface, *subsurface, *pool, *buffer[2], *locked, *relative;
    int mapfd, bw, bh, next;
    unsigned char *map;
    size_t map_size;
    int w, h;                       // tamaño de la capa enviado a Playnix
    int all_keys;
    unsigned char keys[96];         // teclas pedidas (bits, códigos evdev < 768)
} ctls[CLIENTS];
static unsigned char key_to_ctl[768]; // 0: Android; n: ctls[n-1] (dónde fue cada tecla)

static void send_ev(struct ctl *c, unsigned type, const int *v, unsigned n) {
    unsigned buf[2 + 4];
    if (c->fd < 0 || n > 4) return;
    buf[0] = type;
    buf[1] = n * 4;
    for (unsigned k = 0; k < n; k++) buf[2 + k] = (unsigned)v[k];
    send(c->fd, buf, (2 + n) * 4, MSG_DONTWAIT | MSG_NOSIGNAL);
}

static struct ctl *ctl_of_window(void *window) {
    if (!window) return 0;
    for (int k = 0; k < CLIENTS; k++) if (ctls[k].fd >= 0 && ctls[k].window == window) return &ctls[k];
    return 0;
}

static void ctl_unlock_pointer(struct ctl *c) {
    destroy(&c->relative, 0);
    destroy(&c->locked, 0);
}

// Quita la capa de la ventana (la ventana se cerró o Playnix se fue)
static void ctl_detach(struct ctl *c) {
    ctl_unlock_pointer(c);
    destroy(&c->subsurface, 0);
    destroy(&c->surface, 0);
    if (pointer_ctl == c) pointer_ctl = 0;
    c->window = 0;
    c->w = c->h = 0;
}

static void ctl_free_buffers(struct ctl *c) {
    destroy(&c->buffer[0], 0);
    destroy(&c->buffer[1], 0);
    destroy(&c->pool, 1);
    if (c->map) munmap(c->map, c->map_size);
    if (c->mapfd >= 0) close(c->mapfd);
    c->map = 0;
    c->mapfd = -1;
    c->bw = c->bh = 0;
}

static void ctl_close(struct ctl *c) {
    ctl_detach(c);
    ctl_free_buffers(c);
    for (int k = 0; k < 768; k++) if (key_to_ctl[k] == c - ctls + 1) key_to_ctl[k] = 0;
    close(c->fd);
    c->fd = -1;
    c->app_id[0] = 0;
    flush();
}

// Pone la capa en la ventana `i` (llamado con el lock tomado)
static void ctl_attach(struct ctl *c, int i) {
    if (!compositor || !subcompositor || !wins[i].surface) return;
    c->surface = construct(compositor, 0, &i_surface, (void *)0);
    c->subsurface = construct(subcompositor, 1, &i_subsurface, (void *)0, c->surface, wins[i].surface);
    marshal(c->subsurface, 5); // desync: se actualiza sin esperar al juego
    marshal(c->subsurface, 1, wins[i].gx, wins[i].gy);
    c->window = wins[i].window;
    c->w = wins[i].gw;
    c->h = wins[i].gh;
    int v[2] = { c->w, c->h };
    send_ev(c, EV_SIZE, v, 2);
    flush();
}

// Cada cuadro, después de conocer la geometría de la ventana `i`
static void ctl_window_updated(int i) {
    lock();
    struct ctl *c = ctl_of_window(wins[i].window);
    if (c) {
        if (c->w != wins[i].gw || c->h != wins[i].gh) {
            c->w = wins[i].gw;
            c->h = wins[i].gh;
            int v[2] = { c->w, c->h };
            send_ev(c, EV_SIZE, v, 2);
        }
        marshal(c->subsurface, 1, wins[i].gx, wins[i].gy);
    } else if (wins[i].app_id[0]) {
        size_t n = slen(wins[i].app_id);
        for (int k = 0; k < CLIENTS; k++)
            if (ctls[k].fd >= 0 && !ctls[k].window && seq(ctls[k].app_id, slen(ctls[k].app_id), wins[i].app_id)
                && slen(ctls[k].app_id) == n)
                ctl_attach(&ctls[k], i);
    }
    unlock();
}

static void ctl_window_gone(void *window) {
    lock();
    struct ctl *c = ctl_of_window(window);
    if (c) {
        ctl_detach(c);
        send_ev(c, EV_GONE, 0, 0);
    }
    unlock();
}

static int recv_all(int fd, void *buf, size_t n) {
    size_t got = 0;
    while (got < n) {
        ssize_t r = recv(fd, (char *)buf + got, n - got, MSG_WAITALL);
        if (r <= 0) return 0;
        got += r;
    }
    return 1;
}

static int skip(int fd, size_t n) {
    char tmp[4096];
    while (n) {
        size_t k = n < sizeof tmp ? n : sizeof tmp;
        if (!recv_all(fd, tmp, k)) return 0;
        n -= k;
    }
    return 1;
}

static int ctl_frame(struct ctl *c, unsigned len) {
    unsigned wh[2];
    if (len < 8 || !recv_all(c->fd, wh, 8)) return 0;
    size_t size = (size_t)wh[0] * wh[1] * 4;
    if (!wh[0] || !wh[1] || size + 8 != len || size > MAX_FRAME) return skip(c->fd, len - 8);
    lock();
    if ((int)wh[0] != c->bw || (int)wh[1] != c->bh) {
        ctl_free_buffers(c);
        c->mapfd = memfd_create("playnix-controles", MFD_CLOEXEC);
        if (c->mapfd < 0 || ftruncate(c->mapfd, size * 2) < 0) { unlock(); return skip(c->fd, size); }
        c->map = mmap(0, size * 2, PROT_RW, MAP_SHARED, c->mapfd, 0);
        if (c->map == (void *)-1) { c->map = 0; unlock(); return skip(c->fd, size); }
        c->map_size = size * 2;
        c->pool = construct(shm, 0, &i_shm_pool, (void *)0, c->mapfd, (int)(size * 2));
        for (int k = 0; k < 2; k++)
            c->buffer[k] = construct(c->pool, 0, &i_buffer, (void *)0, (int)(k * size), (int)wh[0], (int)wh[1],
                                     (int)wh[0] * 4, 0 /* ARGB8888 */);
        c->bw = wh[0];
        c->bh = wh[1];
    }
    unsigned char *dst = c->map + c->next * size;
    unlock();
    if (!recv_all(c->fd, dst, size)) return 0;
    lock();
    if (c->surface && c->buffer[c->next]) {
        marshal(c->surface, 1, c->buffer[c->next], 0, 0);
        if (proxy_version(c->surface) >= 4) marshal(c->surface, 9, 0, 0, c->bw, c->bh);
        else marshal(c->surface, 2, 0, 0, c->bw, c->bh);
        marshal(c->surface, 6);
        flush();
    }
    c->next ^= 1;
    unlock();
    return 1;
}

static void relative_motion(void *data, struct wl_proxy *rp, unsigned hi, unsigned lo,
                            int dx, int dy, int dxu, int dyu) {
    (void)rp; (void)hi; (void)lo; (void)dx; (void)dy;
    struct ctl *c = data;
    int v[2] = { dxu, dyu };
    lock();
    send_ev(c, EV_REL, v, 2);
    unlock();
}
static const fnptr relative_listener[] = { (fnptr)relative_motion };

static void ctl_lock_pointer(struct ctl *c, int on) {
    if (!on) {
        ctl_unlock_pointer(c);
        if (pointer_ctl == c) ctl_cursor(cursor_serial);
        return;
    }
    if (c->locked || !c->surface || !constraints || !relmgr || !pointer_proxy) return;
    c->locked = construct(constraints, 1, &i_locked, (void *)0, c->surface, pointer_proxy, (void *)0, 2 /* persistente */);
    c->relative = construct(relmgr, 1, &i_relative, (void *)0, pointer_proxy);
    real_add_listener(c->relative, (fnptr *)relative_listener, c);
    flush();
    if (pointer_ctl == c) ctl_cursor(cursor_serial);
}

// Lee un mensaje de Playnix; 0 si se cortó
static int ctl_read(struct ctl *c) {
    unsigned head[2];
    if (!recv_all(c->fd, head, 8)) return 0;
    unsigned type = head[0], len = head[1];
    if (type == MSG_FRAME) return ctl_frame(c, len);
    if (len > 4096) return 0;
    unsigned char buf[4096];
    if (len && !recv_all(c->fd, buf, len)) return 0;
    lock();
    if (type == MSG_HELLO && len < sizeof c->app_id) {
        for (unsigned k = 0; k < len; k++) c->app_id[k] = buf[k];
        c->app_id[len] = 0;
        for (int i = 0; i < WINDOWS; i++)
            if (wins[i].window && wins[i].gw && !ctl_of_window(wins[i].window)
                && seq(wins[i].app_id, slen(wins[i].app_id), c->app_id) && slen(c->app_id) == slen(wins[i].app_id)) {
                ctl_attach(c, i);
                break;
            }
    } else if (type == MSG_KEYS && len >= 8) {
        unsigned *v = (unsigned *)buf;
        c->all_keys = v[0] != 0;
        for (int k = 0; k < 96; k++) c->keys[k] = 0;
        for (unsigned k = 0; k < v[1] && 8 + k * 4 + 4 <= len; k++)
            if (v[2 + k] < 768) c->keys[v[2 + k] / 8] |= 1 << (v[2 + k] % 8);
    } else if (type == MSG_LOCK && len >= 4) {
        ctl_lock_pointer(c, ((unsigned *)buf)[0] != 0);
    }
    unlock();
    return 1;
}

static void *ctl_thread(void *arg) {
    (void)arg;
    mkdir(SOCK_DIR, 0700);
    unlink(SOCK_PATH);
    int srv = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    struct sockaddr_un addr = { AF_UNIX, SOCK_PATH };
    if (srv < 0 || bind(srv, &addr, sizeof addr) < 0 || listen(srv, 4) < 0) {
        __android_log_print(5, TAG, "sin socket de controles: los controles irán en otra ventana");
        return 0;
    }
    for (;;) {
        struct pollfd fds[1 + CLIENTS];
        int n = 0;
        fds[n++] = (struct pollfd){ srv, POLLIN, 0 };
        for (int k = 0; k < CLIENTS; k++) if (ctls[k].fd >= 0) fds[n++] = (struct pollfd){ ctls[k].fd, POLLIN, 0 };
        if (poll(fds, n, -1) < 0) continue;
        for (int j = 1; j < n; j++) {
            if (!fds[j].revents) continue;
            for (int k = 0; k < CLIENTS; k++) {
                if (ctls[k].fd != fds[j].fd) continue;
                if (!ctl_read(&ctls[k])) {
                    lock();
                    ctl_close(&ctls[k]);
                    unlock();
                }
                break;
            }
        }
        if (fds[0].revents) {
            int fd = accept4(srv, 0, 0, SOCK_CLOEXEC);
            if (fd < 0) continue;
            lock();
            int k = 0;
            while (k < CLIENTS && ctls[k].fd >= 0) k++;
            if (k == CLIENTS) {
                close(fd);
            } else {
                struct ctl *c = &ctls[k];
                for (unsigned b = 0; b < sizeof *c; b++) ((char *)c)[b] = 0;
                c->fd = fd;
                c->mapfd = -1;
            }
            unlock();
        }
    }
    return 0;
}

static void ctl_start(uintptr_t caller) {
    construct = (constructor_fn)resolve(caller, "wl_proxy_marshal_constructor");
    construct_ver = (constructor_ver_fn)resolve(caller, "wl_proxy_marshal_constructor_versioned");
    proxy_destroy = (void (*)(struct wl_proxy *))resolve(caller, "wl_proxy_destroy");
    proxy_version = (unsigned (*)(struct wl_proxy *))resolve(caller, "wl_proxy_get_version");
    display_flush = (int (*)(void *))resolve(caller, "wl_display_flush");
    if (!construct || !construct_ver || !proxy_destroy || !proxy_version || !display_flush) return;
    long attr;
    pthread_mutexattr_init(&attr);
    pthread_mutexattr_settype(&attr, PTHREAD_MUTEX_RECURSIVE);
    pthread_mutex_init(big_lock, &attr);
    for (int k = 0; k < CLIENTS; k++) ctls[k].fd = -1;
    unsigned long tid;
    pthread_create(&tid, 0, ctl_thread, 0);
}

// --- wl_registry: global(data, registry, nombre, interfaz, versión) ---

static void registry_global(void *data, struct wl_proxy *r, unsigned name, const char *iface, unsigned version) {
    ((void (*)(void *, struct wl_proxy *, unsigned, const char *, unsigned))registry_orig[0])(data, r, name, iface, version);
    if (!construct_ver) return;
    size_t n = slen(iface);
    const struct wl_interface *want = 0;
    struct wl_proxy **slot = 0;
    unsigned ver = 1;
    if (seq(iface, n, "wl_compositor")) { want = &i_compositor; slot = &compositor; ver = version < 4 ? version : 4; }
    else if (seq(iface, n, "wl_subcompositor")) { want = &i_subcompositor; slot = &subcompositor; }
    else if (seq(iface, n, "wl_shm")) { want = &i_shm; slot = &shm; }
    else if (seq(iface, n, "zwp_pointer_constraints_v1")) { want = &i_constraints; slot = &constraints; }
    else if (seq(iface, n, "zwp_relative_pointer_manager_v1")) { want = &i_relmgr; slot = &relmgr; }
    if (!want || *slot) return;
    wl_display_ptr = display_of(r);
    *slot = construct_ver(r, 0, want, ver, name, want->name, ver, (void *)0);
}

// --- Entrada para Playnix (desde los listeners del puntero y el teclado) ---
static struct ctl *ctl_of_surface(struct wl_proxy *surface) {
    struct ctl *found = 0;
    if (!surface || !construct) return 0;
    lock();
    for (int k = 0; k < CLIENTS; k++) if (ctls[k].fd >= 0 && ctls[k].surface == surface) found = &ctls[k];
    unlock();
    return found;
}

static void ctl_pointer(struct ctl *c, unsigned type, const int *v, unsigned n) {
    lock();
    send_ev(c, type, v, n);
    unlock();
}

// ¿La tecla es para Playnix? Cada tecla suelta va adonde fue al presionarla
static int ctl_key(unsigned key, unsigned state) {
    if (key >= 768 || !construct) return 0;
    int mine = 0;
    lock();
    if (state) {
        struct ctl *c = ctl_of_window(focused);
        key_to_ctl[key] = 0;
        if (c && (c->all_keys || (c->keys[key / 8] >> (key % 8) & 1))) {
            key_to_ctl[key] = c - ctls + 1;
            mine = 1;
        }
    } else if (key_to_ctl[key]) {
        mine = 1;
    }
    if (mine) {
        int v[2] = { (int)key, (int)state };
        send_ev(&ctls[key_to_ctl[key] - 1], EV_KEY, v, 2);
        if (!state) key_to_ctl[key] = 0;
    }
    unlock();
    return mine;
}

static void ctl_mods(const int *v) {
    if (!construct) return;
    lock();
    struct ctl *c = ctl_of_window(focused);
    if (c) send_ev(c, EV_MODS, v, 4);
    unlock();
}

static void ctl_focus(void *window, int on) {
    if (!construct) return;
    lock();
    struct ctl *c = ctl_of_window(window);
    if (c) send_ev(c, EV_FOCUS, &on, 1);
    unlock();
}

// --- Cursor sobre la capa ---
// Flecha clásica: X borde negro, . relleno blanco
static const char *const arrow[] = {
    "X           ", "XX          ", "X.X         ", "X..X        ", "X...X       ",
    "X....X      ", "X.....X     ", "X......X    ", "X.......X   ", "X........X  ",
    "X.........X ", "X......XXXXX", "X...X..X    ", "X..XX..X    ", "X.X  X..X   ",
    "XX   X..X   ", "X     X..X  ", "      X..X  ", "       XX   ",
};
#define ARROW_W 12
#define ARROW_H 19
static struct wl_proxy *cursor_surface;

static void cursor_create(void) {
    int scale = 2; // nítida también con escalado 2
    int w = ARROW_W * scale, h = ARROW_H * scale;
    size_t size = (size_t)w * h * 4;
    int fd = memfd_create("playnix-cursor", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, size) < 0) { if (fd >= 0) close(fd); return; }
    unsigned *px = mmap(0, size, PROT_RW, MAP_SHARED, fd, 0);
    if (px == (void *)-1) { close(fd); return; }
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++) {
            char c = arrow[y / scale][x / scale];
            px[y * w + x] = c == 'X' ? 0xFF000000u : c == '.' ? 0xFFFFFFFFu : 0;
        }
    munmap(px, size);
    struct wl_proxy *pool = construct(shm, 0, &i_shm_pool, (void *)0, fd, (int)size);
    struct wl_proxy *buf = construct(pool, 0, &i_buffer, (void *)0, 0, w, h, w * 4, 0 /* ARGB8888 */);
    destroy(&pool, 1);
    close(fd);
    cursor_surface = construct(compositor, 0, &i_surface, (void *)0);
    marshal(cursor_surface, 1, buf, 0, 0);
    marshal(cursor_surface, 8, scale);           // set_buffer_scale
    marshal(cursor_surface, 2, 0, 0, w, h);
    marshal(cursor_surface, 6);
}

// Pone la flecha (u oculta el cursor si Playnix fijó el mouse para apuntar)
static void ctl_cursor(unsigned serial) {
    if (!construct || !pointer_proxy || !compositor || !shm) return;
    lock();
    if (!cursor_surface) cursor_create();
    struct ctl *c = pointer_ctl;
    int hidden = c && c->locked;
    if (cursor_surface)
        marshal(pointer_proxy, 0, serial, hidden ? 0 : cursor_surface, 0, 0);
    flush();
    unlock();
}
