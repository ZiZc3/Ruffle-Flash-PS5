//! Software drawing on RGBA frames: blending, smooth rounded shapes, images,
//! blur, and the PlayStation button icons.

pub type Rgb = [u8; 3];

pub const WHITE: Rgb = [0xFF, 0xFF, 0xFF];
pub const ORANGE: Rgb = [0xF2, 0x6B, 0x1D];
pub const ORANGE_LIGHT: Rgb = [0xFF, 0xA0, 0x5A];
pub const INK: Rgb = [0x0B, 0x0B, 0x10];
pub const GOLD: Rgb = [0xFF, 0xC8, 0x5A];

/// The accent colour (Settings > App > Accent colour), packed 0xRRGGBB.
static ACCENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0xF26B1D);

pub fn set_accent(c: Rgb) {
    ACCENT.store(((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32, std::sync::atomic::Ordering::Relaxed);
}

/// The accent colour: tabs, selection bars, values, spinners.
pub fn accent() -> Rgb {
    let v = ACCENT.load(std::sync::atomic::Ordering::Relaxed);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// The accent, lighter (glows).
pub fn accent_light() -> Rgb {
    let a = accent();
    [a[0] / 2 + 128, a[1] / 2 + 128, a[2] / 2 + 128]
}

/// An RGBA8 image.
#[derive(Clone)]
pub struct Image {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
}

impl Image {
    pub fn new(w: i32, h: i32) -> Self {
        Image { w, h, px: vec![0; (w * h * 4) as usize] }
    }

    pub fn from_rgba(w: u32, h: u32, px: Vec<u8>) -> Self {
        Image { w: w as i32, h: h as i32, px }
    }

    pub fn resized(&self, w: i32, h: i32) -> Image {
        let src = image::RgbaImage::from_raw(self.w as u32, self.h as u32, self.px.clone())
            .expect("image size");
        let out = image::imageops::resize(&src, w as u32, h as u32, image::imageops::FilterType::Triangle);
        Image::from_rgba(w as u32, h as u32, out.into_raw())
    }

    /// Scales to cover w x h (cropping the excess, centred).
    pub fn cover(&self, w: i32, h: i32) -> Image {
        let scale = (w as f32 / self.w as f32).max(h as f32 / self.h as f32);
        let sw = ((self.w as f32 * scale).ceil() as i32).max(w);
        let sh = ((self.h as f32 * scale).ceil() as i32).max(h);
        let scaled = self.resized(sw, sh);
        let mut out = Image::new(w, h);
        let ox = (sw - w) / 2;
        let oy = (sh - h) / 2;
        for y in 0..h {
            let s = (((y + oy) * sw + ox) * 4) as usize;
            let d = ((y * w) * 4) as usize;
            out.px[d..d + (w * 4) as usize].copy_from_slice(&scaled.px[s..s + (w * 4) as usize]);
        }
        out
    }

    /// Box blur, three passes (close to a gaussian).
    pub fn blur(&mut self, radius: i32) {
        for _ in 0..3 {
            self.box_pass(radius, true);
            self.box_pass(radius, false);
        }
    }

    fn box_pass(&mut self, r: i32, horizontal: bool) {
        let (len, lines) = if horizontal { (self.w, self.h) } else { (self.h, self.w) };
        let mut line = vec![[0i32; 3]; len as usize];
        for l in 0..lines {
            let idx = |i: i32| -> usize {
                let (x, y) = if horizontal { (i, l) } else { (l, i) };
                ((y * self.w + x) * 4) as usize
            };
            for i in 0..len {
                let p = idx(i);
                line[i as usize] = [self.px[p] as i32, self.px[p + 1] as i32, self.px[p + 2] as i32];
            }
            let mut acc = [0i32; 3];
            for k in -r..=r {
                let v = line[k.clamp(0, len - 1) as usize];
                for c in 0..3 {
                    acc[c] += v[c];
                }
            }
            let n = 2 * r + 1;
            for i in 0..len {
                let p = idx(i);
                for c in 0..3 {
                    self.px[p + c] = (acc[c] / n) as u8;
                }
                let out = line[(i - r).clamp(0, len - 1) as usize];
                let inn = line[(i + r + 1).clamp(0, len - 1) as usize];
                for c in 0..3 {
                    acc[c] += inn[c] - out[c];
                }
            }
        }
    }
}

/// The frame being drawn.
pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
}

#[inline]
fn mix(dst: u8, src: u8, a: u32) -> u8 {
    ((dst as u32 * (255 - a) + src as u32 * a + 127) / 255) as u8
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Self {
        Canvas { w, h, px: vec![0; (w * h * 4) as usize] }
    }

    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: Rgb, a: u32) {
        if a == 0 || x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        if a >= 255 {
            self.px[i..i + 3].copy_from_slice(&c);
        } else {
            self.px[i] = mix(self.px[i], c[0], a);
            self.px[i + 1] = mix(self.px[i + 1], c[1], a);
            self.px[i + 2] = mix(self.px[i + 2], c[2], a);
        }
        self.px[i + 3] = 0xFF;
    }

    /// Blends one colour over a row span, clipped.
    pub fn blend_span(&mut self, y: i32, x0: i32, x1: i32, c: Rgb, a: u32) {
        let (x0, x1) = (x0.max(0), x1.min(self.w));
        if y < 0 || y >= self.h || x1 <= x0 || a == 0 {
            return;
        }
        let a = a.min(255) * 257; // 0..65535
        let i = ((y * self.w + x0) * 4) as usize;
        let n = ((x1 - x0) * 4) as usize;
        let pre = [c[0] as u32 * a, c[1] as u32 * a, c[2] as u32 * a];
        let keep = 65536 - a;
        for p in self.px[i..i + n].chunks_exact_mut(4) {
            p[0] = ((p[0] as u32 * keep + pre[0]) >> 16) as u8;
            p[1] = ((p[1] as u32 * keep + pre[1]) >> 16) as u8;
            p[2] = ((p[2] as u32 * keep + pre[2]) >> 16) as u8;
        }
    }

    pub fn copy_from(&mut self, img: &Image) {
        if img.w == self.w && img.h == self.h {
            self.px.copy_from_slice(&img.px);
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb, alpha: f32) {
        let a = (alpha.clamp(0.0, 1.0) * 255.0) as u32;
        for py in y.max(0)..(y + h).min(self.h) {
            self.blend_span(py, x, x + w, c, a);
        }
    }

    /// Coverage (0..1) of a rounded rectangle at a pixel centre, antialiased.
    fn round_cover(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
        let cx = px.clamp(x + r, x + w - r);
        let cy = py.clamp(y + r, y + h - r);
        let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r;
        (0.5 - d).clamp(0.0, 1.0)
    }

    pub fn fill_round_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, c: Rgb, alpha: f32) {
        let ri = r.min(w / 2).min(h / 2);
        let r = ri as f32;
        let a = (alpha.clamp(0.0, 1.0) * 255.0) as u32;
        for py in (y - 1).max(0)..(y + h + 1).min(self.h) {
            let corner_row = py < y + ri || py >= y + h - ri;
            if !corner_row {
                self.blend_span(py, x, x + w, c, a);
                continue;
            }
            for px in (x - 1).max(0)..(x + w + 1).min(self.w) {
                // Only the corners need the distance test.
                if !corner_row && px >= x && px < x + w {
                    self.blend(px, py, c, a);
                    continue;
                }
                if !corner_row || px < x + ri || px >= x + w - ri {
                    let cov = Self::round_cover(px as f32 + 0.5, py as f32 + 0.5, x as f32, y as f32, w as f32, h as f32, r);
                    self.blend(px, py, c, (cov * a as f32) as u32);
                } else if py >= y && py < y + h {
                    self.blend(px, py, c, a);
                }
            }
        }
    }

    /// A rounded outline of the given thickness.
    pub fn stroke_round_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, c: Rgb, alpha: f32) {
        let ro = r as f32;
        let ri = (r - t).max(0) as f32;
        let band = r.max(t) + 1;
        for py in (y - 1).max(0)..(y + h + 1).min(self.h) {
            // Between the corners only the left and right edges are visited.
            let middle = py >= y + band && py < y + h - band;
            for px in (x - 1).max(0)..(x + w + 1).min(self.w) {
                if middle && px >= x + t + 1 && px < x + w - t - 1 {
                    continue;
                }
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let outer = Self::round_cover(fx, fy, x as f32, y as f32, w as f32, h as f32, ro);
                let inner = Self::round_cover(
                    fx,
                    fy,
                    (x + t) as f32,
                    (y + t) as f32,
                    (w - 2 * t) as f32,
                    (h - 2 * t) as f32,
                    ri,
                );
                let cov = (outer - inner).max(0.0);
                self.blend(px, py, c, (cov * alpha * 255.0) as u32);
            }
        }
    }

    /// A soft shadow / glow around a rounded rectangle.
    pub fn glow(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, spread: i32, c: Rgb, alpha: f32) {
        let rr = r as f32;
        for py in (y - spread).max(0)..(y + h + spread).min(self.h) {
            for px in (x - spread).max(0)..(x + w + spread).min(self.w) {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let cx = fx.clamp(x as f32 + rr, (x + w) as f32 - rr);
                let cy = fy.clamp(y as f32 + rr, (y + h) as f32 - rr);
                let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt() - rr;
                if d <= 0.0 {
                    continue;
                }
                let t = 1.0 - (d / spread as f32);
                if t > 0.0 {
                    self.blend(px, py, c, (t * t * alpha * 255.0) as u32);
                }
            }
        }
    }

    /// Draws an image with transparency at (x, y).
    pub fn draw_image(&mut self, img: &Image, x: i32, y: i32) {
        for iy in 0..img.h {
            for ix in 0..img.w {
                let s = ((iy * img.w + ix) * 4) as usize;
                let a = img.px[s + 3] as u32;
                if a > 0 {
                    self.blend(x + ix, y + iy, [img.px[s], img.px[s + 1], img.px[s + 2]], a);
                }
            }
        }
    }

    /// Draws an image at (x, y), clipped to a rounded rectangle.
    pub fn draw_image_rounded(&mut self, img: &Image, x: i32, y: i32, r: i32, alpha: f32) {
        let rf = r.min(img.w / 2).min(img.h / 2) as f32;
        let opaque = alpha >= 1.0;
        for iy in 0..img.h {
            let py = y + iy;
            if py < 0 || py >= self.h {
                continue;
            }
            // Fast path: the rows between the corners need no corner test;
            // opaque ones are copied straight in.
            if iy >= r && iy < img.h - r {
                let x0 = x.max(0);
                let x1 = (x + img.w).min(self.w);
                if x1 > x0 {
                    let s = ((iy * img.w + (x0 - x)) * 4) as usize;
                    let d = ((py * self.w + x0) * 4) as usize;
                    let n = ((x1 - x0) * 4) as usize;
                    if opaque {
                        self.px[d..d + n].copy_from_slice(&img.px[s..s + n]);
                    } else {
                        let a = (alpha.clamp(0.0, 1.0) * 255.0) as u32;
                        for (dp, sp) in self.px[d..d + n].chunks_exact_mut(4).zip(img.px[s..s + n].chunks_exact(4)) {
                            dp[0] = mix(dp[0], sp[0], a);
                            dp[1] = mix(dp[1], sp[1], a);
                            dp[2] = mix(dp[2], sp[2], a);
                        }
                    }
                }
                continue;
            }
            for ix in 0..img.w {
                let px = x + ix;
                if px < 0 || px >= self.w {
                    continue;
                }
                let corner = ix < r || iy < r || ix >= img.w - r || iy >= img.h - r;
                let cov = if corner {
                    Self::round_cover(ix as f32 + 0.5, iy as f32 + 0.5, 0.0, 0.0, img.w as f32, img.h as f32, rf)
                } else {
                    1.0
                };
                let s = ((iy * img.w + ix) * 4) as usize;
                let a = (cov * alpha * img.px[s + 3] as f32) as u32;
                self.blend(px, py, [img.px[s], img.px[s + 1], img.px[s + 2]], a);
            }
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, alpha: f32) {
        for py in (cy - r - 1.0) as i32..=(cy + r + 1.0) as i32 {
            for px in (cx - r - 1.0) as i32..=(cx + r + 1.0) as i32 {
                let d = ((px as f32 + 0.5 - cx).powi(2) + (py as f32 + 0.5 - cy).powi(2)).sqrt();
                let cov = (r + 0.5 - d).clamp(0.0, 1.0);
                self.blend(px, py, c, (cov * alpha * 255.0) as u32);
            }
        }
    }

    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, t: f32, c: Rgb, alpha: f32) {
        for py in (cy - r - 1.0) as i32..=(cy + r + 1.0) as i32 {
            for px in (cx - r - 1.0) as i32..=(cx + r + 1.0) as i32 {
                let d = ((px as f32 + 0.5 - cx).powi(2) + (py as f32 + 0.5 - cy).powi(2)).sqrt();
                let cov = ((t / 2.0 + 0.5) - (d - (r - t / 2.0)).abs()).clamp(0.0, 1.0);
                self.blend(px, py, c, (cov * alpha * 255.0) as u32);
            }
        }
    }

    /// An antialiased thick line.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, c: Rgb, alpha: f32) {
        let (minx, maxx) = (x0.min(x1) - t, x0.max(x1) + t);
        let (miny, maxy) = (y0.min(y1) - t, y0.max(y1) + t);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len2 = (dx * dx + dy * dy).max(0.0001);
        for py in miny as i32..=maxy as i32 {
            for px in minx as i32..=maxx as i32 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let k = (((fx - x0) * dx + (fy - y0) * dy) / len2).clamp(0.0, 1.0);
                let d = ((fx - (x0 + k * dx)).powi(2) + (fy - (y0 + k * dy)).powi(2)).sqrt();
                let cov = (t / 2.0 + 0.5 - d).clamp(0.0, 1.0);
                self.blend(px, py, c, (cov * alpha * 255.0) as u32);
            }
        }
    }

    /// Multiplies the frame toward black: 1.0 leaves it, 0.0 is black.
    pub fn fade(&mut self, brightness: f32) {
        fade(&mut self.px, brightness);
    }
}

