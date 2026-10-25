"""Prepara una copia de hwcomposer.waydroid.so que usa libpnxh.so.

Uso: python3 -I hwc-patch.py ORIGINAL SALIDA

Dos cambios de datos, sin tocar código:
1. split_layer_names_helper::split_layer_name pasa a enlace local, así la llamada por
   PLT se resuelve con la versión de libpnxh.so (que llama a la original).
2. El DT_NEEDED libsync.so se renombra a libpnxh.so (misma longitud); libpnxh.so a
   su vez depende de libsync.so, así que nada se pierde.

Sale con código 1 si el archivo no tiene la forma esperada (otra versión de Waydroid).
"""
import struct
import sys

SPLIT = b"_ZN24split_layer_names_helper16split_layer_nameERKNSt3__112basic_stringIcNS0_11char_traitsIcEENS0_9allocatorIcEEEE"
OLD, NEW = b"libsync.so", b"libpnxh.so"
SHT_DYNSYM, SHT_DYNAMIC, DT_NEEDED, STB_LOCAL = 11, 6, 1, 0


def fail(msg):
    print(f"hwc-patch: {msg}", file=sys.stderr)
    sys.exit(1)


def main(src, dst):
    b = bytearray(open(src, "rb").read())
    if b[:4] != b"\x7fELF" or b[4] != 2 or struct.unpack_from("<H", b, 0x12)[0] != 62:
        fail("no es un ELF x86_64")
    shoff, = struct.unpack_from("<Q", b, 0x28)
    shentsize, shnum = struct.unpack_from("<HH", b, 0x3A)
    secs = [struct.unpack_from("<IIQQQQIIQQ", b, shoff + i * shentsize) for i in range(shnum)]
    dynsym = next((s for s in secs if s[1] == SHT_DYNSYM), None)
    dynamic = next((s for s in secs if s[1] == SHT_DYNAMIC), None)
    if not dynsym or not dynamic:
        fail("faltan .dynsym o .dynamic")
    stroff = secs[dynsym[6]][4]

    def cstr(off):
        return bytes(b[stroff + off:b.index(0, stroff + off)])

    names, hits = [], []
    for off in range(dynsym[4], dynsym[4] + dynsym[5], 24):
        st_name, st_info = struct.unpack_from("<IB", b, off)
        st_shndx, = struct.unpack_from("<H", b, off + 6)
        names.append(st_name)
        if st_shndx != 0 and cstr(st_name) == SPLIT:
            hits.append((off, st_info))
    if len(hits) != 1:
        fail("no está split_layer_name")
    off, st_info = hits[0]
    b[off + 4] = (STB_LOCAL << 4) | (st_info & 0xF)

    needed = []
    for off in range(dynamic[4], dynamic[4] + dynamic[5], 16):
        tag, val = struct.unpack_from("<qQ", b, off)
        if tag == DT_NEEDED and cstr(val) == OLD:
            needed.append(val)
    if len(needed) != 1:
        fail("no depende de libsync.so")
    start = needed[0]
    if any(start <= n < start + len(OLD) for n in names):
        fail("la cadena libsync.so está compartida")
    b[stroff + start:stroff + start + len(OLD)] = NEW

    open(dst, "wb").write(b)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        fail("uso: hwc-patch.py ORIGINAL SALIDA")
    main(sys.argv[1], sys.argv[2])
