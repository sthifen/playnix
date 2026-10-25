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

// La definición original sigue en el hwcomposer (con enlace local): se busca en
// su .dynsym en disco y se suma a la base donde quedó cargado.
static split_fn resolve(uintptr_t caller) {
    struct find f = { caller, 0, 0 };
    if (!dl_iterate_phdr(find_lib, &f) || !f.name) return 0;
    int fd = open(f.name, 0x80000 /* O_RDONLY|O_CLOEXEC */);
    if (fd < 0) return 0;
    split_fn fn = 0;
    Ehdr eh;
    Shdr sh[128];
    if (pread(fd, &eh, sizeof eh, 0) != sizeof eh || eh.e_shnum > 128
        || pread(fd, sh, eh.e_shnum * sizeof(Shdr), eh.e_shoff) != (ssize_t)(eh.e_shnum * sizeof(Shdr)))
        goto out;
    for (int i = 0; i < eh.e_shnum && !fn; i++) {
        if (sh[i].sh_type != 11 /* SHT_DYNSYM */) continue;
        const Shdr *str = &sh[sh[i].sh_link];
        size_t want = slen(SPLIT_SYM) + 1;
        char name[160];
        for (unsigned long off = 0; off < sh[i].sh_size; off += sizeof(Sym)) {
            Sym s;
            if (pread(fd, &s, sizeof s, sh[i].sh_offset + off) != sizeof s) break;
            if (s.st_shndx == 0 || s.st_name >= str->sh_size) continue;
            if (pread(fd, name, want, str->sh_offset + s.st_name) != (ssize_t)want) continue;
            if (seq(name, want - 1, SPLIT_SYM)) { fn = (split_fn)(f.bias + s.st_value); break; }
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
        orig = resolve((uintptr_t)__builtin_return_address(0));
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
