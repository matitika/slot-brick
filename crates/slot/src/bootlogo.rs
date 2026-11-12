use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const CARD_LOGO: &str = "System/bootlogo.bmp";
pub const USER_LOGO: &str = "Config/bootlogo.bmp";
pub const INSTALLED_COPY: &str = "Cache/bootlogo.installed";
const LOGO: &str = "bootlogo.bmp";
const ORIGINAL: &str = "bootlogo.baseos.bmp";
const PARTITION: &str = "boot-resource";
const MOUNT: &str = "/tmp/slot-boot-resource";

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Same,
    Installed,
    Invalid,
}

pub struct Bmp {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<[u8; 3]>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

pub fn size(bmp: &[u8]) -> Option<(usize, usize)> {
    if bmp.get(0..2)? != b"BM" || u32_at(bmp, 14)? < 40 {
        return None;
    }
    let w = u32_at(bmp, 18)? as i32;
    let h = u32_at(bmp, 22)? as i32;
    let (w, h) = (w.unsigned_abs() as usize, h.unsigned_abs() as usize);
    ((1..=4096).contains(&w) && (1..=4096).contains(&h)).then_some((w, h))
}

pub fn decode(bmp: &[u8]) -> Option<Bmp> {
    let (width, height) = size(bmp)?;
    if u16_at(bmp, 28)? != 24 || u32_at(bmp, 30)? != 0 {
        return None;
    }
    let top_down = (u32_at(bmp, 22)? as i32) < 0;
    let offset = u32_at(bmp, 10)? as usize;
    let stride = (width * 3 + 3) & !3;
    let data = bmp.get(offset..offset + stride * height)?;
    let mut rgb = Vec::with_capacity(width * height);
    for y in 0..height {
        let row = if top_down { y } else { height - 1 - y };
        for px in data[row * stride..].chunks_exact(3).take(width) {
            rgb.push([px[2], px[1], px[0]]);
        }
    }
    Some(Bmp { width, height, rgb })
}

pub fn encode(img: &Bmp) -> Vec<u8> {
    let stride = (img.width * 3 + 3) & !3;
    let data = stride * img.height;
    let mut out = Vec::with_capacity(54 + data);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + data) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(img.width as i32).to_le_bytes());
    out.extend_from_slice(&(img.height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(data as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..img.height).rev() {
        for c in &img.rgb[y * img.width..(y + 1) * img.width] {
            out.extend_from_slice(&[c[2], c[1], c[0]]);
        }
        out.resize(out.len() + stride - img.width * 3, 0);
    }
    out
}

fn turned(img: &Bmp) -> Bmp {
    let (w, h) = (img.height, img.width);
    let mut rgb = vec![[0; 3]; w * h];
    for y in 0..img.height {
        for x in 0..img.width {
            rgb[(img.width - 1 - x) * w + y] = img.rgb[y * img.width + x];
        }
    }
    Bmp {
        width: w,
        height: h,
        rgb,
    }
}

pub fn compose(wordmark: &Bmp, (width, height): (usize, usize)) -> Option<Bmp> {
    let turned_mark;
    let mark = if height > width {
        turned_mark = turned(wordmark);
        &turned_mark
    } else {
        wordmark
    };
    if mark.width > width || mark.height > height {
        return None;
    }
    let mut rgb = vec![*mark.rgb.first()?; width * height];
    let (x0, y0) = ((width - mark.width) / 2, (height - mark.height) / 2);
    for y in 0..mark.height {
        let at = (y0 + y) * width + x0;
        rgb[at..at + mark.width].copy_from_slice(&mark.rgb[y * mark.width..(y + 1) * mark.width]);
    }
    Some(Bmp { width, height, rgb })
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Logo {
    Exact(Vec<u8>),
    Wordmark(Vec<u8>),
}

struct Plan {
    logo: Vec<u8>,
    current: Option<Vec<u8>>,
    original: Option<Vec<u8>>,
}

fn built(logo: &Logo, current: Option<&[u8]>, original: Option<&[u8]>) -> Option<Vec<u8>> {
    match logo {
        Logo::Exact(bytes) => Some(bytes.clone()),
        Logo::Wordmark(bytes) => {
            let mark = decode(bytes)?;
            let panel = original.or(current).and_then(size)?;
            compose(&mark, panel).map(|b| encode(&b))
        }
    }
}

fn plan(logo: &Logo, dir: &Path) -> Result<Plan, Outcome> {
    let current = fs::read(dir.join(LOGO)).ok();
    let original = fs::read(dir.join(ORIGINAL)).ok();
    let logo = built(logo, current.as_deref(), original.as_deref()).ok_or(Outcome::Invalid)?;
    if current.as_deref() == Some(&logo[..]) {
        return Err(Outcome::Same);
    }
    Ok(Plan {
        logo,
        current,
        original,
    })
}

pub fn pending(logo: &Logo, dir: &Path) -> Option<Outcome> {
    plan(logo, dir).err()
}

pub fn install(logo: &Logo, dir: &Path) -> io::Result<Outcome> {
    let Plan {
        logo,
        current,
        original,
    } = match plan(logo, dir) {
        Ok(plan) => plan,
        Err(outcome) => return Ok(outcome),
    };
    if let Some(current) = &current {
        if original.is_none() {
            replace(dir, ORIGINAL, current)?;
        }
        if current.len() == logo.len() {
            overwrite(dir, LOGO, &logo)?;
            return Ok(Outcome::Installed);
        }
    }
    replace(dir, LOGO, &logo)?;
    Ok(Outcome::Installed)
}

fn overwrite(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let mut f = fs::OpenOptions::new().write(true).open(dir.join(name))?;
    f.write_all(bytes)?;
    f.sync_all()
}

fn replace(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = dir.join("bootlogo.tmp");
    let mut f = File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, dir.join(name))?;
    File::open(dir)?.sync_all()
}

pub fn chosen(card: &Path) -> Option<Logo> {
    if let Ok(logo) = fs::read(card.join(USER_LOGO)) {
        return Some(Logo::Exact(logo));
    }
    let shipped = fs::read(card.join(CARD_LOGO)).ok()?;
    if decode(&shipped).is_none() {
        eprintln!("slot: bootlogo: {CARD_LOGO} is not a 24-bit bmp, skipping it");
        return None;
    }
    Some(Logo::Wordmark(shipped))
}

pub fn remember(card: &Path) {
    let user = card.join(USER_LOGO);
    let source = if user.is_file() { user } else { card.join(CARD_LOGO) };
    let Ok(logo) = fs::read(source) else {
        return;
    };
    let copy = card.join(INSTALLED_COPY);
    let (Some(dir), Some(name)) = (copy.parent(), copy.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    if let Err(e) = fs::create_dir_all(dir).and_then(|_| replace(dir, name, &logo)) {
        eprintln!("slot: bootlogo: could not keep the installed copy: {e}");
    }
}

pub fn refresh(card: &Path) {
    let Some(logo) = chosen(card) else {
        return;
    };
    let Some(device) = partition(PARTITION) else {
        eprintln!("slot: bootlogo: no {PARTITION} partition");
        return;
    };
    if fs::create_dir_all(MOUNT).is_err()
        || !run("mount", &["-t", "vfat", "-o", "ro,noatime", &device, MOUNT])
    {
        eprintln!("slot: bootlogo: could not mount {device}");
        return;
    }
    match pending(&logo, Path::new(MOUNT)) {
        Some(Outcome::Invalid) => eprintln!("slot: bootlogo: no logo fits this panel, left alone"),
        Some(_) => {}
        None if !run("mount", &["-o", "remount,rw,noatime", MOUNT]) => {
            eprintln!("slot: bootlogo: could not remount {device} to write the logo")
        }
        None => match install(&logo, Path::new(MOUNT)) {
            Ok(Outcome::Installed) => eprintln!("slot: bootlogo: installed"),
            Ok(_) => {}
            Err(e) => eprintln!("slot: bootlogo: {e}"),
        },
    }
    if !run("umount", &[MOUNT]) {
        eprintln!("slot: bootlogo: could not unmount {MOUNT}");
    }
}

fn partition(name: &str) -> Option<String> {
    for entry in fs::read_dir("/sys/class/block").ok()?.flatten() {
        let uevent = fs::read_to_string(entry.path().join("uevent")).unwrap_or_default();
        if uevent.lines().any(|l| l == format!("PARTNAME={name}")) {
            return Some(
                PathBuf::from("/dev")
                    .join(entry.file_name())
                    .display()
                    .to_string(),
            );
        }
    }
    None
}

fn run(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

// ---- Slot OS on the TrimUI Brick: the card's own FAT partition is the boot-resource partition
// u-boot reads, already mounted as the card, so the logo is the card's root bootlogo.bmp. TrimUI's
// u-boot shows 32-bit BMPs with a V5 header, rows bottom-up (LordNordyx's Slot-Brick found
// it so); a logo of any size or the usual 24-bit form is fitted and rewritten that way.

/// The Brick's panel.
pub const BRICK_PANEL: (usize, usize) = (1024, 768);
/// The Slot OS image's own logo, kept while a chosen one is shown, put back when it goes.
const SLOT_OS_ORIGINAL: &str = "bootlogo.slotos.bmp";

/// A 24- or 32-bit uncompressed BMP (BI_RGB, or BI_BITFIELDS with the usual masks), either way up.
pub fn decode_any(bmp: &[u8]) -> Option<Bmp> {
    let (width, height) = size(bmp)?;
    let bits = u16_at(bmp, 28)?;
    let compression = u32_at(bmp, 30)?;
    if !matches!((bits, compression), (24, 0) | (32, 0) | (32, 3)) {
        return None;
    }
    let top_down = (u32_at(bmp, 22)? as i32) < 0;
    let offset = u32_at(bmp, 10)? as usize;
    let px = usize::from(bits / 8);
    let stride = (width * px + 3) & !3;
    let data = bmp.get(offset..offset + stride * height)?;
    let mut rgb = Vec::with_capacity(width * height);
    for y in 0..height {
        let row = if top_down { y } else { height - 1 - y };
        for p in data[row * stride..].chunks_exact(px).take(width) {
            rgb.push([p[2], p[1], p[0]]);
        }
    }
    Some(Bmp { width, height, rgb })
}

/// As big as fits inside `panel`, centred on the picture's top-left colour; nearest-neighbour,
/// so pixel art stays sharp. A logo of exactly the panel's size is left as it is.
pub fn fit_to(img: &Bmp, (w, h): (usize, usize)) -> Option<Bmp> {
    if img.width == 0 || img.height == 0 {
        return None;
    }
    if (img.width, img.height) == (w, h) {
        return Some(Bmp { width: w, height: h, rgb: img.rgb.clone() });
    }
    let k = (w as f64 / img.width as f64).min(h as f64 / img.height as f64);
    let (dw, dh) = (
        ((img.width as f64 * k).round() as usize).clamp(1, w),
        ((img.height as f64 * k).round() as usize).clamp(1, h),
    );
    let (x0, y0) = ((w - dw) / 2, (h - dh) / 2);
    let mut rgb = vec![*img.rgb.first()?; w * h];
    for y in 0..dh {
        let sy = (y * img.height / dh).min(img.height - 1);
        for x in 0..dw {
            let sx = (x * img.width / dw).min(img.width - 1);
            rgb[(y0 + y) * w + x0 + x] = img.rgb[sy * img.width + sx];
        }
    }
    Some(Bmp { width: w, height: h, rgb })
}

/// 32-bit BMP, V5 header (BI_BITFIELDS, sRGB), rows bottom-up: the form TrimUI's u-boot shows.
pub fn encode_v5(img: &Bmp) -> Vec<u8> {
    let data = img.width * img.height * 4;
    let mut out = Vec::with_capacity(14 + 124 + data);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((14 + 124 + data) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(14u32 + 124).to_le_bytes());
    out.extend_from_slice(&124u32.to_le_bytes());
    out.extend_from_slice(&(img.width as i32).to_le_bytes());
    out.extend_from_slice(&(img.height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&(data as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
        out.extend_from_slice(&mask.to_le_bytes());
    }
    out.extend_from_slice(b"BGRs");
    out.extend_from_slice(&[0u8; 36]);
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 12]);
    for y in (0..img.height).rev() {
        for c in &img.rgb[y * img.width..(y + 1) * img.width] {
            out.extend_from_slice(&[c[2], c[1], c[0], 255]);
        }
    }
    out
}

/// The chosen logo (Config/bootlogo.bmp, from Boot Logo Studio or Gamebit) as the card's
/// bootlogo.bmp; with none chosen, the image's own logo back. Shown from the next start.
pub fn refresh_slot_os(card: &Path) -> io::Result<Outcome> {
    let target = card.join(LOGO);
    let original = card.join(SLOT_OS_ORIGINAL);
    let Ok(chosen) = fs::read(card.join(USER_LOGO)) else {
        if original.is_file() {
            let logo = fs::read(&original)?;
            replace(card, LOGO, &logo)?;
            fs::remove_file(&original)?;
            return Ok(Outcome::Installed);
        }
        return Ok(Outcome::Same);
    };
    let Some(img) = decode_any(&chosen).and_then(|b| fit_to(&b, BRICK_PANEL)) else {
        return Ok(Outcome::Invalid);
    };
    let logo = encode_v5(&img);
    let current = fs::read(&target).ok();
    if current.as_deref() == Some(&logo[..]) {
        return Ok(Outcome::Same);
    }
    if let Some(current) = current.filter(|_| !original.is_file()) {
        replace(card, SLOT_OS_ORIGINAL, &current)?;
    }
    replace(card, LOGO, &logo)?;
    Ok(Outcome::Installed)
}
