#!/usr/bin/env bash
# Compila playnix-clip.apk (proveedor de imágenes del portapapeles + servidor que
# corre como shell) sin Gradle: aapt2 → javac → d8 → zipalign → apksigner.
# Necesita el Android SDK en $ANDROID_HOME (por defecto ~/Android/Sdk) con
# build-tools 36.1.0 y la plataforma android-36. Si no hay javac, usa el módulo
# jdk.compiler del JRE. Resultado: data/clip/playnix-clip.apk (se versiona; Playnix
# lo embebe). La llave de firma se crea la primera vez (data/clip/playnix.keystore):
# Android no deja actualizar el APK si cambia, así que no hay que borrarla.
set -euo pipefail

DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
BT="$SDK/build-tools/36.1.0"
JAR="$SDK/platforms/android-36/android.jar"
MIN_SDK=28
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if command -v javac >/dev/null; then JAVAC=(javac); else JAVAC=(java -m jdk.compiler/com.sun.tools.javac.Main); fi

"$BT/aapt2" link -o "$TMP/base.apk" -I "$JAR" --manifest "$DIR/AndroidManifest.xml" \
    --min-sdk-version $MIN_SDK --target-sdk-version 33 --version-code 1 --version-name 1
mkdir -p "$TMP/classes" "$TMP/dex"
"${JAVAC[@]}" -source 11 -target 11 -nowarn -Xlint:-options -encoding UTF-8 \
    -classpath "$JAR" -d "$TMP/classes" $(find "$DIR/src" -name '*.java')
"$BT/d8" --release --min-api $MIN_SDK --lib "$JAR" --output "$TMP/dex" $(find "$TMP/classes" -name '*.class')
(cd "$TMP/dex" && zip -q -X "$TMP/base.apk" classes.dex)

KEYSTORE="$DIR/playnix.keystore"
if [[ ! -f "$KEYSTORE" ]]; then
    keytool -genkeypair -keystore "$KEYSTORE" -storepass playnix -keypass playnix \
        -alias playnix -keyalg RSA -keysize 2048 -validity 20000 -dname "CN=Playnix" >/dev/null 2>&1
fi
"$BT/zipalign" -P 16 -f 4 "$TMP/base.apk" "$TMP/aligned.apk"
"$BT/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:playnix --key-pass pass:playnix \
    --out "$DIR/playnix-clip.apk" "$TMP/aligned.apk"
rm -f "$DIR/playnix-clip.apk.idsig"
echo "Listo: $DIR/playnix-clip.apk ($(du -h "$DIR/playnix-clip.apk" | cut -f1))"
