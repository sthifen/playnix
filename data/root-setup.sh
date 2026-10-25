#!/usr/bin/env bash
# Playnix — parte de la instalación que necesita root. La lanza `playnix` con
# pkexec (gráfico) o sudo (--cli). Emite líneas de protocolo en stdout:
#   @@STEP <id> <texto>   @@PROGRESS <0-100>   @@INFO <texto>   @@WARN <texto>
#   @@ERROR <texto>       @@DONE
# Opciones: --user NOMBRE --gpu auto|hw|sw --arm auto|houdini|ndk|none
#           [--widevine] [--reinit]
set -uo pipefail
export LC_ALL=C

OTA_SYSTEM=https://ota.waydro.id/system
OTA_VENDOR=https://ota.waydro.id/vendor
CFG=/var/lib/waydroid/waydroid.cfg
LIBEXEC=/usr/libexec/playnix
WS_DIR=/var/lib/playnix/waydroid_script
SRC_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

TARGET_USER="" GPU=auto ARM=auto WIDEVINE=0 REINIT=0
while [ $# -gt 0 ]; do
    case "$1" in
        --user) TARGET_USER=$2; shift ;;
        --gpu) GPU=$2; shift ;;
        --arm) ARM=$2; shift ;;
        --widevine) WIDEVINE=1 ;;
        --reinit) REINIT=1 ;;
        *) echo "@@ERROR Opción desconocida: $1"; exit 2 ;;
    esac
    shift
done

step()  { echo "@@STEP $1 $2"; }
info()  { echo "@@INFO $*"; }
warn()  { echo "@@WARN $*"; }
fail()  { echo "@@ERROR $*"; exit 1; }
run()   { "$@" 2>&1 | sed -u 's/^/@@LOG /'; return "${PIPESTATUS[0]}"; }

[ "$(id -u)" = 0 ] || fail "root-setup.sh debe correr como root"
[ -n "$TARGET_USER" ] && id "$TARGET_USER" >/dev/null 2>&1 || fail "Falta --user válido"
USER_HOME=$(getent passwd "$TARGET_USER" | cut -d: -f6)

# ---------------------------------------------------------------------------
step deps "Instalando componentes del sistema"
. /etc/os-release
falta=0
for c in waydroid git lzip python3 wl-copy; do command -v "$c" >/dev/null || falta=1; done
if [ "$falta" = 1 ]; then
    info "Distribución: ${PRETTY_NAME:-$ID}"
    if command -v dnf >/dev/null; then
        run dnf install -y waydroid git lzip python3 wl-clipboard || fail "dnf no pudo instalar los paquetes"
    elif command -v rpm-ostree >/dev/null; then
        run rpm-ostree install -y --idempotent waydroid git lzip wl-clipboard || fail "rpm-ostree falló"
        fail "Se instalaron paquetes del sistema: reinicia la PC y vuelve a abrir Playnix."
    elif command -v apt-get >/dev/null; then
        if ! apt-cache policy waydroid 2>/dev/null | grep -q 'Candidate: [0-9]'; then
            run apt-get install -y curl ca-certificates
            curl -fsSL https://repo.waydro.id | bash 2>&1 | sed -u 's/^/@@LOG /'
        fi
        run apt-get update
        run apt-get install -y waydroid git lzip python3 python3-venv wl-clipboard || fail "apt no pudo instalar los paquetes"
    elif command -v pacman >/dev/null; then
        run pacman -S --needed --noconfirm git lzip python wl-clipboard || fail "pacman falló"
        command -v waydroid >/dev/null || run pacman -S --needed --noconfirm waydroid ||
            fail "En Arch instala 'waydroid' desde AUR (yay -S waydroid) y vuelve a abrir Playnix."
    elif command -v zypper >/dev/null; then
        run zypper -n install waydroid git lzip python3 wl-clipboard || fail "zypper falló"
    elif command -v xbps-install >/dev/null; then
        run xbps-install -Sy waydroid git lzip python3 wl-clipboard || fail "xbps falló"
    else
        fail "No sé instalar paquetes en esta distribución. Instala waydroid a mano: https://docs.waydro.id"
    fi
fi
info "Waydroid $(waydroid --version 2>/dev/null)"

if [ ! -e /dev/binderfs/binder ] && [ ! -e /dev/binder ]; then
    modprobe binder_linux devices=binder,hwbinder,vndbinder 2>/dev/null || true
    grep -q binder /proc/filesystems ||
        fail "Tu kernel no tiene 'binder', que Android necesita. Usa un kernel como linux-zen o xanmod."
fi

# Docker deja la política FORWARD en DROP: Android tendría DNS pero no internet.
if systemctl is-active -q docker 2>/dev/null; then
    if ! grep -qs '"ip-forward-no-drop": *true' /etc/docker/daemon.json; then
        info "Docker detectado: dando internet a Android"
        mkdir -p /etc/docker
        python3 -I -c '
import json, os
p = "/etc/docker/daemon.json"
d = json.load(open(p)) if os.path.exists(p) and os.path.getsize(p) else {}
d["ip-forward-no-drop"] = True
json.dump(d, open(p, "w"), indent=2)'
        systemctl restart docker
    fi
    iptables -P FORWARD ACCEPT 2>/dev/null || true
fi

run systemctl enable --now waydroid-container || fail "No arrancó el servicio waydroid-container"

