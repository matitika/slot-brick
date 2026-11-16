use std::path::Path;
use std::sync::OnceLock;

use slot_gfx::{Canvas, Draw, TexId, OUT_H, OUT_W};

use crate::art;

pub const SLOT_WALLPAPER_PNG: &[u8] = include_bytes!("../../../assets/background_default.png");
const DEFAULT_PNG: &[u8] = SLOT_WALLPAPER_PNG;
const BEZEL_GB_PNG: &[u8] = include_bytes!("../../../assets/bezel_gb.png");
const BEZEL_GBC_PNG: &[u8] = include_bytes!("../../../assets/bezel_gbc.png");
const BEZEL_GB_FIT_3X2_PNG: &[u8] = include_bytes!("../../../assets/bezel_gb_fit_3x2.png");
const BEZEL_GBC_FIT_3X2_PNG: &[u8] = include_bytes!("../../../assets/bezel_gbc_fit_3x2.png");
const BEZEL_GB_INTEGER_4X3_PNG: &[u8] = include_bytes!("../../../assets/bezel_gb_integer_4x3.png");
const BEZEL_GBC_INTEGER_4X3_PNG: &[u8] =
    include_bytes!("../../../assets/bezel_gbc_integer_4x3.png");
const BEZEL_GB_FIT_4X3_PNG: &[u8] = include_bytes!("../../../assets/bezel_gb_fit_4x3.png");
const BEZEL_GBC_FIT_4X3_PNG: &[u8] = include_bytes!("../../../assets/bezel_gbc_fit_4x3.png");
pub const SLOT_BLUR_WALLPAPER_PNG: &[u8] =
    include_bytes!("../../../assets/background_default_blur.png");
const BLUR_PNG: &[u8] = SLOT_BLUR_WALLPAPER_PNG;

const SCRIM: f32 = 0.62;

pub fn wallpaper_face(path: &Path, w: u32, h: u32) -> Option<Vec<u8>> {
    art::cover(path, w, h)
}

pub fn bezel_face(path: &Path, w: u32, h: u32) -> Option<Vec<u8>> {
    art::cover(path, w, h)
}

