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
    ((void (*)(void *, struct wl_proxy *, unsigned, struct wl_proxy *, struct wl_array *))keyboard_orig[1])(data, kb, serial, surface, keys);
}

static void keyboard_leave(void *data, struct wl_proxy *kb, unsigned serial, struct wl_proxy *surface) {
    focused = 0;
    ((void (*)(void *, struct wl_proxy *, unsigned, struct wl_proxy *))keyboard_orig[2])(data, kb, serial, surface);
}

// --- wl_pointer: enter · leave · motion · button ---
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
    ((void (*)(void *, struct wl_proxy *, unsigned, struct wl_proxy *))pointer_orig[1])(data, p, serial, surface);
}

static void pointer_motion(void *data, struct wl_proxy *p, unsigned time, int sx, int sy) {
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
}

__attribute__((visibility("default")))
int wl_proxy_add_listener(struct wl_proxy *proxy, fnptr *impl, void *data) {
    wl_init((uintptr_t)__builtin_return_address(0));
    if (!get_class || !get_user_data) return real_add_listener(proxy, impl, data);

    const char *cls = get_class(proxy);
    size_t n = slen(cls);
    if (seq(cls, n, "wl_seat")) {
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
    } else if (seq(cls, n, "xdg_toplevel")) {
        int i = -1;
        for (int k = 0; k < WINDOWS; k++) if (wins[k].window == data && !wins[k].toplevel) { i = k; break; }
        if (i < 0) {
            i = next_win++ % WINDOWS;
            wins[i].window = data;
            wins[i].xdg_surface = 0;
            wins[i].fixed = wins[i].gw = 0;
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
        impl = keyboard_wrap;
    } else if (seq(cls, n, "wl_pointer")) {
        wrap(pointer_orig, pointer_wrap, impl);
        pointer_wrap[0] = (fnptr)pointer_enter;
        pointer_wrap[1] = (fnptr)pointer_leave;
        pointer_wrap[2] = (fnptr)pointer_motion;
        pointer_wrap[3] = (fnptr)pointer_button;
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
    if (r < 0 || i < 0 || !regions[r].valid || !wins[i].xdg_surface) return;
    int x = regions[r].x0 + INPUT_OUTSET, y = regions[r].y0 + INPUT_OUTSET;
    int w = regions[r].x1 - regions[r].x0 - 2 * INPUT_OUTSET;
    int h = regions[r].y1 - regions[r].y0 - 2 * INPUT_OUTSET;
    if (w <= 0 || h <= 0) return;
    if (!wins[i].gw && wins[i].maximized && wins[i].toplevel) {
        // la primera ventana, con la que Waydroid mide la pantalla, llega maximizada
        wins[i].gw = w;
        wins[i].gh = h;
        if (!fills_display(i)) marshal(wins[i].toplevel, XDG_TOPLEVEL_UNSET_MAXIMIZED);
        wins[i].gw = 0;
    }
    if (x != wins[i].gx || y != wins[i].gy || w != wins[i].gw || h != wins[i].gh) {
        wins[i].gx = x;
        wins[i].gy = y;
        wins[i].gw = w;
        wins[i].gh = h;
        marshal(wins[i].xdg_surface, XDG_SURFACE_SET_WINDOW_GEOMETRY, x, y, w, h);
        wins[i].fixed = -1; // hay que volver a poner el tamaño
    }
    set_fixed(i, !wins[i].maximized && !wins[i].fullscreen);
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
    } else if (seq(cls, n, "xdg_toplevel") && opcode == XDG_TOPLEVEL_DESTROY) {
        // ventana cerrada: no se debe volver a usar
        int i = slot_of_toplevel(p);
        if (i >= 0) wins[i].window = wins[i].toplevel = 0;
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
