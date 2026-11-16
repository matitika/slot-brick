//! A manual's page in the guide viewer: one decoded image, shown fit to the screen or wider,
//! scrolled and panned like a text guide. The pages themselves are read by slot (manual.rs).

use slot_gfx::Canvas;

use crate::guide::page_box;

/// Zoom levels, as how wide the page is against the viewer's width: the whole page, then fit
/// to width (the default), then wider to read small print.
pub const MANUAL_ZOOMS: usize = 4;
pub const MANUAL_FIT_WIDTH: u8 = 1;
const WIDER: [f32; 2] = [1.5, 2.0];
/// Never blown up past this, however small the scan.
const MAX_SCALE: f32 = 3.0;
/// Up/Down move a sixth of the view, Left/Right a quarter.
const SCROLL_PARTS: u32 = 6;
const PAN_PARTS: u32 = 4;

/// Where a press should take the viewer when it runs off the page: the page to load, and
/// whether to land at its bottom (going back up) rather than its top.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Turn {
    pub page: usize,
    pub at_bottom: bool,
}

pub struct ManualPage {
    page: usize,
    pages: usize,
    src: (Vec<u8>, u32, u32),
    level: u8,
    scaled: Vec<u8>,
    sw: u32,
    sh: u32,
    x: u32,
    y: u32,
}

impl ManualPage {
    /// The page, or a blank one when it would not decode.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        page: usize,
        pages: usize,
        image: Option<(Vec<u8>, u32, u32)>,
        level: u8,
        y: u32,
        at_bottom: bool,
        canvas: Canvas,
    ) -> Self {
        let src = image
            .filter(|(rgba, w, h)| *w > 0 && *h > 0 && rgba.len() == (*w * *h * 4) as usize)
            .unwrap_or_else(|| (vec![0, 0, 0, 255], 1, 1));
        let mut p = ManualPage {
            page,
            pages: pages.max(1),
            src,
            level: level.min(MANUAL_ZOOMS as u8 - 1),
            scaled: Vec::new(),
            sw: 0,
            sh: 0,
            x: 0,
            y,
        };
        p.rescale(canvas);
        if at_bottom {
            p.y = u32::MAX;
        }
        p.clamp(canvas);
        p
    }

    /// The same page shown again at `level`, from its top or bottom: rescaled only when the
    /// zoom changed (a page read ahead was scaled for the zoom of its time).
    pub fn reshown(mut self, level: u8, y: u32, at_bottom: bool, canvas: Canvas) -> Self {
        let level = level.min(MANUAL_ZOOMS as u8 - 1);
        if level != self.level {
            self.level = level;
            self.rescale(canvas);
        }
        self.x = 0;
        self.y = match at_bottom {
            true => u32::MAX,
            false => y,
        };
        self.clamp(canvas);
        self
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn pages(&self) -> usize {
        self.pages
    }

    pub fn level(&self) -> u8 {
        self.level
    }

    pub fn offset(&self) -> (u32, u32) {
        (self.x, self.y)
    }

    pub fn scaled_size(&self) -> (u32, u32) {
        (self.sw, self.sh)
    }

    fn scale(&self, canvas: Canvas) -> f32 {
        let (_, _, bw, bh) = page_box(canvas);
        let (w, h) = (self.src.1 as f32, self.src.2 as f32);
        let width = bw as f32 / w;
        let s = match self.level {
            0 => width.min(bh as f32 / h),
            1 => width,
            l => width * WIDER[(l as usize - 2).min(WIDER.len() - 1)],
        };
        s.min(MAX_SCALE)
    }

    fn rescale(&mut self, canvas: Canvas) {
        let s = self.scale(canvas);
        let (src, w, h) = (&self.src.0, self.src.1, self.src.2);
        self.sw = ((w as f32 * s).round() as u32).max(1);
        self.sh = ((h as f32 * s).round() as u32).max(1);
        self.scaled = resize(src, w, h, self.sw, self.sh);
    }

    fn room(&self, canvas: Canvas) -> (u32, u32) {
        let (_, _, bw, bh) = page_box(canvas);
        (self.sw.saturating_sub(bw), self.sh.saturating_sub(bh))
    }

    fn clamp(&mut self, canvas: Canvas) {
        let (rx, ry) = self.room(canvas);
        self.x = self.x.min(rx);
        self.y = self.y.min(ry);
    }

    /// Up/Down: within the page, then onto the next or previous one at its edge.
    pub fn scroll(&mut self, by: i32, canvas: Canvas) -> Option<Turn> {
        let (_, _, _, bh) = page_box(canvas);
        let (_, ry) = self.room(canvas);
        let step = (bh / SCROLL_PARTS).max(1);
        match by.signum() {
            1 if self.y >= ry => self.turn(1),
            -1 if self.y == 0 => self.turn(-1),
            1 => {
                self.y = (self.y + step).min(ry);
                None
            }
            -1 => {
                self.y = self.y.saturating_sub(step);
                None
            }
            _ => None,
        }
    }

    /// Left/Right: across a page wider than the view, else to the previous or next page.
    pub fn pan(&mut self, by: i32, canvas: Canvas) -> Option<Turn> {
        let (_, _, bw, _) = page_box(canvas);
        let (rx, _) = self.room(canvas);
        if rx == 0 {
            return self.turn(by.signum() as isize).map(|t| Turn {
                at_bottom: false,
                ..t
            });
        }
        let step = (bw / PAN_PARTS).max(1);
        match by.signum() {
            1 => self.x = (self.x + step).min(rx),
            -1 => self.x = self.x.saturating_sub(step),
            _ => {}
        }
        None
    }

    /// L1/R1: the previous or next page, from its top.
    pub fn turn(&self, by: isize) -> Option<Turn> {
        let to = self.page.checked_add_signed(by)?;
        (to < self.pages && to != self.page).then_some(Turn {
            page: to,
            at_bottom: by < 0,
        })
    }

    pub fn zoom_by(&mut self, by: i8, canvas: Canvas) -> bool {
        let to = (self.level as i16 + by as i16).clamp(0, MANUAL_ZOOMS as i16 - 1) as u8;
        if to == self.level {
            return false;
        }
        let (_, _, bw, bh) = page_box(canvas);
        // keep the middle of the view where it was
        let (cx, cy) = (
            (self.x + bw / 2) as f32 / self.sw as f32,
            (self.y + bh / 2) as f32 / self.sh as f32,
        );
        self.level = to;
        self.rescale(canvas);
        self.x = ((cx * self.sw as f32) as u32).saturating_sub(bw / 2);
        self.y = ((cy * self.sh as f32) as u32).saturating_sub(bh / 2);
        self.clamp(canvas);
        true
    }

    /// The view, w x h, cut from the scaled page, centred where the page is the smaller.
    pub fn render(&self, w: u32, h: u32) -> Vec<u8> {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let (ox, oy) = (w.saturating_sub(self.sw) / 2, h.saturating_sub(self.sh) / 2);
        let cw = self.sw.saturating_sub(self.x).min(w - ox) as usize;
        let rows = self.sh.saturating_sub(self.y).min(h - oy);
        for r in 0..rows {
            let from = (((self.y + r) * self.sw + self.x) * 4) as usize;
            let to = (((oy + r) * w + ox) * 4) as usize;
            rgba[to..to + cw * 4].copy_from_slice(&self.scaled[from..from + cw * 4]);
        }
        rgba
    }
}