pub fn game_bezel_face(path: &Path, w: u32, h: u32) -> Option<Vec<u8>> {
    let (src, sw, sh) = art::decode(path)?;
    panel_bezel(&src, (sw, sh), (w, h), false)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BezelArt {
    Default,
    Fit3x2,
    Integer4x3,
    Fit4x3,
}

pub fn default_game_bezel_face(colour: bool, kind: BezelArt, w: u32, h: u32) -> Option<Vec<u8>> {
    let png = match (kind, colour) {
        (BezelArt::Default, false) => BEZEL_GB_PNG,
        (BezelArt::Default, true) => BEZEL_GBC_PNG,
        (BezelArt::Fit3x2, false) => BEZEL_GB_FIT_3X2_PNG,
        (BezelArt::Fit3x2, true) => BEZEL_GBC_FIT_3X2_PNG,
        (BezelArt::Integer4x3, false) => BEZEL_GB_INTEGER_4X3_PNG,
        (BezelArt::Integer4x3, true) => BEZEL_GBC_INTEGER_4X3_PNG,
        (BezelArt::Fit4x3, false) => BEZEL_GB_FIT_4X3_PNG,
        (BezelArt::Fit4x3, true) => BEZEL_GBC_FIT_4X3_PNG,
    };
    let (src, sw, sh) = art::decode_png_bytes(png)?;
    if (w, h) == BRICK_PANEL {
        return brick_bezel(&src, (sw, sh), kind);
    }
    match kind {
        BezelArt::Default => panel_bezel(&src, (sw, sh), (w, h), true),
        _ => art::cover_rgba(&src, sw, sh, w, h),
    }
}

const BEZEL_GRAIN_ROWS: usize = 10;

/// The 1024x768 panel (the TrimUI Brick), where Game Boy is laid out from slot-gfx's table:
/// Integer at 5x (800x720 at 112,24), Fit at 853x768, Band in a 960x640 band at 32,64.
const BRICK_PANEL: (u32, u32) = (1024, 768);
const BRICK_BAND: (u32, u32, u32, u32) = (32, 64, 960, 640);

/// The 640x480 (and 720x480 band) shell art made to sit around the Brick's game: scaled so its
/// window lands on the picture, the grain carried out to the panel's edges where it falls short.
fn brick_bezel(src: &[u8], (sw, sh): (u32, u32), kind: BezelArt) -> Option<Vec<u8>> {
    let (pw, ph) = BRICK_PANEL;
    match kind {
        // the window 480x432 at 80,24 becomes 800x720 at 112,24: 5/3, then the middle
        BezelArt::Integer4x3 => {
            let (bw, bh) = (sw * 5 / 3, sh * 5 / 3);
            let big = art::cover_rgba(src, sw, sh, bw, bh)?;
            let mut out = crop(&big, bw, (bw - pw) / 2, (bh - ph) / 2, pw, ph);
            // the crop takes the edge off the power LED and its label: set them back from
            // the art at 1.6, which fits the plastic left of the window
            let small = art::cover_rgba(src, sw, sh, pw, ph)?;
            lend(&mut out, &small, pw, LED_BLOCK);
            Some(out)
        }
        BezelArt::Default => {
            let (bx, by, bw, bh) = BRICK_BAND;
            let band = art::cover_rgba(src, sw, sh, bw, bh)?;
            Some(extend(&band, (bw, bh), (bx, by), (pw, ph)))
        }
        // Fit4x3: the window 533x480 becomes 853x768 at 1.6; Fit3x2 is not laid out here
        _ => art::cover_rgba(src, sw, sh, pw, ph),
    }
}

/// Where the power LED and its label sit on the Brick, and how far they shift left: the block
/// (x, y, w, h) taken from the art at 1.6 and laid `shift` pixels left, its edges feathered.
const LED_BLOCK: (u32, u32, u32, u32, u32) = (0, 225, 106, 120, 8);
const FEATHER: f32 = 10.0;

fn lend(out: &mut [u8], from: &[u8], w: u32, (bx, by, bw, bh, shift): (u32, u32, u32, u32, u32)) {
    for y in by..by + bh {
        for x in bx + shift..bx + bw {
            let to_x = x - shift;
            let edge = [
                (y - by) as f32,
                (by + bh - 1 - y) as f32,
                (bx + bw - 1 - x) as f32,
            ];
            let k = edge
                .iter()
                .fold(1.0f32, |k, d| k.min(d / FEATHER))
                .clamp(0.0, 1.0);
            let (s, d) = (((y * w + x) * 4) as usize, ((y * w + to_x) * 4) as usize);
            for c in 0..4 {
                let v = from[s + c] as f32 * k + out[d + c] as f32 * (1.0 - k);
                out[d + c] = v.round() as u8;
            }
        }
    }
}

fn crop(src: &[u8], sw: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let from = ((row * sw + x) * 4) as usize;
        out.extend_from_slice(&src[from..from + (w * 4) as usize]);
    }
    out
}

/// `inner` placed at `at` on a `size` panel, the rows and columns around it mirrored out from
/// its edge grain (a few pixels, back and forth) so the plastic carries on unbroken.
fn extend(inner: &[u8], (iw, ih): (u32, u32), (ax, ay): (u32, u32), (w, h): (u32, u32)) -> Vec<u8> {
    let ping = |d: u32, n: u32| {
        let span = (BEZEL_GRAIN_ROWS as u32).min(n);
        let p = d % (2 * span);
        p.min(2 * span - 1 - p)
    };
    let source = |v: i64, start: u32, len: u32| -> u32 {
        let v = v - start as i64;
        if v < 0 {
            ping((-v - 1) as u32, len)
        } else if v >= len as i64 {
            len - 1 - ping((v - len as i64) as u32, len)
        } else {
            v as u32
        }
    };
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let sy = source(y as i64, ay, ih);
        for x in 0..w {
            let sx = source(x as i64, ax, iw);
            let (from, to) = (((sy * iw + sx) * 4) as usize, ((y * w + x) * 4) as usize);
            out[to..to + 4].copy_from_slice(&inner[from..from + 4]);
        }
    }
    out
}

