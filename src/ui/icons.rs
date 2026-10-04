//! Badge icons: one small drawing per badge (in badges::BADGES order),
//! made of a few shapes so they stay crisp at any size.

use super::gfx::{Canvas, Rgb};

/// Draws badge `i`'s icon centred at (cx, cy), about `r` across from the
/// centre, in colour `c`; `bg` is the medal's face (for cut-outs).
pub fn badge_icon(cv: &mut Canvas, i: usize, cx: f32, cy: f32, r: f32, c: Rgb, bg: Rgb, a: f32) {
    let u = r * 0.5;
    let t = (r * 0.09).max(1.6);
    let p = |x: f32, y: f32| (cx + x * u, cy + y * u);
    let poly = |cv: &mut Canvas, pts: &[(f32, f32)], col: Rgb| {
        let v: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| p(x, y)).collect();
        cv.fill_polygon(&v, col, a);
    };
    let line = |cv: &mut Canvas, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, col: Rgb| {
        let (a0, b0) = p(x0, y0);
        let (a1, b1) = p(x1, y1);
        cv.line(a0, b0, a1, b1, w, col, a);
    };
    let disc = |cv: &mut Canvas, x: f32, y: f32, rr: f32, col: Rgb| {
        let (px, py) = p(x, y);
        cv.fill_circle(px, py, rr * u, col, a);
    };
    let ring = |cv: &mut Canvas, x: f32, y: f32, rr: f32, w: f32, col: Rgb| {
        let (px, py) = p(x, y);
        cv.stroke_circle(px, py, rr * u, w, col, a);
    };
    let rect = |cv: &mut Canvas, x: f32, y: f32, w: f32, h: f32, col: Rgb| {
        poly(cv, &[(x, y), (x + w, y), (x + w, y + h), (x, y + h)], col);
    };

    match i {
        // First Flight: a paper plane.
        0 => {
            poly(cv, &[(-0.9, 0.0), (0.9, -0.7), (0.1, 0.8)], c);
            line(cv, 0.9, -0.7, -0.15, 0.2, t, bg);
        }
        // Regular: a play button.
        1 => poly(cv, &[(-0.5, -0.75), (0.75, 0.0), (-0.5, 0.75)], c),
        // Arcade Rat: a joystick.
        2 => {
            rect(cv, -0.8, 0.45, 1.6, 0.4, c);
            line(cv, 0.0, 0.45, 0.0, -0.3, t * 1.4, c);
            disc(cv, 0.0, -0.5, 0.35, c);
        }
        // Flash Legend: a crown.
        3 => poly(cv, &[(-0.85, 0.6), (-0.85, -0.45), (-0.4, 0.0), (0.0, -0.75), (0.4, 0.0), (0.85, -0.45), (0.85, 0.6)], c),
        // Warm Up: a flame.
        4 => {
            poly(cv, &[(0.0, -0.95), (0.55, -0.1), (0.5, 0.45), (0.0, 0.85), (-0.5, 0.45), (-0.55, -0.1), (-0.15, -0.4)], c);
            disc(cv, 0.0, 0.4, 0.28, bg);
        }
        // Hour of Flash: a clock.
        5 => {
            ring(cv, 0.0, 0.0, 0.8, t, c);
            line(cv, 0.0, 0.0, 0.0, -0.55, t, c);
            line(cv, 0.0, 0.0, 0.4, 0.15, t, c);
        }
        // Marathon: an hourglass.
        6 => {
            poly(cv, &[(-0.6, -0.85), (0.6, -0.85), (0.0, 0.0)], c);
            poly(cv, &[(0.0, 0.0), (0.6, 0.85), (-0.6, 0.85)], c);
        }
        // Living in 2007: an old monitor.
        7 => {
            rect(cv, -0.85, -0.7, 1.7, 1.15, c);
            rect(cv, -0.65, -0.52, 1.3, 0.8, bg);
            rect(cv, -0.3, 0.45, 0.6, 0.2, c);
            rect(cv, -0.55, 0.65, 1.1, 0.15, c);
        }
        // Sit Tight: a cup of tea.
        8 => {
            poly(cv, &[(-0.7, -0.3), (0.45, -0.3), (0.3, 0.7), (-0.55, 0.7)], c);
            ring(cv, 0.55, 0.1, 0.25, t, c);
            line(cv, -0.3, -0.85, -0.2, -0.5, t * 0.8, c);
            line(cv, 0.05, -0.85, 0.15, -0.5, t * 0.8, c);
        }
        // Deep Dive: an arrow down into water.
        9 => {
            line(cv, 0.0, -0.9, 0.0, 0.1, t * 1.3, c);
            poly(cv, &[(-0.4, -0.05), (0.4, -0.05), (0.0, 0.4)], c);
            line(cv, -0.8, 0.6, 0.8, 0.6, t, c);
            line(cv, -0.6, 0.85, 0.6, 0.85, t, c);
        }
        // Explorer: a compass.
        10 => {
            ring(cv, 0.0, 0.0, 0.85, t, c);
            poly(cv, &[(0.0, -0.65), (0.22, 0.0), (0.0, 0.65), (-0.22, 0.0)], c);
        }
        // Collector: a fan of cards.
        11 => {
            rect(cv, -0.75, -0.55, 0.9, 1.2, c);
            rect(cv, -0.6, -0.4, 0.6, 0.9, bg);
            rect(cv, -0.2, -0.75, 0.9, 1.2, c);
            rect(cv, -0.05, -0.6, 0.6, 0.9, bg);
            disc(cv, 0.25, -0.15, 0.15, c);
        }
        // Archivist: an open book.
        12 => {
            poly(cv, &[(-0.9, -0.6), (-0.05, -0.45), (-0.05, 0.75), (-0.9, 0.6)], c);
            poly(cv, &[(0.05, -0.45), (0.9, -0.6), (0.9, 0.6), (0.05, 0.75)], c);
        }
        // Full Shelf: books on a shelf.
        13 => {
            rect(cv, -0.75, -0.6, 0.35, 1.25, c);
            rect(cv, -0.3, -0.35, 0.35, 1.0, c);
            poly(cv, &[(0.2, -0.45), (0.5, -0.55), (0.8, 0.6), (0.5, 0.68)], c);
            rect(cv, -0.9, 0.7, 1.8, 0.15, c);
        }
        // Hoarder: a crate.
        14 => {
            rect(cv, -0.75, -0.7, 1.5, 1.4, c);
            rect(cv, -0.55, -0.5, 1.1, 1.0, bg);
            line(cv, -0.55, -0.5, 0.55, 0.5, t, c);
            line(cv, -0.55, 0.5, 0.55, -0.5, t, c);
        }
        // First Love: a heart.
        15 => heart(cv, &p, u, c, a),
        // Curator: a gem.
        16 => {
            poly(cv, &[(-0.85, -0.25), (-0.45, -0.7), (0.45, -0.7), (0.85, -0.25), (0.0, 0.85)], c);
            line(cv, -0.85, -0.25, 0.85, -0.25, t * 0.8, bg);
            line(cv, -0.25, -0.25, 0.0, 0.8, t * 0.6, bg);
            line(cv, 0.25, -0.25, 0.0, 0.8, t * 0.6, bg);
        }
        // Photographer: a camera.
        17 => {
            rect(cv, -0.85, -0.45, 1.7, 1.15, c);
            rect(cv, -0.3, -0.7, 0.6, 0.3, c);
            disc(cv, 0.0, 0.12, 0.42, bg);
            disc(cv, 0.0, 0.12, 0.25, c);
        }
        // Gallery: a framed picture.
        18 => {
            rect(cv, -0.85, -0.7, 1.7, 1.4, c);
            rect(cv, -0.68, -0.53, 1.36, 1.06, bg);
            poly(cv, &[(-0.68, 0.53), (-0.2, -0.1), (0.15, 0.3), (0.35, 0.1), (0.68, 0.53)], c);
            disc(cv, 0.35, -0.25, 0.14, c);
        }
        // Take a Breather: pause.
        19 => {
            rect(cv, -0.55, -0.7, 0.38, 1.4, c);
            rect(cv, 0.17, -0.7, 0.38, 1.4, c);
        }
        // Pause Master: pause in a ring.
        20 => {
            ring(cv, 0.0, 0.0, 0.88, t, c);
            rect(cv, -0.38, -0.45, 0.26, 0.9, c);
            rect(cv, 0.12, -0.45, 0.26, 0.9, c);
        }
        // Again!: a circular arrow.
        21 => {
            ring(cv, 0.0, 0.05, 0.65, t * 1.2, c);
            rect(cv, 0.05, -0.85, 0.9, 0.55, bg);
            poly(cv, &[(0.0, -0.95), (0.45, -0.6), (0.0, -0.3)], c);
        }
        // Never Give Up: a flag.
        22 => {
            line(cv, -0.6, -0.85, -0.6, 0.9, t * 1.2, c);
            poly(cv, &[(-0.6, -0.85), (0.8, -0.55), (-0.6, -0.15)], c);
        }
        // Typist: a keyboard.
        23 => {
            rect(cv, -0.9, -0.5, 1.8, 1.0, c);
            for row in 0..2 {
                for col in 0..5 {
                    let (x, y) = (-0.72 + col as f32 * 0.32, -0.33 + row as f32 * 0.3);
                    rect(cv, x, y, 0.18, 0.18, bg);
                }
            }
            rect(cv, -0.45, 0.25, 0.9, 0.12, bg);
        }
        // Point and Click: a mouse.
        24 => {
            poly(cv, &[(-0.45, -0.3), (-0.3, -0.75), (0.3, -0.75), (0.45, -0.3), (0.45, 0.45), (0.2, 0.85), (-0.2, 0.85), (-0.45, 0.45)], c);
            line(cv, 0.0, -0.75, 0.0, -0.2, t * 0.8, bg);
            line(cv, -0.45, -0.2, 0.45, -0.2, t * 0.8, bg);
        }
        // Seeker: a magnifying glass.
        25 => {
            ring(cv, -0.15, -0.15, 0.5, t * 1.2, c);
            line(cv, 0.22, 0.22, 0.75, 0.75, t * 1.6, c);
        }
        // Night Owl: a moon and a star.
        26 => {
            disc(cv, -0.1, 0.05, 0.75, c);
            disc(cv, 0.25, -0.2, 0.6, bg);
            disc(cv, 0.6, 0.45, 0.12, c);
        }
        // Early Bird: a rising sun.
        27 => {
            disc(cv, 0.0, 0.35, 0.45, c);
            rect(cv, -0.95, 0.38, 1.9, 0.6, bg);
            line(cv, -0.9, 0.45, 0.9, 0.45, t, c);
            for k in 0..5 {
                let ang = std::f32::consts::PI * (0.1 + 0.2 * k as f32);
                let (dx, dy) = (-ang.cos(), -ang.sin());
                line(cv, dx * 0.62, 0.35 + dy * 0.62, dx * 0.9, 0.35 + dy * 0.9, t, c);
            }
        }
        // Fashionista: a shirt.
        28 => poly(
            cv,
            &[(-0.35, -0.75), (-0.9, -0.4), (-0.65, 0.0), (-0.45, -0.1), (-0.45, 0.8), (0.45, 0.8), (0.45, -0.1), (0.65, 0.0), (0.9, -0.4), (0.35, -0.75), (0.0, -0.5)],
            c,
        ),
        // Painter: a palette.
        29 => {
            disc(cv, 0.0, 0.0, 0.85, c);
            disc(cv, 0.35, 0.4, 0.22, bg);
            disc(cv, -0.4, -0.3, 0.13, bg);
            disc(cv, 0.0, -0.5, 0.13, bg);
            disc(cv, -0.5, 0.2, 0.13, bg);
        }
        // Tinkerer: a gear.
        30 => {
            for k in 0..8 {
                let ang = k as f32 * std::f32::consts::PI / 4.0;
                line(cv, ang.cos() * 0.45, ang.sin() * 0.45, ang.cos() * 0.88, ang.sin() * 0.88, t * 2.2, c);
            }
            disc(cv, 0.0, 0.0, 0.62, c);
            disc(cv, 0.0, 0.0, 0.25, bg);
        }
        // Daydreamer: a cloud.
        31 => {
            disc(cv, -0.4, 0.15, 0.38, c);
            disc(cv, 0.05, -0.15, 0.48, c);
            disc(cv, 0.45, 0.18, 0.35, c);
            rect(cv, -0.4, 0.15, 0.85, 0.38, c);
        }
        // Remapper: a D-pad.
        32 => {
            rect(cv, -0.28, -0.85, 0.56, 1.7, c);
            rect(cv, -0.85, -0.28, 1.7, 0.56, c);
            disc(cv, 0.0, 0.0, 0.14, bg);
        }
        // Completionist: a trophy.
        33 => {
            poly(cv, &[(-0.6, -0.8), (0.6, -0.8), (0.45, -0.05), (0.12, 0.2), (-0.12, 0.2), (-0.45, -0.05)], c);
            ring(cv, -0.6, -0.45, 0.25, t, c);
            ring(cv, 0.6, -0.45, 0.25, t, c);
            rect(cv, -0.1, 0.2, 0.2, 0.35, c);
            rect(cv, -0.45, 0.55, 0.9, 0.25, c);
        }
        // Anything else: a star.
        _ => {
            let mut pts = Vec::with_capacity(10);
            for k in 0..10 {
                let ang = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
                let rr = if k % 2 == 0 { 1.0 } else { 0.42 };
                pts.push((ang.cos() * rr, ang.sin() * rr));
            }
            poly(cv, &pts, c);
        }
    }
}

fn heart(cv: &mut Canvas, p: &dyn Fn(f32, f32) -> (f32, f32), u: f32, c: Rgb, a: f32) {
    let (lx, ly) = p(-0.38, -0.25);
    let (rx, ry) = p(0.38, -0.25);
    cv.fill_circle(lx, ly, 0.42 * u, c, a);
    cv.fill_circle(rx, ry, 0.42 * u, c, a);
    let pts = [p(-0.79, -0.12), p(0.79, -0.12), p(0.0, 0.85)];
    cv.fill_polygon(&pts, c, a);
}