/// Weights along one axis, in 1/16384ths: for each output pixel, the first source pixel and
/// a run of weights from it into `w`. Shrinking averages every source pixel under the output
/// one (so small print stays legible rather than shimmering); growing blends the two nearest.
struct Axis {
    taps: Vec<(u32, u32, u32)>,
    w: Vec<u32>,
}

const ONE: f32 = 16384.0;

fn axis(src: u32, dst: u32) -> Axis {
    let s = dst as f32 / src as f32;
    let mut taps = Vec::with_capacity(dst as usize);
    let mut w = Vec::new();
    for o in 0..dst {
        let (first, run): (usize, Vec<f32>) = if s < 1.0 {
            let (a, b) = (o as f32 / s, (o + 1) as f32 / s);
            let first = a.floor() as usize;
            let last = (b.ceil() as usize).min(src as usize).max(first + 1);
            let run: Vec<f32> = (first..last)
                .map(|i| ((i + 1) as f32).min(b) - (i as f32).max(a))
                .collect();
            (first, run)
        } else {
            let c = ((o as f32 + 0.5) / s - 0.5).max(0.0);
            let i = (c.floor() as usize).min(src as usize - 1);
            let f = c - i as f32;
            match i + 1 < src as usize {
                true => (i, vec![1.0 - f, f]),
                false => (i, vec![1.0]),
            }
        };
        let sum: f32 = run.iter().sum::<f32>().max(f32::EPSILON);
        taps.push((first as u32, w.len() as u32, run.len() as u32));
        w.extend(run.iter().map(|x| (x / sum * ONE).round() as u32));
    }
    Axis { taps, w }
}