/// Multiplies an RGBA frame toward black: 1.0 leaves it, 0.0 is black.
pub fn fade(px: &mut [u8], brightness: f32) {
    let b = (brightness.clamp(0.0, 1.0) * 256.0) as u32;
    if b >= 256 {
        return;
    }
    for p in px.chunks_exact_mut(4) {
        p[0] = ((p[0] as u32 * b) >> 8) as u8;
        p[1] = ((p[1] as u32 * b) >> 8) as u8;
        p[2] = ((p[2] as u32 * b) >> 8) as u8;
    }
}

/// Themes (Settings > App > Theme): name, gradient top and bottom, glow,
/// and the waves' colour.
pub const THEMES: [(&str, Rgb, Rgb, Rgb, [u32; 3]); 4] = [
    ("Ember", [0x2A, 0x12, 0x08], [0x08, 0x06, 0x0A], [0xC0, 0x50, 0x20], [0xFF, 0xD2, 0xA8]),
    ("Midnight", [0x0A, 0x14, 0x30], [0x03, 0x04, 0x0C], [0x30, 0x58, 0xC0], [0xB8, 0xD0, 0xFF]),
    ("Retro CRT", [0x05, 0x1A, 0x0E], [0x01, 0x05, 0x03], [0x20, 0x90, 0x48], [0x9C, 0xFF, 0xB8]),
    ("Newgrounds Dark", [0x20, 0x20, 0x24], [0x09, 0x09, 0x0B], [0x60, 0x58, 0x50], [0xFF, 0xE0, 0xC0]),
];
pub const THEME_CRT: u8 = 2;

