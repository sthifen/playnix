"""Instala la traducción ARM de Android (libhoudini o libndk_translation) en el
overlay de Waydroid, sin waydroid_script: descarga un zip fijado, copia su
carpeta prebuilts/ a /system y escribe las propiedades en waydroid.cfg.

Uso (como root): python3 -I arm-bridge.py houdini|ndk|none
Imprime "ok" si ya estaba así o "changed" si hay que reiniciar Android
(`waydroid upgrade --offline`). Los avisos (@@INFO, @@ERROR) van por stderr.
"""
import configparser
import glob
import hashlib
import json
import os
import shutil
import sys
import tempfile
import urllib.request
import zipfile

OVERLAY = "/var/lib/waydroid/overlay/system"
CFG = "/var/lib/waydroid/waydroid.cfg"
CACHE = "/var/lib/playnix/cache"
MANIFEST = "/var/lib/playnix/arm-bridge.json"

COMMON_PROPS = {
    "ro.product.cpu.abilist": "x86_64,x86,arm64-v8a,armeabi-v7a,armeabi",
    "ro.product.cpu.abilist32": "x86,armeabi-v7a,armeabi",
    "ro.product.cpu.abilist64": "x86_64,arm64-v8a",
    "ro.enable.native.bridge.exec": "1",
    "ro.dalvik.vm.isa.arm": "x86",
    "ro.dalvik.vm.isa.arm64": "x86_64",
}

# Mismas versiones (Android 13) que fijaba waydroid_script.
BRIDGES = {
    "houdini": {
        "name": "libhoudini",
        "url": "https://github.com/supremegamers/vendor_intel_proprietary_houdini/archive/2f8f088671182e17e67321e098e8411a3972a628.zip",
        "sha256": "2e82cdc88ddc4d418f7fb861aeebb7c49c83a91b97f57da10510b5e1146a4ed5",
        "mb": 75,
        "props": {**COMMON_PROPS, "ro.dalvik.vm.native.bridge": "libhoudini.so"},
        # houdini no trae cómo registrar sus ejecutables ARM en binfmt_misc
        "rc": ("etc/init/houdini.rc", "\n".join([
            "on early-init",
            "    mount binfmt_misc binfmt_misc /proc/sys/fs/binfmt_misc",
            "",
            "on property:ro.enable.native.bridge.exec=1",
        ] + [
            f"    exec -- /system/bin/sh -c \"echo ':{name}:M::\\\\x7f\\\\x45\\\\x4c\\\\x46\\\\x{cls}\\\\x01\\\\x01\\\\x00"
            f"\\\\x00\\\\x00\\\\x00\\\\x00\\\\x00\\\\x00\\\\x00\\\\x00\\\\x{typ}\\\\x00\\\\x{mach}::/system/bin/{runner}:P' "
            f"{op} /proc/sys/fs/binfmt_misc/register\""
            for name, cls, typ, mach, runner, op in [
                ("arm_exe", "01", "02", "28", "houdini", ">"),
                ("arm_dyn", "01", "03", "28", "houdini", ">>"),
                ("arm64_exe", "02", "02", "b7", "houdini64", ">>"),
                ("arm64_dyn", "02", "03", "b7", "houdini64", ">>"),
            ]
        ]) + "\n"),
    },
    "ndk": {
        "name": "libndk",
        "url": "https://github.com/supremegamers/vendor_google_proprietary_ndk_translation-prebuilt/archive/68734c52556d3d7a6db34c603dd9276915c29f2f.zip",
        "sha256": "a142d1586c9eafb5edf62277110f2128bc03066179a6783bcaa33ee322e1cbd0",
        "mb": 18,
        "props": {
            **COMMON_PROPS,
            "ro.dalvik.vm.native.bridge": "libndk_translation.so",
            "ro.vendor.enable.native.bridge.exec": "1",
            "ro.vendor.enable.native.bridge.exec64": "1",
            "ro.ndk_translation.version": "0.2.3",
        },
    },
}

# Lo que dejaba waydroid_script, para poder cambiar de una instalación suya.
LEGACY = [
    "bin/arm", "bin/arm64", "bin/houdini", "bin/houdini64", "bin/ndk_translation_program_runner_binfmt_misc*",
    "etc/binfmt_misc", "etc/init/houdini.rc", "etc/init/ndk_translation.rc", "etc/ld.config.arm*.txt",
    "etc/cpuinfo.arm*.txt", "lib/arm", "lib64/arm64", "lib/libhoudini.so", "lib64/libhoudini.so",
    "lib/libndk*", "lib64/libndk*",
]


