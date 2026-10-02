//! Software drawing on RGBA frames: blending, smooth rounded shapes, images,
//! blur, and the PlayStation button icons.

pub type Rgb = [u8; 3];

pub const WHITE: Rgb = [0xFF, 0xFF, 0xFF];
pub const ORANGE: Rgb = [0xF2, 0x6B, 0x1D];
pub const ORANGE_LIGHT: Rgb = [0xFF, 0xA0, 0x5A];
pub const INK: Rgb = [0x0B, 0x0B, 0x10];

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
    fn blend_span(&mut self, y: i32, x0: i32, x1: i32, c: Rgb, a: u32) {
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

/// The app's backdrop: a warm base with waves flowing over it, on a clock
/// shared by every screen so the waves never jump.
pub struct Background {
    base: Image,
    start: std::time::Instant,
}

impl Background {
    pub fn new(base: Image) -> Self {
        Background { base, start: std::time::Instant::now() }
    }

    pub fn draw(&self, cv: &mut Canvas, waves: bool) {
        cv.copy_from(&self.base);
        if waves {
            draw_waves(cv, self.start.elapsed().as_secs_f32());
        }
    }
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
pub fn draw_waves(cv: &mut Canvas, t: f32) {
    const RIBBONS: [(f32, f32, f32, f32, f32, f32); 3] = [
        // base y, amplitude, wavelength k, speed, thickness, phase
        (610.0, 46.0, 0.0021, 0.32, 150.0, 0.0),
        (660.0, 60.0, 0.0016, -0.22, 190.0, 2.1),
        (720.0, 38.0, 0.0027, 0.41, 120.0, 4.2),
    ];
    let tint: [u32; 3] = [0xFF, 0xD2, 0xA8];
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