/// A theme's full-screen base: its gradient with a soft glow up top (and a
/// vignette for the CRT).
pub fn theme_base(theme: u8) -> Image {
    let (_, top, bottom, glow, _) = THEMES[theme as usize % THEMES.len()];
    let mut small = Image::new(240, 135);
    for y in 0..135 {
        for x in 0..240 {
            let t = y as f32 / 134.0;
            let d = (((x as f32 - 150.0) / 150.0).powi(2) + ((y as f32 - 25.0) / 95.0).powi(2)).sqrt().min(1.0);
            let g = (1.0 - d).powi(2) * 0.45;
            let mut c = [0u8; 3];
            for i in 0..3 {
                let base = top[i] as f32 * (1.0 - t) + bottom[i] as f32 * t;
                let mut v = base + (glow[i] as f32 - base) * g;
                if theme == THEME_CRT {
                    let (vx, vy) = (x as f32 / 120.0 - 1.0, y as f32 / 67.5 - 1.0);
                    v *= 1.0 - 0.45 * (vx * vx * 0.6 + vy * vy).min(1.0);
                }
                c[i] = v.clamp(0.0, 255.0) as u8;
            }
            let p = ((y * 240 + x) * 4) as usize;
            small.px[p..p + 4].copy_from_slice(&[c[0], c[1], c[2], 0xFF]);
        }
    }
    small.blur(2);
    small.resized(1920, 1080)
}

