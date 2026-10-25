//! Lectura mínima del AndroidManifest.xml binario (AXML) de un APK: paquete,
//! versión y nombre (cuando es texto literal y no una referencia a recursos).

use std::io::Read;
use std::path::Path;

#[derive(Default, Debug)]
pub struct ApkInfo {
    pub package: String,
    pub version: String,
    pub label: String,
    /// La app se declara juego (`android:isGame` o `android:appCategory="game"`).
    pub game: bool,
}

fn u16_at(d: &[u8], p: usize) -> Option<u16> {
    d.get(p..p + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], p: usize) -> Option<u32> {
    d.get(p..p + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_manifest(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip_entry(file, "AndroidManifest.xml")?;
    let mut buf = Vec::new();
    zip.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Busca una entrada en el directorio central del ZIP y la devuelve descomprimida.
fn zip_entry(mut f: std::fs::File, name: &str) -> Option<std::io::Cursor<Vec<u8>>> {
    use std::io::{Seek, SeekFrom};
    let len = f.seek(SeekFrom::End(0)).ok()?;
    let tail = len.min(65_557);
    f.seek(SeekFrom::Start(len - tail)).ok()?;
    let mut end = vec![0; tail as usize];
    f.read_exact(&mut end).ok()?;
    let eocd = (0..end.len().saturating_sub(21)).rev().find(|&i| end[i..i + 4] == [0x50, 0x4b, 5, 6])?;
    let cd_size = u32_at(&end, eocd + 12)? as usize;
    let cd_off = u32_at(&end, eocd + 16)? as u64;
    f.seek(SeekFrom::Start(cd_off)).ok()?;
    let mut cd = vec![0; cd_size];
    f.read_exact(&mut cd).ok()?;
    let mut p = 0;
    while p + 46 <= cd.len() && cd[p..p + 4] == [0x50, 0x4b, 1, 2] {
        let method = u16_at(&cd, p + 10)?;
        let csize = u32_at(&cd, p + 20)? as usize;
        let nlen = u16_at(&cd, p + 28)? as usize;
        let xlen = u16_at(&cd, p + 30)? as usize;
        let clen = u16_at(&cd, p + 32)? as usize;
        let local = u32_at(&cd, p + 42)? as u64;
        if cd.get(p + 46..p + 46 + nlen)? == name.as_bytes() {
            let mut lh = [0u8; 30];
            f.seek(SeekFrom::Start(local)).ok()?;
            f.read_exact(&mut lh).ok()?;
            let skip = 30 + u16_at(&lh, 26)? as u64 + u16_at(&lh, 28)? as u64;
            f.seek(SeekFrom::Start(local + skip)).ok()?;
            let mut raw = vec![0; csize];
            f.read_exact(&mut raw).ok()?;
            let data = match method {
                0 => raw,
                8 => inflate(&raw)?,
                _ => return None,
            };
            return Some(std::io::Cursor::new(data));
        }
        p += 46 + nlen + xlen + clen;
    }
    None
}

/// Descompresión DEFLATE usando gio (ya está en el binario por GTK).
fn inflate(raw: &[u8]) -> Option<Vec<u8>> {
    use gtk::gio::prelude::*;
    use gtk::{gio, glib};
    let conv = gio::ZlibDecompressor::new(gio::ZlibCompressorFormat::Raw);
    let input = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(raw));
    let stream = gio::ConverterInputStream::new(&input, &conv);
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = stream.read(&mut buf[..], gio::Cancellable::NONE).ok()?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    Some(out)
}

fn string_pool(d: &[u8], off: usize) -> Option<Vec<String>> {
    let count = u32_at(d, off + 8)? as usize;
    let flags = u32_at(d, off + 16)?;
    let start = u32_at(d, off + 20)? as usize;
    let utf8 = flags & 0x100 != 0;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut p = off + start + u32_at(d, off + 28 + 4 * i)? as usize;
        let s = if utf8 {
            p += if d.get(p)? & 0x80 != 0 { 2 } else { 1 }; // longitud en caracteres
            let mut n = *d.get(p)? as usize;
            if n & 0x80 != 0 {
                n = ((n & 0x7f) << 8) | *d.get(p + 1)? as usize;
                p += 2;
            } else {
                p += 1;
            }
            String::from_utf8_lossy(d.get(p..p + n)?).into_owned()
        } else {
            let mut n = u16_at(d, p)? as usize;
            p += 2;
            if n & 0x8000 != 0 {
                n = ((n & 0x7fff) << 16) | u16_at(d, p)? as usize;
                p += 2;
            }
            let units: Vec<u16> = (0..n).filter_map(|k| u16_at(d, p + 2 * k)).collect();
            String::from_utf16_lossy(&units)
        };
        out.push(s);
    }
    Some(out)
}

pub fn read(path: &Path) -> Option<ApkInfo> {
    let d = read_manifest(path)?;
    let mut strings = Vec::new();
    // ID de recurso de cada nombre de atributo (algunos APK dejan el nombre vacío)
    let mut res_ids: Vec<u32> = Vec::new();
    let mut info = ApkInfo::default();
    let mut pos = 8;
    while pos + 8 <= d.len() {
        let ctype = u16_at(&d, pos)?;
        let csize = u32_at(&d, pos + 4)? as usize;
        if csize == 0 {
            break;
        }
        if ctype == 0x0001 {
            strings = string_pool(&d, pos)?;
        } else if ctype == 0x0180 {
            res_ids = (pos + 8..pos + csize).step_by(4).filter_map(|p| u32_at(&d, p)).collect();
        } else if ctype == 0x0102 {
            let ext = pos + 16;
            let name = strings.get(u32_at(&d, ext + 4)? as usize).cloned().unwrap_or_default();
            let astart = u16_at(&d, ext + 8)? as usize;
            let asize = u16_at(&d, ext + 10)? as usize;
            let acount = u16_at(&d, ext + 12)? as usize;
            for i in 0..acount {
                let a = ext + astart + i * asize;
                let key = strings.get(u32_at(&d, a + 4)? as usize).cloned().unwrap_or_default();
                let key_idx = u32_at(&d, a + 4)? as usize;
                let raw = u32_at(&d, a + 8)?;
                let dtype = *d.get(a + 15)?;
                let val = u32_at(&d, a + 16)?;
                if name == "application" {
                    let id = res_ids.get(key_idx).copied().unwrap_or(0);
                    // isGame: booleano (0x12); appCategory: entero (0x10/0x11), game = 0
                    if (key == "isGame" || id == 0x0101_03f4) && dtype == 0x12 && val != 0 {
                        info.game = true;
                    }
                    if (key == "appCategory" || id == 0x0101_0545) && (dtype == 0x10 || dtype == 0x11) && val == 0 {
                        info.game = true;
                    }
                }
                let s = if raw != u32::MAX {
                    strings.get(raw as usize).cloned()
                } else if dtype == 3 {
                    strings.get(val as usize).cloned()
                } else {
                    None
                };
                let Some(s) = s else { continue };
                match (name.as_str(), key.as_str()) {
                    ("manifest", "package") => info.package = s,
                    ("manifest", "versionName") => info.version = s,
                    ("application", "label") => info.label = s,
                    _ => {}
                }
            }
            if name == "application" {
                break;
            }
        }
        pos += csize;
    }
    (!info.package.is_empty()).then_some(info)
}

/// Nombre para mostrar cuando el APK no trae etiqueta literal: limpia el nombre del archivo.
pub fn pretty_filename(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    stem.replace(['_', '-', '.'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Apps partidas: .xapk (APKPure, Uptodown…) y .apks (SAI, bundletool)

pub fn is_bundle(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("xapk" | "apks")
    )
}

/// Lee los datos de un paquete partido: manifest.json si lo trae; si no, el base.apk.
pub fn read_bundle(path: &Path) -> Option<ApkInfo> {
    const PY: &str = r#"
import json, sys, tempfile, zipfile
z = zipfile.ZipFile(sys.argv[1])
names = z.namelist()
if "manifest.json" in names:
    m = json.loads(z.read("manifest.json"))
    print(m.get("package_name", ""), m.get("version_name", ""), m.get("name", ""), "", sep="\t")
else:
    apks = [n for n in names if n.endswith(".apk")]
    base = "base.apk" if "base.apk" in names else (apks[0] if apks else None)
    if base is None:
        sys.exit(1)
    t = tempfile.NamedTemporaryFile(suffix=".apk", delete=False)
    t.write(z.read(base))
    print("", "", "", t.name, sep="\t")
"#;
    let out = std::process::Command::new("python3").args(["-I", "-c", PY]).arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string();
    let mut f = line.split('\t');
    let (pkg, ver, name, tmp) = (f.next()?, f.next()?, f.next()?, f.next().unwrap_or(""));
    if !tmp.is_empty() {
        let info = read(Path::new(tmp));
        let _ = std::fs::remove_file(tmp);
        return info;
    }
    (!pkg.is_empty()).then(|| ApkInfo { package: pkg.into(), version: ver.into(), label: name.into(), game: false })
}

/// Extrae los .apk (y los OBB) de un paquete partido en `dest`.
pub fn extract_bundle(path: &Path, dest: &Path) -> Result<usize, String> {
    const PY: &str = r#"
import os, shutil, sys, zipfile
z, dest = zipfile.ZipFile(sys.argv[1]), sys.argv[2]
n = 0
for info in z.infolist():
    name = info.filename
    if info.is_dir() or ".." in name.split("/"):
        continue
    if name.endswith(".apk"):
        out = os.path.join(dest, "%02d-%s" % (n, os.path.basename(name)))
        n += 1
    elif name.startswith("Android/obb/") and name.endswith(".obb"):
        out = os.path.join(dest, "obb", *name.split("/")[2:])
    else:
        continue
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with z.open(info) as s, open(out, "wb") as d:
        shutil.copyfileobj(s, d, 1 << 20)
print(n)
"#;
    let out = std::process::Command::new("python3")
        .args(["-I", "-c", PY])
        .arg(path)
        .arg(dest)
        .output()
        .map_err(|e| e.to_string())?;
    let n: usize = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0);
    if !out.status.success() || n == 0 {
        return Err("El archivo no contiene ninguna app".into());
    }
    Ok(n)
}