def info(msg):
    print(f"@@INFO {msg}", file=sys.stderr, flush=True)


def current():
    try:
        with open(MANIFEST) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def remove(manifest):
    paths = [os.path.join(OVERLAY, p) for p in (manifest or {}).get("files", [])]
    if manifest is None:
        paths = [g for pat in LEGACY for g in glob.glob(os.path.join(OVERLAY, pat))]
    for p in paths:
        if os.path.isdir(p) and not os.path.islink(p):
            shutil.rmtree(p, ignore_errors=True)
        elif os.path.lexists(p):
            os.remove(p)
    # carpetas que quedaron vacías
    for d in sorted({os.path.dirname(p) for p in paths}, key=len, reverse=True):
        while d.startswith(OVERLAY + "/") and os.path.isdir(d) and not os.listdir(d):
            os.rmdir(d)
            d = os.path.dirname(d)


def fetch(bridge):
    os.makedirs(CACHE, exist_ok=True)
    path = os.path.join(CACHE, bridge["name"] + ".zip")

    def ok():
        if not os.path.isfile(path):
            return False
        h = hashlib.sha256()
        with open(path, "rb") as f:
            for chunk in iter(lambda: f.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest() == bridge["sha256"]

    if not ok():
        info(f"Descargando {bridge['name']} ({bridge['mb']} MB)")
        tmp = path + ".part"
        try:
            with urllib.request.urlopen(bridge["url"], timeout=60) as r, open(tmp, "wb") as f:
                shutil.copyfileobj(r, f, 1 << 20)
        except OSError as e:
            sys.exit(f"@@ERROR No se pudo descargar {bridge['name']}: {e}")
        os.replace(tmp, path)
        if not ok():
            os.remove(path)
            sys.exit(f"@@ERROR {bridge['name']}: la descarga no coincide con la firma esperada")
    return path


def install(bridge, zpath):
    """Copia prebuilts/ al overlay. Devuelve los archivos escritos (relativos)."""
    files = []
    with zipfile.ZipFile(zpath) as z:
        for m in z.infolist():
            parts = m.filename.split("/")
            if len(parts) < 3 or parts[1] != "prebuilts" or m.is_dir() or ".." in parts:
                continue
            rel = "/".join(parts[2:])
            dst = os.path.join(OVERLAY, rel)
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            with z.open(m) as src, open(dst, "wb") as out:
                shutil.copyfileobj(src, out)
            # bin/: root:shell 755; el resto root:root 644 (como en una imagen de Android)
            in_bin = parts[2] == "bin"
            os.chown(dst, 0, 2000 if in_bin else 0)
            os.chmod(dst, 0o755 if in_bin else 0o644)
            files.append(rel)
    rc = bridge.get("rc")
    if rc:
        dst = os.path.join(OVERLAY, rc[0])
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        with open(dst, "w") as f:
            f.write(rc[1])
        os.chmod(dst, 0o644)
        files.append(rc[0])
    for root, dirs, _ in os.walk(OVERLAY):
        for d in dirs:
            os.chmod(os.path.join(root, d), 0o755)
    return files


def set_props(props):
    cfg = configparser.ConfigParser()
    cfg.optionxform = str
    cfg.read(CFG)
    if not cfg.has_section("properties"):
        cfg.add_section("properties")
    for b in BRIDGES.values():
        for k in b["props"]:
            cfg.remove_option("properties", k)
    for k, v in props.items():
        cfg.set("properties", k, v)
    with open(CFG, "w") as f:
        cfg.write(f)


def main(want):
    if want not in (*BRIDGES, "none"):
        sys.exit("uso: arm-bridge.py houdini|ndk|none")
    have = current()
    if have and have.get("bridge") == want and all(
        os.path.lexists(os.path.join(OVERLAY, p)) for p in have.get("files", [])
    ):
        print("ok")
        return
    if have is None and want == "none" and not glob.glob(os.path.join(OVERLAY, "lib64/libhoudini.so")) \
            and not glob.glob(os.path.join(OVERLAY, "lib64/libndk_translation.so")):
        print("ok")
        return
    bridge = BRIDGES.get(want)
    zpath = fetch(bridge) if bridge else None  # descargar antes de borrar lo que funciona
    remove(have)
    files = install(bridge, zpath) if bridge else []
    set_props(bridge["props"] if bridge else {})
    os.makedirs(os.path.dirname(MANIFEST), exist_ok=True)
    with tempfile.NamedTemporaryFile("w", dir=os.path.dirname(MANIFEST), delete=False) as f:
        json.dump({"bridge": want, "files": files}, f)
    os.replace(f.name, MANIFEST)
    print("changed")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) == 2 else "")