/// The app's backdrop: the theme's base with waves flowing over it, on a
/// clock shared by every screen so the waves never jump.
pub struct Background {
    base: Image,
    start: std::time::Instant,
    tint: [u32; 3],
    pub theme: u8,
}

impl Background {
    pub fn new(base: Image, theme: u8) -> Self {
        let tint = THEMES[theme as usize % THEMES.len()].4;
        Background { base, start: std::time::Instant::now(), tint, theme }
    }

    /// Another theme, keeping the waves' clock.
    pub fn set_theme(&mut self, base: Image, theme: u8) {
        self.base = base;
        self.tint = THEMES[theme as usize % THEMES.len()].4;
        self.theme = theme;
    }

    pub fn draw(&self, cv: &mut Canvas, waves: bool) {
        cv.copy_from(&self.base);
        if waves {
            draw_waves(cv, self.start.elapsed().as_secs_f32(), self.tint);
        }
    }

    /// Drawn over a finished menu frame: the CRT theme's scanlines.
    pub fn overlay(&self, cv: &mut Canvas) {
        if self.theme == THEME_CRT {
            let roll = (self.start.elapsed().as_secs_f32() * 40.0) as i32;
            for y in (0..cv.h).step_by(3) {
                cv.blend_span(y, 0, cv.w, INK, 72);
            }
            // A faint bright band rolling down, like an old set.
            let band = roll % (cv.h + 200) - 100;
            for y in band.max(0)..(band + 100).min(cv.h) {
                let a = (1.0 - ((y - band) as f32 / 50.0 - 1.0).abs()) * 7.0;
                cv.blend_span(y, 0, cv.w, WHITE, a as u32);
            }
        }
    }
}

