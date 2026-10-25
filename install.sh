#!/usr/bin/env bash
# Instalador de una línea de Playnix:
#   curl -fsSL https://raw.githubusercontent.com/sthifen/playnix/main/install.sh | bash
# Descarga el AppImage más reciente (trae su propia interfaz GTK4/libadwaita) y
# lanza la instalación en modo texto. Pasa opciones así: ... | bash -s -- --yes --fast-start
set -euo pipefail

URL=https://github.com/sthifen/playnix/releases/latest/download/Playnix-x86_64.AppImage
BIN="$HOME/.local/bin/playnix"

[ "$(id -u)" -ne 0 ] || { echo "Ejecútalo como tu usuario normal (sin sudo)." >&2; exit 1; }
[ "$(uname -m)" = x86_64 ] || { echo "Playnix por ahora solo funciona en PCs x86_64." >&2; exit 1; }

# Sin FUSE el AppImage se extrae y corre igual (al instalarse, Playnix queda extraído)
command -v fusermount3 >/dev/null || command -v fusermount >/dev/null || export APPIMAGE_EXTRACT_AND_RUN=1

echo "==> Descargando Playnix"
mkdir -p "$(dirname "$BIN")"
curl -fL --progress-bar -o "$BIN.new" "$URL"
chmod +x "$BIN.new"
mv "$BIN.new" "$BIN"

exec "$BIN" --cli "$@" </dev/tty
