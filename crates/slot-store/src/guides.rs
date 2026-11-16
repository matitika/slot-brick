use std::path::{Path, PathBuf};

use crate::config::CONFIG_DIR;
use crate::platform::Platform;
use crate::scan::listing;

pub const GUIDES_DIR: &str = "Guides";
pub const GUIDE_SPOTS_FILE: &str = "guides.txt";
pub const MANUALS_DIR: &str = "Manuals";
/// What a manual can be: a folder of page images, or the same zipped (a comic-book .cbz).
const MANUAL_ZIPS: [&str; 2] = ["cbz", "zip"];

const TAB: usize = 8;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct GuideSpot {
    pub line: u32,
    pub col: u32,
    pub zoom: u8,
    pub wrap: bool,
}

pub fn find_guide(root: &Path, platform: Platform, stem: &str) -> Option<PathBuf> {
    let dir = root.join(GUIDES_DIR).join(platform.dir_name());
    listing(&dir)
        .get(&format!("{stem}.txt").to_lowercase())
        .map(|name| dir.join(name))
}

/// A game's manual: Manuals/<platform>/<game name>/ holding its pages as PNG or JPEG images,
/// or <game name>.cbz / .zip holding them, named so they sort in page order.
pub fn find_manual(root: &Path, platform: Platform, stem: &str) -> Option<PathBuf> {
    let dir = root.join(MANUALS_DIR).join(platform.dir_name());
    let names = listing(&dir);
    let stem = stem.to_lowercase();
    if let Some(name) = names.get(&stem) {
        let path = dir.join(name);
        if path.is_dir() {
            return Some(path);
        }
    }
    MANUAL_ZIPS
        .iter()
        .find_map(|ext| names.get(&format!("{stem}.{ext}")))
        .map(|name| dir.join(name))
        .filter(|p| p.is_file())
}

pub fn read_guide(path: &Path) -> std::io::Result<Vec<String>> {
    Ok(decode_guide(&std::fs::read(path)?))
}

pub fn decode_guide(bytes: &[u8]) -> Vec<String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    text.lines().map(expand).collect()
}

fn expand(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut col = 0;
    for ch in line.chars() {
        match ch {
            '\t' => {
                let to = (col / TAB + 1) * TAB;
                out.extend(std::iter::repeat_n(' ', to - col));
                col = to;
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                col += 1;
            }
        }
    }
    out.truncate(out.trim_end().len());
    out
}

fn spot_key(guide: &Path) -> Option<String> {
    let name = guide.file_name()?.to_str()?;
    let dir = guide.parent()?.file_name()?.to_str()?;
    Some(format!("{dir}/{name}"))
}

fn spots_file() -> String {
    format!("{CONFIG_DIR}/{GUIDE_SPOTS_FILE}")
}

pub fn read_guide_spot(root: &Path, guide: &Path) -> Option<GuideSpot> {
    let value = crate::ini::value(root, &spots_file(), &spot_key(guide)?)?;
    let mut parts = value.split_whitespace().map(str::parse::<u32>);
    let (Some(Ok(line)), Some(Ok(col)), Some(Ok(zoom))) =
        (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    Some(GuideSpot {
        line,
        col,
        zoom: u8::try_from(zoom).ok()?,
        wrap: matches!(parts.next(), Some(Ok(1))),
    })
}

pub fn write_guide_spot(root: &Path, guide: &Path, spot: GuideSpot) -> std::io::Result<()> {
    let Some(key) = spot_key(guide) else {
        return Ok(());
    };
    let value = format!(
        "{} {} {} {}",
        spot.line,
        spot.col,
        spot.zoom,
        u32::from(spot.wrap)
    );
    crate::ini::write(root, &spots_file(), &key, &value)
}