/// A badge medal at (cx, cy) of radius r: gold (or grey when locked) with a
/// star, an accent ring and a soft glow.
pub fn medal(cv: &mut Canvas, cx: f32, cy: f32, r: f32, unlocked: bool, alpha: f32, icon: usize) {
    let (rim, face, star): (Rgb, Rgb, Rgb) = if unlocked {
        ([0xC8, 0x8A, 0x1E], GOLD, [0xFF, 0xF4, 0xD0])
    } else {
        ([0x40, 0x40, 0x46], [0x5A, 0x5A, 0x62], [0x80, 0x80, 0x88])
    };
    if unlocked {
        for i in 0..6 {
            cv.fill_circle(cx, cy, r * (1.5 - i as f32 * 0.08), GOLD, 0.03 * alpha);
        }
    }
    cv.fill_circle(cx, cy, r, rim, alpha);
    cv.fill_circle(cx, cy, r * 0.84, face, alpha);
    cv.stroke_circle(cx, cy, r * 0.84, (r * 0.07).max(1.5), if unlocked { accent() } else { rim }, alpha);
    super::icons::badge_icon(cv, icon, cx, cy, r * 0.95, star, face, alpha);
}

/// A diagonal glass highlight sweeping across a rounded rectangle;
/// `progress` 0..1 moves it from the left edge past the right.
pub fn shine(cv: &mut Canvas, x: i32, y: i32, w: i32, h: i32, r: i32, progress: f32, strength: f32) {
    let slant = 0.55;
    let span = w as f32 + h as f32 * slant;
    let centre = (-0.2 + 1.4 * progress) * span;
    let half = span * 0.07;
    let rr = r as f32;
    for py in y.max(0)..(y + h).min(cv.h) {
        let row = (py - y) as f32;
        // Only the columns the band crosses on this row.
        let from = (centre - half - row * slant).floor() as i32;
        let to = (centre + half - row * slant).ceil() as i32;
        for ix in from.max(0)..to.min(w) {
            let u = ix as f32 + row * slant;
            let a = 1.0 - ((u - centre).abs() / half);
            if a <= 0.0 {
                continue;
            }
            // Stay inside the rounded corners.
            let (fx, fy) = (ix as f32 + 0.5, row + 0.5);
            let cx = fx.clamp(rr, w as f32 - rr);
            let cy = fy.clamp(rr, h as f32 - rr);
            if (fx - cx).powi(2) + (fy - cy).powi(2) > rr * rr {
                continue;
            }
            cv.blend(x + ix, py, WHITE, (a * a * strength * 255.0) as u32);
        }
    }
}

