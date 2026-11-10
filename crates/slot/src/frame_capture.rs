//! A debugging aid for new hardware: with an empty `Logs/capture-frame` on the card, the
//! next frame drawn while a game is shown is saved as PNGs in `Logs/`, then the trigger is
//! removed:
//! - `frame-canvas.png`: slot's own frame, before it is put on the panel
//! - `frame-window.png`: that frame as slot puts it on the panel
//! - `frame-fb0-<n>.png`: what the display actually holds, read back from /dev/fb0 (each
//!   page of its virtual screen)
use std::path::Path;

const TRIGGER: &str = "Logs/capture-frame";

pub fn wanted(root: &Path) -> bool {
    root.join(TRIGGER).is_file()
}

pub fn done(root: &Path) {
    let _ = std::fs::remove_file(root.join(TRIGGER));
}

pub fn save_rgba(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    let px = (w * h) as usize;
    if rgba.len() < px * 4 {
        eprintln!("slot: capture {}: short frame", path.display());
        return;
    }
    let rgb: Vec<u8> = rgba[..px * 4]
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let encoded = enc.write_header().and_then(|mut wr| {
        wr.write_image_data(&rgb)?;
        wr.finish()
    });
    match encoded.map_err(std::io::Error::other).and_then(|_| std::fs::write(path, &out)) {
        Ok(()) => eprintln!("slot: capture: wrote {}", path.display()),
        Err(e) => eprintln!("slot: capture {}: {e}", path.display()),
    }
}

/// Every page of /dev/fb0's virtual screen, as 32-bit pixels in the byte order the panel's
/// sysfs says (BGRA on the Allwinner display engines): each one a PNG.
pub fn save_fb0(root: &Path) {
    let attr = |name: &str| std::fs::read_to_string(format!("/sys/class/graphics/fb0/{name}")).ok();
    let pair = |s: Option<String>| {
        let s = s?;
        let (a, b) = s.trim().split_once(',')?;
        Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))
    };
    let Some((vw, vh)) = pair(attr("virtual_size")) else {
        eprintln!("slot: capture: no fb0 virtual_size");
        return;
    };
    let bpp = attr("bits_per_pixel")
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(32);
    let stride = attr("stride")
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(vw * bpp / 8);
    eprintln!(
        "slot: capture: fb0 virtual {vw}x{vh}, {bpp} bpp, stride {stride}, pan {:?}",
        attr("pan").map(|s| s.trim().to_string())
    );
    if bpp != 32 {
        return;
    }
    let (pw, ph) = pair(attr("mode").and_then(|m| {
        let m = m.trim().trim_start_matches(|c: char| !c.is_ascii_digit()).to_string();
        let dims = m.split(['p', 'i', '-']).next()?.replace('x', ",");
        Some(dims)
    }))
    .unwrap_or((vw, vw * 3 / 4));
    let Ok(raw) = std::fs::read("/dev/fb0") else {
        eprintln!("slot: capture: cannot read /dev/fb0");
        return;
    };
    let page_bytes = (stride * ph) as usize;
    let pages = (raw.len() / page_bytes.max(1)).min((vh / ph.max(1)).max(1) as usize).min(4);
    for n in 0..pages {
        let page = &raw[n * page_bytes..(n + 1) * page_bytes];
        let mut rgba = Vec::with_capacity((pw * ph * 4) as usize);
        for y in 0..ph as usize {
            let row = &page[y * stride as usize..];
            for x in 0..pw as usize {
                let p = &row[x * 4..x * 4 + 4];
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
        }
        save_rgba(&root.join(format!("Logs/frame-fb0-{n}.png")), pw, ph, &rgba);
    }
}
