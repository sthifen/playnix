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
# git y lzip solo hacen falta para Widevine (waydroid_script)
extra=""; extra_apt=""
[ "$WIDEVINE" = 1 ] && extra="git lzip" && extra_apt="git lzip python3-venv"
for c in waydroid python3 wl-copy debugfs adb $extra; do command -v "$c" >/dev/null || falta=1; done
if [ "$falta" = 1 ]; then
    info "Distribución: ${PRETTY_NAME:-$ID}"
    if command -v dnf >/dev/null; then
        run dnf install -y waydroid python3 wl-clipboard e2fsprogs android-tools $extra || fail "dnf no pudo instalar los paquetes"
    elif command -v rpm-ostree >/dev/null; then
        run rpm-ostree install -y --idempotent waydroid wl-clipboard android-tools $extra || fail "rpm-ostree falló"
        fail "Se instalaron paquetes del sistema: reinicia la PC y vuelve a abrir Playnix."
    elif command -v apt-get >/dev/null; then
        if ! apt-cache policy waydroid 2>/dev/null | grep -q 'Candidate: [0-9]'; then
            run apt-get install -y curl ca-certificates
            curl -fsSL https://repo.waydro.id | bash 2>&1 | sed -u 's/^/@@LOG /'
        fi
        run apt-get update
        run apt-get install -y waydroid python3 wl-clipboard e2fsprogs adb $extra_apt || fail "apt no pudo instalar los paquetes"
    elif command -v pacman >/dev/null; then
        run pacman -S --needed --noconfirm python wl-clipboard e2fsprogs android-tools $extra || fail "pacman falló"
        command -v waydroid >/dev/null || run pacman -S --needed --noconfirm waydroid ||
            fail "En Arch instala 'waydroid' desde AUR (yay -S waydroid) y vuelve a abrir Playnix."
    elif command -v zypper >/dev/null; then
        run zypper -n install waydroid python3 wl-clipboard e2fsprogs android-tools $extra || fail "zypper falló"
    elif command -v xbps-install >/dev/null; then
        run xbps-install -Sy waydroid python3 wl-clipboard e2fsprogs android-tools $extra || fail "xbps falló"
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

# Versiones anteriores de Playnix escribían "ip-forward-no-drop" en la configuración de
# Docker, que Docker anterior a la 28 no reconoce: no arrancaba. Se repara.
if grep -qs '"ip-forward-no-drop"' /etc/docker/daemon.json && systemctl is-failed -q docker 2>/dev/null; then
    python3 -I -c '
import json
p = "/etc/docker/daemon.json"
d = json.load(open(p))
d.pop("ip-forward-no-drop", None)
json.dump(d, open(p, "w"), indent=2)'
    systemctl restart docker && info "Docker reparado (una versión anterior de Playnix lo había detenido)"
fi
# Docker (y algunos firewalls) dejan la política FORWARD en DROP: Android tendría DNS
# pero no internet. Se deja pasar solo el tráfico de Android, sin tocar Docker ni sus
# contenedores; playnix-priv lo repite cada vez que arranca Android.
if [ "$(bash "$SRC_DIR/playnix-priv" net-fix)" = changed ]; then
    info "Firewall o Docker detectado: dando internet a Android"
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
    if ! python3 -I -c '
import sys, urllib.request, urllib.error
try:
    urllib.request.urlopen(sys.argv[1], timeout=20)
except urllib.error.HTTPError:
    pass  # el servidor respondió
' "$OTA_SYSTEM" >/dev/null 2>&1; then
        fail "No hay conexión con el servidor de Android (ota.waydro.id). Revisa tu internet, VPN o firewall e inténtalo de nuevo."
    fi
    # waydroid descarga sin límite de tiempo: si la conexión se atasca espera para
    # siempre. Con 60 s sin recibir nada falla, y se reintenta. Su salida se interpreta
    # aquí mismo y sin búfer: por una tubería (tr, awk) el progreso llegaba todo al
    # final y la instalación parecía congelada. Imprime "[Downloading] 12.85 MB/1191.2
    # MB ..." con \r, primero la imagen system (~86% del total) y luego vendor.
    waydroid_init() {
        python3 -I -c '
import os, re, socket, sys, runpy

real = sys.__stdout__
def emit(text):
    real.write(text + "\n")
    real.flush()

class Out:
    buf, fase, ult = "", 0, -1
    def write(self, text):
        *lines, self.buf = re.split(r"[\r\n]", self.buf + text)
        for line in lines:
            self.line(line)
        return len(text)
    def flush(self):
        pass
    def line(self, line):
        m = re.search(r"\[Downloading\]\s+([\d.]+) MB/([\d.]+) MB", line)
        if m:
            n, t = float(m[1]), float(m[2])
            if t > 0:
                p = int(n * 86 / t) if self.fase <= 1 else 86 + int(n * 14 / t)
                if p != self.ult:
                    emit("@@PROGRESS %d" % p)
                    self.ult = p
        elif "Downloading http" in line:
            self.fase += 1
        elif "Validating" in line or "Extracting" in line:
            emit("@@INFO " + re.sub(r"^\[[0-9:]+\] ", "", line))
        elif line.strip():
            emit("@@LOG " + line)

sys.stdout = sys.stderr = Out()
w = os.path.realpath("/usr/bin/waydroid")
sys.path.insert(0, os.path.dirname(w))
socket.setdefaulttimeout(60)
sys.argv = ["waydroid"] + sys.argv[1:]
runpy.run_path(w, run_name="__main__")
' init -f -s GAPPS -c "$OTA_SYSTEM" -v "$OTA_VENDOR"
    }
    descargado=0
    for intento in 1 2 3; do
        [ "$intento" = 1 ] || info "La descarga se detuvo; reintentando ($intento de 3)…"
        waydroid_init && { descargado=1; break; }
    done
    [ "$descargado" = 1 ] || fail "No se pudo descargar Android: la conexión se cortó varias veces. Revisa tu internet e inténtalo de nuevo."
