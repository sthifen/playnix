#!/usr/bin/env bash
# Copia al AppDir las bibliotecas que necesita el binario (GTK4, libadwaita y sus
# dependencias), salvo las que deben ser las del sistema: glibc, gráficos/Wayland,
# fuentes y gdk-pixbuf (sus cargadores de imágenes viven en el sistema). Se enlazan
# por RPATH, sin LD_LIBRARY_PATH, para que los programas del sistema que lanza
# Playnix (waydroid, python3, el navegador…) no carguen estas copias.
# Uso: bundle-libs.sh APPDIR   (dentro del contenedor de construcción)
set -euo pipefail

APPDIR=$1
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
BIN="$APPDIR/usr/bin/playnix"
LIB="$APPDIR/usr/lib"
mkdir -p "$LIB"

# Lista estándar de AppImage + las que deben coincidir con el sistema
excluded() {
    local n=$1
    grep -qxF "$n" <(sed 's/#.*//; s/[[:space:]]*$//' "$HERE/excludelist") && return 0
    case "$n" in
        libwayland-*|libvulkan.so*|libgdk_pixbuf-2.0.so*|libxkbcommon.so*|libdbus-1.so*|libsystemd.so*|libudev.so*) return 0 ;;
        libxcb*|libX*|libGL*|libEGL*|libgbm*|libdrm*) return 0 ;;
    esac
    return 1
}

copied=0 skipped=()
while read -r name path; do
    if excluded "$name"; then
        skipped+=("$name")
        continue
    fi
    install -m 644 "$path" "$LIB/$name"
    copied=$((copied + 1))
done < <(ldd "$BIN" | awk '$2 == "=>" && $3 ~ /^\// { print $1, $3 }' | sort -u)

patchelf --set-rpath '$ORIGIN/../lib' "$BIN"
for f in "$LIB"/*.so*; do
    patchelf --set-rpath '$ORIGIN' "$f"
done

# Comprobación: con el RPATH, todo lo incluido se resuelve dentro del AppDir
if ldd "$BIN" | grep -q 'not found'; then
    ldd "$BIN" | grep 'not found' >&2
    exit 1
fi
echo "bibliotecas incluidas: $copied; del sistema: ${#skipped[@]} (${skipped[*]})"