# ---------------------------------------------------------------------------
step image "Descargando Android con Google Play"
if [ -f "$CFG" ] && grep -q GAPPS "$CFG" && grep -q ota.waydro.id "$CFG" && [ "$REINIT" = 0 ]; then
    info "Android ya estaba instalado, se conserva"
else
    datos="$USER_HOME/.local/share/waydroid/data"
    if [ -d "$datos" ]; then
        resp="$USER_HOME/.local/share/playnix/respaldo-$(date +%Y%m%d-%H%M%S)"
        mkdir -p "$(dirname "$resp")" && mv "$datos" "$resp" && info "Datos anteriores guardados en $resp"
    fi
    # waydroid imprime "[Downloading]  12.85 MB/1191.2 MB  ..." con \r, primero la imagen
    # system (~86% del total) y luego vendor: lo convertimos en un solo porcentaje
    waydroid init -f -s GAPPS -c "$OTA_SYSTEM" -v "$OTA_VENDOR" 2>&1 | tr '\r' '\n' | awk '
        /\[Downloading\]/ { n = $2 + 0; split($3, b, "/"); t = b[2] + 0
                            if (t > 0) { p = (fase <= 1) ? int(n * 86 / t) : 86 + int(n * 14 / t)
                                         if (p != ult) { print "@@PROGRESS " p; ult = p; fflush() } }
                            next }
        /Downloading http/ { fase++; next }
        /Validating|Extracting/ { sub(/^\[[0-9:]+\] /, ""); print "@@INFO " $0; fflush(); next }
        NF { print "@@LOG " $0; fflush() }'
    [ "${PIPESTATUS[0]}" = 0 ] || fail "No se pudo descargar Android. Revisa tu conexión e inténtalo de nuevo."
fi

# ---------------------------------------------------------------------------
step gpu "Configurando gráficos"
detectar_gpu() {
    local mesa=0 nv=0 d drv
    for d in /sys/class/drm/renderD*; do
        drv=$(basename "$(readlink -f "$d/device/driver")")
        case "$drv" in i915|xe|amdgpu|radeon|nouveau|virtio*) mesa=1 ;; esac
    done
    for d in /sys/class/drm/card[0-9]; do
        drv=$(basename "$(readlink -f "$d/device/driver")")
        [ "$drv" = nvidia ] && [ "$(cat "$d/device/boot_vga" 2>/dev/null)" = 1 ] && nv=1
    done
    for f in /etc/udev/rules.d/*.rules; do
        grep -qs 'mutter-device-preferred-primary' "$f" && grep -qs '0x10de' "$f" && nv=1
    done
    if [ "$mesa" = 0 ] || [ "$nv" = 1 ]; then echo sw; else echo hw; fi
}
[ "$GPU" = auto ] && GPU=$(detectar_gpu)
sed -i '/^ro.hardware.gralloc = default$/d; /^ro.hardware.egl = swiftshader$/d' "$CFG"
if [ "$GPU" = sw ]; then
    sed -i '/^\[properties\]/a ro.hardware.gralloc = default\nro.hardware.egl = swiftshader' "$CFG"
    info "Modo compatible (tu GPU principal es NVIDIA): las apps se dibujan con la CPU"
else
    info "Aceleración por GPU activada"
fi
run waydroid upgrade --offline

# ---------------------------------------------------------------------------
step extras "Instalando compatibilidad con apps ARM"
if [ "$ARM" = auto ]; then
    case "$(lscpu | awk -F: '/Vendor ID/{gsub(/ /,"",$2); print $2; exit}')" in
        GenuineIntel) ARM=houdini ;; *) ARM=ndk ;;
    esac
fi
extras=()
if grep -q 'native.bridge = lib' "$CFG"; then
    info "La traducción ARM ya estaba instalada"
elif [ "$ARM" != none ]; then
    extras+=("lib$ARM")
fi
[ "$WIDEVINE" = 1 ] && extras+=(widevine)
if [ ${#extras[@]} -gt 0 ]; then
    mkdir -p "$(dirname "$WS_DIR")"
    [ -d "$WS_DIR/.git" ] || run git clone -q --depth 1 https://github.com/casualsnek/waydroid_script "$WS_DIR" ||
        fail "No se pudo descargar waydroid_script"
    run git -C "$WS_DIR" pull -q --ff-only || true
    [ -x "$WS_DIR/venv/bin/python3" ] || run python3 -m venv "$WS_DIR/venv" || fail "No se pudo crear el entorno de Python"
    run "$WS_DIR/venv/bin/pip" install -q -r "$WS_DIR/requirements.txt" || fail "No se pudieron instalar dependencias de Python"
    info "Instalando: ${extras[*]}"
    run "$WS_DIR/venv/bin/python3" "$WS_DIR/main.py" -a 13 install "${extras[@]}" || fail "Falló la instalación de ${extras[*]}"
fi

# ---------------------------------------------------------------------------
step helper "Integrando con el sistema"
install -D -m 755 "$SRC_DIR/playnix-priv" "$LIBEXEC/playnix-priv"
install -D -m 644 "$SRC_DIR/io.github.sthifen.playnix.policy" /usr/share/polkit-1/actions/io.github.sthifen.playnix.policy
info "Ayudante de permisos instalado"

echo "@@DONE"