fn panel_bezel(
    src: &[u8],
    (sw, sh): (u32, u32),
    (w, h): (u32, u32),
    extend: bool,
) -> Option<Vec<u8>> {
    if !slot_gfx::framed((w, h)) || sw * 3 == sh * 4 {
        return art::cover_rgba(src, sw, sh, w, h);
    }
    let game_h = slot_gfx::screen_rect((w, h)).3 as usize;
    let inner = art::cover_rgba(src, sw, sh, w, game_h as u32)?;
    let (h, row) = (h as usize, w as usize * 4);
    let gap = h - game_h;
    let top = gap - gap / 2;
    let mut out = vec![0u8; row * h];
    out[top * row..(top + game_h) * row].copy_from_slice(&inner);
    if !extend {
        return Some(out);
    }
    let ping = |d: usize| {
        let p = d % (2 * BEZEL_GRAIN_ROWS);
        p.min(2 * BEZEL_GRAIN_ROWS - 1 - p)
    };
    for d in 0..top {
        let from = ping(d) * row;
        out[(top - 1 - d) * row..][..row].copy_from_slice(&inner[from..from + row]);
    }
    for d in 0..h - top - game_h {
        let from = (game_h - 1 - ping(d)) * row;
        out[(top + game_h + d) * row..][..row].copy_from_slice(&inner[from..from + row]);
    }
    Some(out)
}

/// The carousel ground, scaled to cover the whole screen, including behind the footer.
pub fn default_background() -> Option<&'static [u8]> {
    static P: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    P.get_or_init(|| art::cover_bytes(DEFAULT_PNG, OUT_W, OUT_H))
        .as_deref()
}

/// The blurred carousel ground, scaled to cover the whole screen the same way as the default.
pub fn blur_background() -> Option<&'static [u8]> {
    static P: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    P.get_or_init(|| art::cover_bytes(BLUR_PNG, OUT_W, OUT_H))
        .as_deref()
}

pub fn reference_ground_covers((w, h): (u32, u32)) -> bool {
    h == OUT_H && w <= OUT_W
}

pub fn default_background_at(w: u32, h: u32) -> Option<Vec<u8>> {
    art::cover_bytes(DEFAULT_PNG, w, h)
}

pub fn blur_background_at(w: u32, h: u32) -> Option<Vec<u8>> {
    art::cover_bytes(BLUR_PNG, w, h)
}

pub fn draw_ground(canvas: Canvas, face: Option<(TexId, u32)>, out: &mut Vec<Draw>) {
    let Some(face) = face else {
        return;
    };
    out.push(ground(canvas, face));
}

pub fn draw_backdrop(canvas: Canvas, face: Option<(TexId, u32)>, out: &mut Vec<Draw>) {
    let Some(face) = face else {
        return;
    };
    out.push(ground(canvas, face));
    out.push(Draw::Rect {
        x: 0.0,
        y: 0.0,
        w: canvas.w,
        h: canvas.h,
        colour: [0.0, 0.0, 0.0, SCRIM],
    });
}

fn ground(canvas: Canvas, (tex, tex_w): (TexId, u32)) -> Draw {
    Draw::Part {
        x: 0.0,
        y: 0.0,
        w: canvas.w,
        h: canvas.h,
        tex,
        alpha: 1.0,
        uv: canvas.cover(tex_w as f32),
    }
}
