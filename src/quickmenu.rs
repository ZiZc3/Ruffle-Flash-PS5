//! The in-game quick menu: the touchpad pauses the game behind a frosted
//! card with Resume, Restart, Volume, Retake cover and Back to library.

use std::time::{Duration, Instant};

use crate::input::{
    MenuMouse, PadFrame, PAD_CIRCLE, PAD_CROSS, PAD_DOWN, PAD_LEFT, PAD_OPTIONS, PAD_RIGHT, PAD_TOUCHPAD, PAD_UP,
};
use crate::library::draw_hint_lead;
use crate::sounds::{self, Sfx};
use crate::ui::gfx::{self, Canvas, Image, PadIcon, accent, WHITE};
use crate::ui::text::{Text, Weight};

const W: i32 = 1920;
const H: i32 = 1080;
const CARD_W: i32 = 720;
const ROW_H: i32 = 76;
const OPEN_TIME: f32 = 0.22;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Resume,
    Restart,
    Volume,
    Cover,
    Library,
}

const ITEMS: [(Item, &str); 5] = [
    (Item::Resume, "Resume"),
    (Item::Restart, "Restart game"),
    (Item::Volume, "Volume"),
    (Item::Cover, "Retake cover"),
    (Item::Library, "Back to library"),
];

/// What the player chose.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Resume,
    Restart,
    /// The new game volume, 0..=10.
    Volume(u8),
    Cover,
    Library,
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub struct QuickMenu {
    opened: Instant,
    /// The game's frame as it was, and blurred and dimmed behind the card.
    frame: Vec<u8>,
    backdrop: Vec<u8>,
    sel: usize,
    volume: u8,
    name: String,
    /// Rows as drawn (y, item index), for the pointer.
    rows: Vec<(i32, usize)>,
    card_x: i32,
    held_dir: u32,
    next_repeat: Option<Instant>,
}

impl QuickMenu {
    pub fn new(frame: &[u8], name: &str, volume: u8) -> QuickMenu {
        // Blurred small and scaled back up: cheap, and soft enough.
        let mut small = Image::from_rgba(W as u32, H as u32, frame.to_vec()).resized(W / 4, H / 4);
        small.blur(5);
        let mut backdrop = small.resized(W, H).px;
        gfx::fade(&mut backdrop, 0.42);
        QuickMenu {
            opened: Instant::now(),
            frame: frame.to_vec(),
            backdrop,
            sel: 0,
            volume: volume.min(10),
            name: name.to_string(),
            rows: Vec::new(),
            card_x: (W - CARD_W) / 2,
            held_dir: 0,
            next_repeat: None,
        }
    }

    /// Up/Down with hold-to-repeat.
    fn dpad(&mut self, f: &PadFrame, now: Instant) -> u32 {
        let dirs = PAD_LEFT | PAD_RIGHT | PAD_UP | PAD_DOWN;
        if f.pressed & dirs != 0 {
            self.held_dir = f.pressed & dirs;
            self.next_repeat = Some(now + Duration::from_millis(340));
            return self.held_dir;
        }
        if f.held & self.held_dir != 0 && self.next_repeat.is_some_and(|t| now >= t) {
            self.next_repeat = Some(now + Duration::from_millis(90));
            return self.held_dir;
        }
        0
    }

    /// `pointer` is where the cursor is (the game's), and whether the mouse moved.
    pub fn handle(&mut self, f: &PadFrame, mouse: &MenuMouse, pointer: (f64, f64)) -> Action {
        let now = Instant::now();
        // Ignore the press that opened it.
        if self.opened.elapsed() < Duration::from_millis(120) {
            return Action::None;
        }
        if f.just_pressed(PAD_CIRCLE) || f.just_pressed(PAD_OPTIONS) || f.just_pressed(PAD_TOUCHPAD) {
            sounds::play(Sfx::Back);
            return Action::Resume;
        }
        let sel_before = self.sel;
        let action = self.handle_inner(f, mouse, pointer, now);
        match action {
            Action::None if self.sel != sel_before => sounds::play(Sfx::Move),
            Action::None => {}
            Action::Volume(_) => sounds::play(Sfx::Toggle),
            Action::Resume => sounds::play(Sfx::Back),
            _ => sounds::play(Sfx::Select),
        }
        action
    }

    fn handle_inner(&mut self, f: &PadFrame, mouse: &MenuMouse, pointer: (f64, f64), now: Instant) -> Action {
        let dir = self.dpad(f, now);
        if dir & PAD_DOWN != 0 {
            self.sel = (self.sel + 1) % ITEMS.len();
        }
        if dir & PAD_UP != 0 {
            self.sel = (self.sel + ITEMS.len() - 1) % ITEMS.len();
        }

        // The pointer selects the row it's over; a click picks it (on the
        // volume row, its left half turns it down).
        let (px, py) = (pointer.0 as i32, pointer.1 as i32);
        let over = self
            .rows
            .iter()
            .find(|(y, _)| px >= self.card_x && px < self.card_x + CARD_W && py >= *y && py < y + ROW_H)
            .map(|&(_, i)| i);
        if let Some(i) = over {
            if mouse.dx != 0.0 || mouse.dy != 0.0 || mouse.click {
                self.sel = i;
            }
        }
        let clicked = mouse.click && over.is_some();

        let item = ITEMS[self.sel].0;
        if item == Item::Volume {
            let step = if dir & PAD_RIGHT != 0 || f.just_pressed(PAD_CROSS) {
                1
            } else if dir & PAD_LEFT != 0 {
                -1
            } else if clicked {
                if px < self.card_x + CARD_W / 2 { -1 } else { 1 }
            } else {
                0
            };
            if step != 0 {
                let v = (self.volume as i32 + step).clamp(0, 10) as u8;
                if v != self.volume {
                    self.volume = v;
                    return Action::Volume(v);
                }
            }
            return Action::None;
        }
        if f.just_pressed(PAD_CROSS) || clicked {
            return match item {
                Item::Resume => Action::Resume,
                Item::Restart => Action::Restart,
                Item::Cover => Action::Cover,
                Item::Library => Action::Library,
                Item::Volume => Action::None,
            };
        }
        Action::None
    }

