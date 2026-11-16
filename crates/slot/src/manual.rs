//! A game's manual: page images in a folder, or zipped as a .cbz, read one page at a time so a
//! long scanned booklet never sits in memory whole (slot_store::find_manual finds it).

use std::io::Read;
use std::path::{Path, PathBuf};

/// Pages larger than this on their longest side are shrunk as they are read: plenty for the
/// widest zoom on a 1024-pixel screen, and a 6000-pixel scan would otherwise take 100 MB.
const MAX_SIDE: u32 = 2400;
/// A page file larger than this is not read.
const MAX_BYTES: u64 = 32 << 20;

use slot_gfx::Canvas;
use slot_ui::ManualPage;

type Image = (Vec<u8>, u32, u32);

#[derive(Clone)]
enum Pages {
    Folder(Vec<PathBuf>),
    Zipped(PathBuf, Vec<String>),
}

pub struct Manual {
    pages: Pages,
    /// the page read ahead on its own thread while this one is being read
    ahead: Option<(usize, std::thread::JoinHandle<ManualPage>)>,
}

fn is_page(name: &str) -> bool {
    let lower = name.to_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    !file.starts_with('.')
        && !lower.starts_with("__macosx/")
        && [".png", ".jpg", ".jpeg"].iter().any(|e| file.ends_with(e))
}

/// "page2" before "page10": runs of digits compare as numbers.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.to_lowercase().chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((d, run)) if *d == digit => run.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    }
    let (pa, pb) = (parts(a), parts(b));
    for (x, y) in pa.iter().zip(&pb) {
        let ord = match (x.0, y.0) {
            (true, true) => {
                let (tx, ty) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
                tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty))
            }
            _ => x.1.cmp(&y.1),
        };
        if ord.is_ne() {
            return ord;
        }
    }
    pa.len().cmp(&pb.len()).then_with(|| a.cmp(b))
}

impl Manual {
    pub fn open(path: &Path) -> Option<Manual> {
        let pages = match path.is_dir() {
            true => {
                let mut files: Vec<PathBuf> = std::fs::read_dir(path)
                    .ok()?
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_file())
                    .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(is_page))
                    .collect();
                files.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
                Pages::Folder(files)
            }
            false => {
                let file = std::fs::File::open(path).ok()?;
                let archive = zip::ZipArchive::new(file).ok()?;
                let mut names: Vec<String> = archive
                    .file_names()
                    .filter(|n| !n.ends_with('/') && is_page(n))
                    .map(str::to_owned)
                    .collect();
                names.sort_by(|a, b| natural_cmp(a, b));
                Pages::Zipped(path.to_path_buf(), names)
            }
        };
        let m = Manual { pages, ahead: None };
        (!m.is_empty()).then_some(m)
    }

    pub fn len(&self) -> usize {
        match &self.pages {
            Pages::Folder(f) => f.len(),
            Pages::Zipped(_, n) => n.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Starts reading `page` and scaling it for `level` on its own thread, so turning to it
    /// is instant.
    pub fn read_ahead(&mut self, page: usize, level: u8, canvas: Canvas) {
        if page >= self.len() || self.ahead.as_ref().is_some_and(|(p, _)| *p == page) {
            return;
        }
        let (pages, count) = (self.pages.clone(), self.len());
        let spawned = std::thread::Builder::new()
            .name("slot-manual".into())
            .spawn(move || {
                ManualPage::new(page, count, load(&pages, page), level, 0, false, canvas)
            });
        self.ahead = spawned.ok().map(|h| (page, h));
    }

    /// `page` ready to show: the one read ahead when it is that page, else read now.
    pub fn show(
        &mut self,
        page: usize,
        level: u8,
        y: u32,
        at_bottom: bool,
        canvas: Canvas,
    ) -> ManualPage {
        if let Some((p, handle)) = self.ahead.take() {
            if p == page {
                if let Ok(ready) = handle.join() {
                    return ready.reshown(level, y, at_bottom, canvas);
                }
            }
        }
        let count = self.len();
        ManualPage::new(
            page,
            count,
            load(&self.pages, page),
            level,
            y,
            at_bottom,
            canvas,
        )
    }
}

fn load(pages: &Pages, page: usize) -> Option<Image> {
    let decoded = bytes(pages, page).and_then(|b| decode(&b));
    if decoded.is_none() {
        eprintln!("slot: manual: page {} would not decode", page + 1);
    }
    let (rgba, w, h) = decoded?;
    let longest = w.max(h);
    if longest <= MAX_SIDE {
        return Some((rgba, w, h));
    }
    let s = MAX_SIDE as f32 / longest as f32;
    let (dw, dh) = (
        ((w as f32 * s).round() as u32).max(1),
        ((h as f32 * s).round() as u32).max(1),
    );
    Some((slot_ui::resize(&rgba, w, h, dw, dh), dw, dh))
}

fn bytes(pages: &Pages, page: usize) -> Option<Vec<u8>> {
    {
        let mut bytes = Vec::new();
        match pages {
            Pages::Folder(files) => {
                let file = std::fs::File::open(files.get(page)?).ok()?;
                file.take(MAX_BYTES).read_to_end(&mut bytes).ok()?;
            }
            Pages::Zipped(path, names) => {
                let file = std::fs::File::open(path).ok()?;
                let mut archive = zip::ZipArchive::new(file).ok()?;
                let entry = archive.by_name(names.get(page)?).ok()?;
                entry.take(MAX_BYTES).read_to_end(&mut bytes).ok()?;
            }
        }
        Some(bytes)
    }
}

/// PNG or JPEG, by their first bytes, to opaque RGBA (see-through parts on white paper).
pub fn decode(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let (mut rgba, w, h) = match bytes {
        [0x89, b'P', b'N', b'G', ..] => slot_ui::decode_png_bytes(bytes)?,
        [0xFF, 0xD8, ..] => decode_jpeg(bytes)?,
        _ => return None,
    };
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
            }
            px[3] = 255;
        }
    }
    Some((rgba, w, h))
}

fn decode_jpeg(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut dec = jpeg_decoder::Decoder::new(bytes);
    let pixels = dec.decode().ok()?;
    let info = dec.info()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let n = (w * h) as usize;
    let mut rgba = vec![255u8; n * 4];
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            for (o, i) in rgba.chunks_exact_mut(4).zip(pixels.chunks_exact(3)) {
                o[..3].copy_from_slice(i);
            }
        }
        jpeg_decoder::PixelFormat::L8 => {
            for (o, &g) in rgba.chunks_exact_mut(4).zip(&pixels) {
                o[..3].fill(g);
            }
        }
        jpeg_decoder::PixelFormat::L16 => {
            for (o, g) in rgba.chunks_exact_mut(4).zip(pixels.chunks_exact(2)) {
                o[..3].fill(g[0]);
            }
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            for (o, p) in rgba.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
                let k = 255 - p[3] as u32;
                for c in 0..3 {
                    o[c] = ((255 - p[c] as u32) * k / 255) as u8;
                }
            }
        }
    }
    (pixels.len() >= n).then_some((rgba, w, h))
}
