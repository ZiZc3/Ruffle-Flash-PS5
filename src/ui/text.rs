//! Inter, rasterised with fontdue and cached per glyph and size.

use std::collections::HashMap;

use fontdue::{Font, FontSettings, Metrics};

use super::gfx::{Canvas, Rgb};

pub static REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
static SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
pub static BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    SemiBold,
    Bold,
}

pub struct Text {
    fonts: [Font; 3],
    glyphs: HashMap<(Weight, u32, char), (Metrics, Vec<u8>)>,
}

impl Text {
    pub fn new() -> Self {
        let load = |bytes: &[u8]| Font::from_bytes(bytes, FontSettings::default()).expect("font");
        Text {
            fonts: [load(REGULAR), load(SEMIBOLD), load(BOLD)],
            glyphs: HashMap::new(),
        }
    }

    fn font(&self, w: Weight) -> &Font {
        &self.fonts[w as usize]
    }

    fn glyph(&mut self, w: Weight, size: u32, c: char) -> &(Metrics, Vec<u8>) {
        if self.glyphs.len() > 4000 {
            self.glyphs.clear();
        }
        let fonts = &self.fonts;
        self.glyphs
            .entry((w, size, c))
            .or_insert_with(|| fonts[w as usize].rasterize(c, size as f32))
    }

    /// Distance from the top of a line to its baseline.
    pub fn ascent(&self, w: Weight, size: u32) -> f32 {
        self.font(w)
            .horizontal_line_metrics(size as f32)
            .map(|m| m.ascent)
            .unwrap_or(size as f32 * 0.8)
    }

    pub fn width(&mut self, w: Weight, size: u32, s: &str) -> i32 {
        let mut x = 0.0f32;
        let mut prev: Option<char> = None;
        for c in s.chars() {
            if let Some(p) = prev {
                x += self.font(w).horizontal_kern(p, c, size as f32).unwrap_or(0.0);
            }
            x += self.glyph(w, size, c).0.advance_width;
            prev = Some(c);
        }
        x.ceil() as i32
    }

    /// Draws `s` with its top-left at (x, y); returns the width drawn.
    pub fn draw(&mut self, cv: &mut Canvas, w: Weight, size: u32, x: i32, y: i32, s: &str, color: Rgb, alpha: f32) -> i32 {
        let baseline = y as f32 + self.ascent(w, size);
        let mut pen = x as f32;
        let mut prev: Option<char> = None;
        for c in s.chars() {
            if let Some(p) = prev {
                pen += self.font(w).horizontal_kern(p, c, size as f32).unwrap_or(0.0);
            }
            let (m, bitmap) = self.glyph(w, size, c);
            let gx = (pen + m.xmin as f32).round() as i32;
            let gy = (baseline - m.ymin as f32 - m.height as f32).round() as i32;
            let a = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
            for row in 0..m.height {
                for col in 0..m.width {
                    let cov = bitmap[row * m.width + col] as u32;
                    if cov > 0 {
                        cv.blend(gx + col as i32, gy + row as i32, color, (cov * a) >> 8);
                    }
                }
            }
            pen += m.advance_width;
            prev = Some(c);
        }
        (pen - x as f32).ceil() as i32
    }

    /// Splits `s` into at most `lines` lines of `max` pixels (the last one
    /// shortened with an ellipsis if it still doesn't fit).
    pub fn wrap(&mut self, w: Weight, size: u32, s: &str, max: i32, lines: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut line = String::new();
        let words: Vec<&str> = s.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{} {}", line, word) };
            if self.width(w, size, &candidate) <= max || line.is_empty() {
                line = candidate;
            } else {
                out.push(std::mem::take(&mut line));
                if out.len() == lines - 1 {
                    let rest = words[i..].join(" ");
                    out.push(self.fit(w, size, &rest, max));
                    return out;
                }
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            let fitted = self.fit(w, size, &line, max);
            out.push(fitted);
        }
        out
    }

    /// Shortens `s` with an ellipsis to fit `max` pixels.
    pub fn fit(&mut self, w: Weight, size: u32, s: &str, max: i32) -> String {
        if self.width(w, size, s) <= max {
            return s.to_string();
        }
        let mut chars: Vec<char> = s.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let candidate: String = chars.iter().collect::<String>() + "...";
            if self.width(w, size, &candidate) <= max {
                return candidate;
            }
        }
        String::new()
    }
}
