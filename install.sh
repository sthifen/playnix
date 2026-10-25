#!/usr/bin/env bash
# Instalador de una línea de Playnix:
#   curl -fsSL https://raw.githubusercontent.com/sthifen/playnix/main/install.sh | bash
# Descarga el AppImage más reciente, instala GTK4/libadwaita si faltan y lanza la
# instalación en modo texto. Pasa opciones así: ... | bash -s -- --yes --fast-start
set -euo pipefail

URL=https://github.com/sthifen/playnix/releases/latest/download/Playnix-x86_64.AppImage
BIN="$HOME/.local/bin/playnix"

[ "$(id -u)" -ne 0 ] || { echo "Ejecútalo como tu usuario normal (sin sudo)." >&2; exit 1; }
[ "$(uname -m)" = x86_64 ] || { echo "Playnix por ahora solo funciona en PCs x86_64." >&2; exit 1; }

echo "==> Comprobando bibliotecas de interfaz (GTK4 y libadwaita)"
if ! ldconfig -p 2>/dev/null | grep -q 'libadwaita-1.so.0' || ! ldconfig -p 2>/dev/null | grep -q 'libgtk-4.so.1'; then
    if command -v dnf >/dev/null; then sudo dnf install -y gtk4 libadwaita
    elif command -v apt-get >/dev/null; then sudo apt-get update && sudo apt-get install -y libgtk-4-1 libadwaita-1-0
    elif command -v pacman >/dev/null; then sudo pacman -S --needed --noconfirm gtk4 libadwaita
    elif command -v zypper >/dev/null; then sudo zypper -n install libgtk-4-1 libadwaita-1-0
    else echo "Instala gtk4 y libadwaita con el gestor de paquetes de tu distro y vuelve a ejecutar." >&2; exit 1
    fi
fi
# El AppImage necesita FUSE 2 para montarse; sin él se extrae y corre igual
command -v fusermount >/dev/null || export APPIMAGE_EXTRACT_AND_RUN=1

echo "==> Descargando Playnix"
mkdir -p "$(dirname "$BIN")"
curl -fL --progress-bar -o "$BIN.new" "$URL"
chmod +x "$BIN.new"
mv "$BIN.new" "$BIN"

exec "$BIN" --cli "$@" </dev/tty
