<p align="center"><img src="data/playnix.svg" width="128" alt="Playnix"></p>

<h1 align="center">Playnix</h1>
<p align="center"><b>Google Play y apps de Android integradas en Linux.</b><br>
Cada app se abre en su propia ventana, como cualquier programa.</p>

---

Playnix instala y configura [Waydroid](https://waydro.id) para que Android no se sienta
como una máquina virtual:

- **Google Play en tu menú.** Lo que instales aparece como un programa más; clic
  derecho → *Desinstalar*.
- **Ventanas propias** para cada app, sin la pantalla de Android de fondo.
- **Doble clic en un `.apk`** para instalarlo, con una ventana que te avisa.
- **El teclado de tu PC** funciona en los campos de texto (sin teclado en pantalla).
- **Arranque sin esperas confusas**: si Android está apagado, ves "Abriendo…".
- **Detección automática**: tipo de GPU, CPU (traducción ARM), Docker (red), distro.
- **Certificación de Google Play** guiada: copia el código y abre la página por ti.

## Instalar

**Con un comando:**

```sh
curl -fsSL https://raw.githubusercontent.com/sthifen/playnix/main/install.sh | bash
```

**O con el AppImage:** descarga `Playnix-x86_64.AppImage` desde
[Releases](https://github.com/sthifen/playnix/releases/latest), dale permiso de
ejecución y ábrelo.

Te pedirá tu contraseña una sola vez. El único paso manual es iniciar sesión en la
página de Google para registrar el equipo (Google lo exige y no se puede automatizar).

### Requisitos

- PC x86_64 con una sesión **Wayland** (GNOME, KDE Plasma, Hyprland…)
- Kernel con **binder** (Fedora, Ubuntu y la mayoría ya lo traen; en Arch usa `linux-zen`)
- glibc 2.39+ (Ubuntu 24.04, Fedora 40, Debian 13, Linux Mint 22 o más nuevos)
- GTK 4.14+ y libadwaita 1.5+ (el comando de instalación los instala si faltan)
- ~4 GB libres

## Gráficos: importante si tienes NVIDIA

Android dibuja con una GPU de Mesa (Intel o AMD). Si tu escritorio usa una **NVIDIA como
GPU principal**, Playnix activa el *modo compatible* (dibujo por CPU): las apps normales
funcionan, pero **video y juegos van lentos**. Para aceleración completa en una laptop
híbrida, deja que el escritorio use la GPU integrada (Intel/AMD).

## Comandos

```text
playnix                      instalador / panel
playnix --cli [--yes]        instalar desde la terminal
playnix open <paquete>       abrir una app
playnix install-apk <apk>    instalar un APK
playnix uninstall <paquete>  desinstalar una app
```

## Construir

```sh
cargo build --release              # binario para tu sistema
packaging/build-appimage.sh        # AppImage portable (usa podman/docker)
```

## Créditos

Hecho sobre [Waydroid](https://github.com/waydroid/waydroid) y
[waydroid_script](https://github.com/casualsnek/waydroid_script). Tux fue creado por
Larry Ewing con The GIMP; el robot de Android es de Google (CC BY 3.0). Playnix no está
afiliado a Google ni a Waydroid.

Licencia: GPL-3.0-or-later.