/// Light ribbons flowing across the screen (the PSP / PPSSPP wave look):
/// each a soft band under a bright edge, swaying with time `t` in seconds.
pub fn draw_waves(cv: &mut Canvas, t: f32, tint: [u32; 3]) {
    const RIBBONS: [(f32, f32, f32, f32, f32, f32); 3] = [
        // base y, amplitude, wavelength k, speed, thickness, phase
        (610.0, 46.0, 0.0021, 0.32, 150.0, 0.0),
        (660.0, 60.0, 0.0016, -0.22, 190.0, 2.1),
        (720.0, 38.0, 0.0027, 0.41, 120.0, 4.2),
    ];
    let stride = (cv.w * 4) as usize;
    // Body opacity at its top edge, in 1/65536ths (0.085).
    const BODY_A: u32 = 5570;
    for (base, amp, k, speed, thick, phase) in RIBBONS {
        for x in 0..cv.w {
            let xf = x as f32;
            let yc = base
                + amp * (xf * k + t * speed + phase).sin()
                + amp * 0.35 * (xf * k * 2.3 - t * speed * 1.7 + phase).sin();
            let th = thick + 40.0 * (xf * k * 0.7 + t * 0.15 + phase).sin();

            // Bright antialiased edge, split between the two rows it falls on.
            let row = yc.floor();
            let frac = yc - row;
            let edge: Rgb = [tint[0] as u8, tint[1] as u8, tint[2] as u8];
            cv.blend(x, row as i32, edge, ((1.0 - frac) * 0.30 * 255.0) as u32);
            cv.blend(x, row as i32 + 1, edge, (frac * 0.30 * 255.0) as u32);

            // The ribbon body, fading out downward: integer blend down the column.
            let top = row as i32 + 2;
            let len = th as i32;
            let (y0, y1) = (top.max(0), (top + len).min(cv.h));
            if y1 <= y0 {
                continue;
            }
            let step = BODY_A / len as u32;
            let mut a = BODY_A - step * (y0 - top) as u32;
            let mut i = (y0 as usize) * stride + x as usize * 4;
            for _ in y0..y1 {
                let p = &mut cv.px[i..i + 3];
                for c in 0..3 {
                    let d = p[c] as u32;
                    p[c] = ((d * (65536 - a) + tint[c] * a) >> 16) as u8;
                }
                a = a.saturating_sub(step);
                i += stride;
            }
        }
    }
}

/// Soft light rays turning around (cx, cy): `count` beams fading out toward
/// `radius`, rotated by `angle` (radians).
pub fn rays(cv: &mut Canvas, cx: f32, cy: f32, radius: f32, count: u32, angle: f32, c: Rgb, alpha: f32) {
    use std::f32::consts::TAU;
    let step = TAU / count as f32;
    let (x0, x1) = ((cx - radius).max(0.0) as i32, (cx + radius).min(cv.w as f32) as i32);
    let (y0, y1) = ((cy - radius).max(0.0) as i32, (cy + radius).min(cv.h as f32) as i32);
    for py in y0..y1 {
        for px in x0..x1 {
            let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
            let d2 = dx * dx + dy * dy;
            if d2 >= radius * radius {
                continue;
            }
            let d = d2.sqrt() / radius;
            // Angular distance to the nearest beam's centre, 0..0.5 of a step.
            let a = (dy.atan2(dx) - angle).rem_euclid(step) / step;
            let beam = 1.0 - (a - 0.5).abs() * 2.0; // 1 at the centre, 0 between
            let beam = (beam * 1.6 - 0.6).clamp(0.0, 1.0);
            let fall = (1.0 - d) * (1.0 - d);
            let v = beam * beam * fall * alpha;
            if v > 0.004 {
                cv.blend(px, py, c, (v * 255.0) as u32);
            }
        }
    }
}

