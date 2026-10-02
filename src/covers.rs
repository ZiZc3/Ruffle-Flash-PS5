//! Game covers: the player's own art in /data/ruffle/covers, or a frame the
//! app captured while the game ran; and the tiles and blurred backdrops made
//! from them.

use std::fs;
use std::path::{Path, PathBuf};

use crate::ui::gfx::{Canvas, Image, Rgb, WHITE};
use crate::ui::text::{Text, Weight};

pub const COVERS_DIR: &str = "/data/ruffle/covers";
pub const TILE_W: i32 = 298;
pub const TILE_H: i32 = 186;

/// A file-name-safe key for a game, from its file name.
pub fn game_key(path: &str) -> String {
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "builtin-test".into());
    stem.chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' })
        .collect()
}

fn candidates(key: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for name in [key.to_string(), key.to_lowercase()] {
        for ext in ["png", "jpg", "jpeg", "PNG", "JPG"] {
            out.push(PathBuf::from(COVERS_DIR).join(format!("{}.{}", name, ext)));
        }
    }
    out
}

pub fn has_cover(key: &str) -> bool {
    candidates(key).iter().any(|p| p.exists())
}

pub fn load_cover(key: &str) -> Option<Image> {
    for path in candidates(key) {
        if !path.exists() {
            continue;
        }
        match image::open(&path) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                return Some(Image::from_rgba(w, h, rgba.into_raw()));
            }
            Err(e) => println!("[Covers] can't read {}: {}", path.display(), e),
        }
    }
    None
}

/// Saves the movie's area of a 1920x1080 letterboxed frame as the game's cover.
pub fn save_cover(key: &str, frame: &[u8], movie_w: f32, movie_h: f32) -> bool {
    let (fw, fh) = (1920.0f32, 1080.0f32);
    let scale = (fw / movie_w.max(1.0)).min(fh / movie_h.max(1.0));
    let (cw, ch) = ((movie_w * scale) as u32, (movie_h * scale) as u32);
    let (x0, y0) = (((fw as u32) - cw) / 2, ((fh as u32) - ch) / 2);
    let Some(full) = image::RgbaImage::from_raw(1920, 1080, frame.to_vec()) else {
        return false;
    };
    let crop = image::imageops::crop_imm(&full, x0, y0, cw.max(1), ch.max(1)).to_image();
    let out_w = 640u32;
    let out_h = (ch as f32 * out_w as f32 / cw.max(1) as f32) as u32;
    let small = image::imageops::resize(&crop, out_w, out_h.max(1), image::imageops::FilterType::Triangle);
    let _ = fs::create_dir_all(COVERS_DIR);
    let path = PathBuf::from(COVERS_DIR).join(format!("{}.png", key));
    match small.save_with_format(&path, image::ImageFormat::Png) {
        Ok(()) => {
            println!("[Covers] saved {}", path.display());
            true
        }
        Err(e) => {
            println!("[Covers] can't save {}: {}", path.display(), e);
            false
        }
    }
}

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t) as u8,
    ]
}

/// Two warm colours per game (from its name), for games without a cover.
fn palette(key: &str) -> (Rgb, Rgb) {
    const PAIRS: [(Rgb, Rgb); 5] = [
        ([0xF2, 0x6B, 0x1D], [0x5A, 0x14, 0x08]),
        ([0xFF, 0x8F, 0x3D], [0x3B, 0x16, 0x2E]),
        ([0xE8, 0x4A, 0x1A], [0x24, 0x10, 0x30]),
        ([0xFF, 0xA8, 0x4A], [0x6B, 0x22, 0x0C]),
        ([0xD9, 0x5B, 0x2B], [0x1C, 0x14, 0x24]),
    ];
    PAIRS[(hash(key) % PAIRS.len() as u64) as usize]
}

fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let s: String = match words.len() {
        0 => "?".into(),
        1 => words[0].chars().take(2).collect(),
        _ => words.iter().take(2).filter_map(|w| w.chars().next()).collect(),
    };
    s.to_uppercase()
}

/// A w x h tile: the cover, or a generated card with the game's initials.
pub fn make_tile(key: &str, name: &str, cover: Option<&Image>, text: &mut Text, w: i32, h: i32) -> Image {
    if let Some(c) = cover {
        return c.cover(w, h);
    }
    let (light, dark) = palette(key);
    let mut cv = Canvas::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = ((x as f32 / w as f32) * 0.35 + (y as f32 / h as f32) * 0.65).clamp(0.0, 1.0);
            let c = lerp(light, dark, t);
            let i = ((y * w + x) * 4) as usize;
            cv.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 0xFF]);
        }
    }
    cv.fill_circle(w as f32 * 0.82, h as f32 * 0.1, h as f32 * 0.64, WHITE, 0.07);
    let ini = initials(name);
    let big = (h as f32 * 0.39) as u32;
    let iw = text.width(Weight::Bold, big, &ini);
    text.draw(&mut cv, Weight::Bold, big, (w - iw) / 2, (h - big as i32) / 2 - (h / 14), &ini, WHITE, 0.95);
    let small = (h as f32 * 0.08).max(13.0) as u32;
    let pill_h = small as i32 + 11;
    let pill_w = text.width(Weight::SemiBold, small, "SWF") + 20;
    cv.fill_round_rect(14, h - pill_h - 12, pill_w, pill_h, pill_h / 2, [0, 0, 0], 0.35);
    text.draw(&mut cv, Weight::SemiBold, small, 24, h - pill_h - 7, "SWF", WHITE, 0.9);
    Image { w, h, px: cv.px }
}

/// The full-screen base under the waves: the cover blurred into a dark warm
/// gradient, or a warm glow without one.
pub fn make_backdrop(key: &str, cover: Option<&Image>) -> Image {
    let mut small = match cover {
        Some(c) => c.cover(240, 135),
        None => {
            let (light, dark) = palette(key);
            let mut img = Image::new(240, 135);
            let h = hash(key);
            let (gx, gy) = (120.0 + (h % 100) as f32, 20.0 + ((h >> 8) % 60) as f32);
            for y in 0..135 {
                for x in 0..240 {
                    let d = (((x as f32 - gx).powi(2) + (y as f32 - gy).powi(2)).sqrt() / 150.0).min(1.0);
                    let c = lerp(lerp(light, dark, 0.35), [0x08, 0x08, 0x0C], d);
                    let i = ((y * 240 + x) * 4) as usize;
                    img.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 0xFF]);
                }
            }
            img
        }
    };
    small.blur(if cover.is_some() { 5 } else { 3 });

    let mut big = small.resized(1920, 1080);
    // Mixed into a dark ember gradient, so every game's screen shares one mood.
    let (top, bottom): (Rgb, Rgb) = ([0x2A, 0x12, 0x08], [0x08, 0x06, 0x0A]);
    let keep = if cover.is_some() { 0.42 } else { 0.55 };
    for y in 0..1080 {
        let g = lerp(top, bottom, y as f32 / 1080.0);
        for x in 0..1920 {
            let i = ((y * 1920 + x) * 4) as usize;
            for c in 0..3 {
                big.px[i + c] = (big.px[i + c] as f32 * keep + g[c] as f32 * (1.0 - keep)) as u8;
            }
            big.px[i + 3] = 0xFF;
        }
    }
    big
}
