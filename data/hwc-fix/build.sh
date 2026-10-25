#!/usr/bin/env bash
# Compila libpnxh.so para Android x86_64 (bionic) con clang + lld del sistema, sin NDK.
# Las bibliotecas de Android contra las que se enlaza son stubs vacíos con su soname:
# los símbolos reales los resuelve el linker de Android al cargarla.
# Resultado: data/hwc-fix/libpnxh.so (se versiona; Playnix la embebe).
set -euo pipefail

DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
T=(--target=x86_64-linux-android33 -nostdlib -fPIC -shared -fuse-ld=lld)

echo 'void __stub(void){}' > "$TMP/stub.c"
for l in c dl log sync c++; do
    clang "${T[@]}" -Wl,-soname,"lib$l.so" -o "$TMP/lib$l.so" "$TMP/stub.c"
done

clang "${T[@]}" -O2 -Wall -Wextra -Werror -fno-stack-protector -fvisibility=hidden \
    -Wl,-soname,libpnxh.so -Wl,--hash-style=both -Wl,-z,noexecstack -Wl,-z,now \
    -Wl,--build-id=none -Wl,--allow-shlib-undefined -Wl,-z,undefs \
    -L"$TMP" -Wl,--no-as-needed -lsync -lc++ -llog -ldl -lc \
    -o "$DIR/libpnxh.so" "$DIR/pnxhwc.c"
echo "Listo: $DIR/libpnxh.so"