/// A loading spinner: an arc with a fading tail, turning with time `t` (s).
pub fn spinner(cv: &mut Canvas, cx: f32, cy: f32, r: f32, thick: f32, t: f32, c: Rgb) {
    use std::f32::consts::TAU;
    let head = (t * 1.6 * TAU) % TAU;
    let span = TAU * 0.72;
    cv.stroke_circle(cx, cy, r, thick, WHITE, 0.12);
    for py in (cy - r - thick) as i32..=(cy + r + thick) as i32 {
        for px in (cx - r - thick) as i32..=(cx + r + thick) as i32 {
            let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            let ring = ((thick / 2.0 + 0.5) - (d - r).abs()).clamp(0.0, 1.0);
            if ring <= 0.0 {
                continue;
            }
            // How far behind the head this point is, along the arc.
            let ang = dy.atan2(dx).rem_euclid(TAU);
            let behind = (head - ang).rem_euclid(TAU);
            if behind > span {
                continue;
            }
            let fade = (1.0 - behind / span).sqrt();
            cv.blend(px, py, c, (ring * fade * 255.0) as u32);
        }
    }
}

#[derive(Clone, Copy)]
pub enum PadIcon {
    Cross,
    Circle,
    Triangle,
    Square,
}

/// A PlayStation face-button icon: a white disc with the symbol cut in dark.
pub fn pad_icon(cv: &mut Canvas, icon: PadIcon, cx: f32, cy: f32, r: f32) {
    cv.fill_circle(cx, cy, r, WHITE, 0.92);
    let s = r * 0.45;
    let t = (r * 0.16).max(2.0);
    match icon {
        PadIcon::Cross => {
            cv.line(cx - s, cy - s, cx + s, cy + s, t, INK, 1.0);
            cv.line(cx - s, cy + s, cx + s, cy - s, t, INK, 1.0);
        }
        PadIcon::Circle => cv.stroke_circle(cx, cy, s * 1.05, t, INK, 1.0),
        PadIcon::Triangle => {
            let (top, bl, br) = ((cx, cy - s * 1.05), (cx - s * 1.1, cy + s * 0.75), (cx + s * 1.1, cy + s * 0.75));
            cv.line(top.0, top.1, bl.0, bl.1, t, INK, 1.0);
            cv.line(bl.0, bl.1, br.0, br.1, t, INK, 1.0);
            cv.line(br.0, br.1, top.0, top.1, t, INK, 1.0);
        }
        PadIcon::Square => {
            let q = s * 0.85;
            cv.stroke_round_rect((cx - q) as i32, (cy - q) as i32, (2.0 * q) as i32, (2.0 * q) as i32, 2, t as i32, INK, 1.0);
        }
    }
}

/// Is (x, y) inside the polygon (even-odd rule)?
fn inside(points: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut odd = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let ((xi, yi), (xj, yj)) = (points[i], points[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            odd = !odd;
        }
        j = i;
    }
    odd
}

impl Canvas {
    /// A small filled polygon, antialiased by 4x4 samples per pixel.
    pub fn fill_polygon(&mut self, points: &[(f32, f32)], c: Rgb, alpha: f32) {
        if points.len() < 3 {
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in points {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        for py in y0.floor() as i32..=y1.ceil() as i32 {
            for px in x0.floor() as i32..=x1.ceil() as i32 {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let (fx, fy) = (px as f32 + (sx as f32 + 0.5) / 4.0, py as f32 + (sy as f32 + 0.5) / 4.0);
                        hits += inside(points, fx, fy) as u32;
                    }
                }
                if hits > 0 {
                    self.blend(px, py, c, (hits as f32 / 16.0 * alpha * 255.0) as u32);
                }
            }
        }
    }
}

/// The mouse pointer: a white arrow with a dark edge and a soft shadow, its
/// tip at (x, y).
pub fn pointer(cv: &mut Canvas, x: f32, y: f32, alpha: f32) {
    const ARROW: [(f32, f32); 7] =
        [(0.0, 0.0), (0.0, 30.0), (7.5, 23.0), (12.5, 34.5), (17.5, 32.5), (12.5, 21.5), (22.0, 21.5)];
    let at = |dx: f32, dy: f32, grow: f32| -> Vec<(f32, f32)> {
        // Grown about the arrow's middle for the edge.
        ARROW.iter().map(|&(px, py)| (x + dx + (px - 8.0) * grow + 8.0, y + dy + (py - 15.0) * grow + 15.0)).collect()
    };
    cv.fill_polygon(&at(2.0, 3.0, 1.12), INK, 0.35 * alpha);
    cv.fill_polygon(&at(0.0, 0.0, 1.12), INK, 0.9 * alpha);
    cv.fill_polygon(&at(0.0, 0.0, 1.0), WHITE, alpha);
}
