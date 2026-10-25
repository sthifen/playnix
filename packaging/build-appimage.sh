#!/usr/bin/env bash
# Construye Playnix-x86_64.AppImage dentro de un contenedor Ubuntu 24.04, para que
# el binario use una glibc antigua y funcione en casi cualquier distro moderna.
# Requisitos: podman (o docker). Resultado: dist/Playnix-x86_64.AppImage
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ENGINE=$(command -v podman || command -v docker) || { echo "Necesito podman o docker" >&2; exit 1; }
IMAGE=playnix-builder:24.04
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)

if ! "$ENGINE" image exists "$IMAGE" 2>/dev/null && ! "$ENGINE" image inspect "$IMAGE" >/dev/null 2>&1; then
    "$ENGINE" build -t "$IMAGE" -f - "$ROOT/packaging" <<'EOF'
FROM docker.io/library/ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        curl ca-certificates build-essential pkg-config libgtk-4-dev libadwaita-1-dev file desktop-file-utils \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
RUN curl -fsSL -o /usr/local/bin/appimagetool \
        https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage \
    && chmod +x /usr/local/bin/appimagetool
ENV PATH=/root/.cargo/bin:$PATH APPIMAGE_EXTRACT_AND_RUN=1
EOF
fi

mkdir -p "$ROOT/dist"
"$ENGINE" run --rm -v "$ROOT:/src:Z" -w /src -e VERSION="$VERSION" "$IMAGE" bash -euc '
    CARGO_TARGET_DIR=/src/target/appimage cargo build --release --locked
    APPDIR=$(mktemp -d)/Playnix.AppDir
    install -Dm755 target/appimage/release/playnix "$APPDIR/usr/bin/playnix"
    install -Dm644 data/playnix.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/playnix.svg"
    cp data/playnix.svg "$APPDIR/playnix.svg"
    ln -s playnix.svg "$APPDIR/.DirIcon"
    cat > "$APPDIR/io.github.sthifen.Playnix.desktop" <<EOD
[Desktop Entry]
Type=Application
Name=Playnix
Comment=Android apps and Google Play on Linux
Comment[es]=Apps de Android y Google Play en Linux
Exec=playnix
Icon=playnix
Categories=System;
EOD
    cat > "$APPDIR/AppRun" <<"EOD"
#!/bin/sh
HERE=$(dirname "$(readlink -f "$0")")
exec "$HERE/usr/bin/playnix" "$@"
EOD
    chmod +x "$APPDIR/AppRun"
    desktop-file-validate "$APPDIR/io.github.sthifen.Playnix.desktop"
    ARCH=x86_64 appimagetool --no-appstream "$APPDIR" dist/Playnix-x86_64.AppImage
    chown "$(stat -c %u:%g /src)" dist/Playnix-x86_64.AppImage
'
echo "Listo: dist/Playnix-x86_64.AppImage (v$VERSION)"