fi

# ---------------------------------------------------------------------------
step gpu "Configurando gráficos"
# La elección se guarda: "auto" hace que Playnix revise la GPU real cada vez que
# arranca Android (playnix-priv gpu-auto), por si cambia sin reinstalar.
mkdir -p /var/lib/playnix
echo "$GPU" > /var/lib/playnix/gpu-mode
modo=$GPU
[ "$modo" = auto ] && modo=$(bash "$SRC_DIR/playnix-priv" gpu-detect)
r=$(bash "$SRC_DIR/playnix-priv" gpu-auto)
echo "@@LOG gpu-auto: $r"
if [ "$modo" = sw ]; then
    info "Modo compatible: tu escritorio usa una NVIDIA, las apps se dibujan con la CPU"
else
    info "Aceleración por GPU activada"
fi

# ---------------------------------------------------------------------------
step extras "Instalando compatibilidad con apps ARM"
if [ "$ARM" = auto ]; then
    case "$(lscpu | awk -F: '/Vendor ID/{gsub(/ /,"",$2); print $2; exit}')" in
        GenuineIntel) ARM=houdini ;; *) ARM=ndk ;;
    esac
fi
# libhoudini es más compatible en Intel y libndk en AMD. Se instala directo en el
# overlay (data/arm-bridge.py); el zip queda en caché para cambiar sin descargar.
# los avisos de descarga (stderr) salen al momento; stdout trae ok/changed
{ r=$(python3 -I "$SRC_DIR/arm-bridge.py" "$ARM" 2>&3); } 3>&1 || fail "No se pudo instalar la traducción ARM"
[ "$ARM" = none ] || info "Traducción ARM: lib$ARM"
cambio=0
echo "$r" | grep -qx changed && cambio=1

# Widevine (opcional) sigue viniendo de waydroid_script
if [ "$WIDEVINE" = 1 ] && ! grep -q widevine "$CFG"; then
    mkdir -p "$(dirname "$WS_DIR")"
    [ -d "$WS_DIR/.git" ] || run git clone -q --depth 1 https://github.com/casualsnek/waydroid_script "$WS_DIR" ||
        fail "No se pudo descargar waydroid_script"
    run git -C "$WS_DIR" pull -q --ff-only || true
    [ -x "$WS_DIR/venv/bin/python3" ] || run python3 -m venv "$WS_DIR/venv" || fail "No se pudo crear el entorno de Python"
    run "$WS_DIR/venv/bin/pip" install -q -r "$WS_DIR/requirements.txt" || fail "No se pudieron instalar dependencias de Python"
    info "Instalando: widevine"
    run "$WS_DIR/venv/bin/python3" "$WS_DIR/main.py" -a 13 install widevine || fail "Falló la instalación de widevine"
    cambio=0  # waydroid_script ya reinició Android
fi
[ "$cambio" = 1 ] && { run waydroid upgrade --offline || true; }

# ---------------------------------------------------------------------------
step helper "Integrando con el sistema"
install -D -m 755 "$SRC_DIR/playnix-priv" "$LIBEXEC/playnix-priv"
install -D -m 644 "$SRC_DIR/io.github.sthifen.playnix.policy" /usr/share/polkit-1/actions/io.github.sthifen.playnix.policy
info "Ayudante de permisos instalado"
install -D -m 644 "$SRC_DIR/libpnxh.so" "$LIBEXEC/libpnxh.so"
install -D -m 644 "$SRC_DIR/hwc-patch.py" "$LIBEXEC/hwc-patch.py"
install -D -m 644 "$SRC_DIR/arm-bridge.py" "$LIBEXEC/arm-bridge.py"
echo "@@LOG hwc-fix: $("$LIBEXEC/playnix-priv" hwc-fix)"

echo "@@DONE"