    pub fn draw(&mut self, cv: &mut Canvas, text: &mut Text, keyboard_hints: bool, pointer: Option<(f64, f64)>) {
        let t = ease(self.opened.elapsed().as_secs_f32() / OPEN_TIME);
        if t >= 1.0 {
            cv.px.copy_from_slice(&self.backdrop);
        } else {
            // The game softens into the backdrop as the card comes up.
            let k = (t * 256.0) as u32;
            for ((d, a), b) in cv.px.iter_mut().zip(&self.frame).zip(&self.backdrop) {
                *d = ((*a as u32 * (256 - k) + *b as u32 * k) >> 8) as u8;
            }
        }

        let card_h = 200 + ITEMS.len() as i32 * ROW_H + 80;
        let x = self.card_x;
        let y = (H - card_h) / 2 + ((1.0 - t) * 30.0) as i32;
        cv.glow(x, y + 10, CARD_W, card_h, 28, 40, gfx::INK, 0.6 * t);
        cv.fill_round_rect(x, y, CARD_W, card_h, 28, gfx::INK, 0.72 * t);
        cv.stroke_round_rect(x, y, CARD_W, card_h, 28, 1, WHITE, 0.12 * t);

        let ix = x + 44;
        let iw = CARD_W - 88;
        text.draw(cv, Weight::SemiBold, 20, ix, y + 40, "PAUSED", accent(), t);
        let title = text.fit(Weight::Bold, 44, &self.name, iw);
        text.draw(cv, Weight::Bold, 44, ix, y + 72, &title, WHITE, t);
        cv.fill_rect(ix, y + 152, 64, 4, accent(), t);

        self.rows.clear();
        let mut ry = y + 184;
        for (i, (item, label)) in ITEMS.iter().enumerate() {
            self.rows.push((ry, i));
            let selected = i == self.sel;
            if selected {
                cv.fill_round_rect(x + 18, ry + 4, CARD_W - 36, ROW_H - 8, 16, WHITE, 0.12 * t);
                cv.fill_round_rect(x + 18, ry + 20, 5, ROW_H - 40, 3, accent(), t);
            } else if i > 0 {
                cv.fill_rect(ix, ry, iw, 1, WHITE, 0.07 * t);
            }
            let a = if selected { 1.0 } else { 0.75 } * t;
            text.draw(cv, Weight::SemiBold, 30, ix, ry + 20, label, WHITE, a);
            let cy = (ry + ROW_H / 2) as f32;
            let right = x + CARD_W - 44;
            match item {
                Item::Volume => {
                    // < a bar and the percentage >
                    let bar_w = 200;
                    let value = format!("{}%", self.volume as u32 * 10);
                    let vw = text.width(Weight::SemiBold, 26, &value);
                    let bx = right - 30 - vw - 20 - bar_w;
                    cv.fill_round_rect(bx, cy as i32 - 4, bar_w, 8, 4, WHITE, 0.18 * t);
                    let fill = bar_w * self.volume as i32 / 10;
                    if fill > 0 {
                        cv.fill_round_rect(bx, cy as i32 - 4, fill, 8, 4, accent(), t);
                    }
                    text.draw(cv, Weight::SemiBold, 26, right - 22 - vw, ry + 22, &value, WHITE, a);
                    let ca = if selected { 1.0 } else { 0.35 } * t;
                    let (lx, rx) = ((bx - 22) as f32, right as f32 - 4.0);
                    cv.line(lx + 6.0, cy - 8.0, lx - 2.0, cy, 3.0, accent(), ca);
                    cv.line(lx - 2.0, cy, lx + 6.0, cy + 8.0, 3.0, accent(), ca);
                    cv.line(rx - 6.0, cy - 8.0, rx + 2.0, cy, 3.0, accent(), ca);
                    cv.line(rx + 2.0, cy, rx - 6.0, cy + 8.0, 3.0, accent(), ca);
                }
                _ if selected => {
                    let rx = right as f32 - 4.0;
                    cv.line(rx - 6.0, cy - 9.0, rx + 3.0, cy, 3.0, WHITE, t);
                    cv.line(rx + 3.0, cy, rx - 6.0, cy + 9.0, 3.0, WHITE, t);
                }
                _ => {}
            }
            ry += ROW_H;
        }

        // Hints: select and resume.
        let hy = ry + 44;
        let mut hx = ix;
        for (icon, label) in [(PadIcon::Cross, "Select"), (PadIcon::Circle, "Resume")] {
            let lead = draw_hint_lead(cv, text, Some(icon), "", keyboard_hints, hx, hy);
            hx += lead + text.draw(cv, Weight::SemiBold, 24, hx + lead, hy - 15, label, WHITE, 0.8 * t) + 36;
        }

        if let Some((px, py)) = pointer {
            gfx::pointer(cv, px as f32, py as f32, t);
        }
    }
}