/// RGBA w x h to dw x dh, opaque: across each row, then down each column.
pub fn resize(src: &[u8], w: u32, h: u32, dw: u32, dh: u32) -> Vec<u8> {
    if (w, h) == (dw, dh) {
        return src.to_vec();
    }
    let (xs, ys) = (axis(w, dw), axis(h, dh));
    let (w, dw) = (w as usize, dw as usize);
    let mut mid = vec![0u8; dw * h as usize * 3];
    for (row, out) in src.chunks_exact(w * 4).zip(mid.chunks_exact_mut(dw * 3)) {
        for (o, &(first, at, n)) in out.chunks_exact_mut(3).zip(&xs.taps) {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            let px = &row[first as usize * 4..];
            for (k, &wt) in xs.w[at as usize..(at + n) as usize].iter().enumerate() {
                r += px[k * 4] as u32 * wt;
                g += px[k * 4 + 1] as u32 * wt;
                b += px[k * 4 + 2] as u32 * wt;
            }
            o[0] = ((r + 8192) >> 14).min(255) as u8;
            o[1] = ((g + 8192) >> 14).min(255) as u8;
            o[2] = ((b + 8192) >> 14).min(255) as u8;
        }
    }
    let mut out = vec![255u8; dw * dh as usize * 4];
    let mut acc = vec![0u32; dw * 3];
    for (line, &(first, at, n)) in out.chunks_exact_mut(dw * 4).zip(&ys.taps) {
        acc.fill(0);
        for (k, &wt) in ys.w[at as usize..(at + n) as usize].iter().enumerate() {
            let from = (first as usize + k) * dw * 3;
            for (a, &v) in acc.iter_mut().zip(&mid[from..from + dw * 3]) {
                *a += v as u32 * wt;
            }
        }
        for (o, a) in line.chunks_exact_mut(4).zip(acc.chunks_exact(3)) {
            o[0] = ((a[0] + 8192) >> 14).min(255) as u8;
            o[1] = ((a[1] + 8192) >> 14).min(255) as u8;
            o[2] = ((a[2] + 8192) >> 14).min(255) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Canvas {
        Canvas {
            w: 1024.0,
            h: 768.0,
        }
    }

    fn sheet(w: u32, h: u32) -> Option<(Vec<u8>, u32, u32)> {
        Some((vec![200; (w * h * 4) as usize], w, h))
    }

    #[test]
    fn halving_averages_and_doubling_keeps_flat_colour() {
        let src = [0, 0, 0, 255, 255, 255, 255, 255];
        assert_eq!(resize(&src, 2, 1, 1, 1)[..3], [128, 128, 128]);
        let flat = vec![90u8; 3 * 3 * 4];
        assert!(resize(&flat, 3, 3, 7, 5)
            .chunks(4)
            .all(|p| p == [90, 90, 90, 255]));
    }

    #[test]
    fn a_tall_page_scrolls_down_then_turns_to_the_next() {
        let mut p = ManualPage::new(0, 3, sheet(500, 1400), MANUAL_FIT_WIDTH, 0, false, canvas());
        let mut turned = None;
        for _ in 0..100 {
            turned = p.scroll(1, canvas());
            if turned.is_some() {
                break;
            }
        }
        assert_eq!(
            turned,
            Some(Turn {
                page: 1,
                at_bottom: false
            })
        );
        assert_eq!(p.scroll(-1, canvas()), None);
    }

    #[test]
    fn going_back_from_a_pages_top_lands_at_the_previous_bottom() {
        let mut p = ManualPage::new(1, 3, sheet(500, 1400), MANUAL_FIT_WIDTH, 0, false, canvas());
        let back = p.scroll(-1, canvas()).unwrap();
        assert_eq!(back.page, 0);
        let prev = ManualPage::new(0, 3, sheet(500, 1400), MANUAL_FIT_WIDTH, 0, true, canvas());
        let (_, _, _, bh) = page_box(canvas());
        assert_eq!(prev.offset().1, prev.scaled_size().1 - bh);
    }

    #[test]
    fn left_and_right_turn_pages_until_zoomed_wider_than_the_view() {
        let mut p = ManualPage::new(0, 2, sheet(500, 700), MANUAL_FIT_WIDTH, 0, false, canvas());
        assert_eq!(p.pan(1, canvas()).map(|t| t.page), Some(1));
        assert_eq!(p.pan(-1, canvas()), None);
        assert!(p.zoom_by(1, canvas()));
        assert_eq!(p.pan(1, canvas()), None);
        assert!(p.offset().0 > 0);
    }

    #[test]
    fn the_whole_page_fits_the_view_and_renders_centred() {
        let p = ManualPage::new(0, 1, sheet(500, 700), 0, 0, false, canvas());
        let (_, _, bw, bh) = page_box(canvas());
        let (sw, sh) = p.scaled_size();
        assert!(sw <= bw && sh <= bh);
        let out = p.render(bw, bh);
        let mid = ((bh / 2 * bw + bw / 2) * 4) as usize;
        assert_eq!(out[mid], 200);
        assert_eq!(out[(bh / 2 * bw * 4) as usize], 0);
    }

    #[test]
    fn a_page_that_will_not_decode_is_blank_not_a_panic() {
        let p = ManualPage::new(0, 1, None, 1, 0, false, canvas());
        let (_, _, bw, bh) = page_box(canvas());
        assert_eq!(p.render(bw, bh).len(), (bw * bh * 4) as usize);
        assert_eq!(p.turn(1), None);
    }
}
