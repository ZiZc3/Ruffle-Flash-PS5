//! On-screen keyboard for games that ask for keys the controller doesn't have
//! ("press K", "press 1"...). L2 opens it over the game; the D-Pad picks a key,
//! Cross sends it; Square deletes and Triangle types a space.

use std::time::{Duration, Instant};

use ruffle_core::events::{KeyDescriptor, KeyLocation, LogicalKey, NamedKey, PhysicalKey, PlayerEvent};

use crate::input::{
    PadFrame, PAD_CIRCLE, PAD_CROSS, PAD_DOWN, PAD_L2, PAD_LEFT, PAD_RIGHT, PAD_SQUARE, PAD_TRIANGLE, PAD_UP,
};
use crate::ui::gfx::{self, Canvas, PadIcon, accent, WHITE};
use crate::ui::text::{Text, Weight};

#[derive(Clone, Copy)]
pub struct Key {
    pub label: &'static str,
    physical: PhysicalKey,
    logical: Logical,
    /// Width in key units.
    width: i32,
}

#[derive(Clone, Copy)]
enum Logical {
    Char(char),
    Named(NamedKey),
}

impl Key {
    const fn ch(label: &'static str, c: char, physical: PhysicalKey) -> Key {
        Key { label, physical, logical: Logical::Char(c), width: 1 }
    }
    const fn named(label: &'static str, named: NamedKey, physical: PhysicalKey, width: i32) -> Key {
        Key { label, physical, logical: Logical::Named(named), width }
    }

    fn descriptor(&self) -> KeyDescriptor {
        KeyDescriptor {
            physical_key: self.physical,
            logical_key: match self.logical {
                Logical::Char(c) => LogicalKey::Character(c),
                Logical::Named(n) => LogicalKey::Named(n),
            },
            key_location: KeyLocation::Standard,
        }
    }

    pub fn down(&self) -> Vec<PlayerEvent> {
        let mut events = vec![PlayerEvent::KeyDown { key: self.descriptor() }];
        // Text fields (names, codes) take the character too.
        if let Logical::Char(c) = self.logical {
            events.push(PlayerEvent::TextInput { codepoint: c });
        }
        events
    }

    pub fn up(&self) -> PlayerEvent {
        PlayerEvent::KeyUp { key: self.descriptor() }
    }
}

use PhysicalKey as P;

const ROWS: [&[Key]; 5] = [
    &[
        Key::ch("1", '1', P::Digit1), Key::ch("2", '2', P::Digit2), Key::ch("3", '3', P::Digit3),
        Key::ch("4", '4', P::Digit4), Key::ch("5", '5', P::Digit5), Key::ch("6", '6', P::Digit6),
        Key::ch("7", '7', P::Digit7), Key::ch("8", '8', P::Digit8), Key::ch("9", '9', P::Digit9),
        Key::ch("0", '0', P::Digit0),
    ],
    &[
        Key::ch("Q", 'q', P::KeyQ), Key::ch("W", 'w', P::KeyW), Key::ch("E", 'e', P::KeyE),
        Key::ch("R", 'r', P::KeyR), Key::ch("T", 't', P::KeyT), Key::ch("Y", 'y', P::KeyY),
        Key::ch("U", 'u', P::KeyU), Key::ch("I", 'i', P::KeyI), Key::ch("O", 'o', P::KeyO),
        Key::ch("P", 'p', P::KeyP),
    ],
    &[
        Key::ch("A", 'a', P::KeyA), Key::ch("S", 's', P::KeyS), Key::ch("D", 'd', P::KeyD),
        Key::ch("F", 'f', P::KeyF), Key::ch("G", 'g', P::KeyG), Key::ch("H", 'h', P::KeyH),
        Key::ch("J", 'j', P::KeyJ), Key::ch("K", 'k', P::KeyK), Key::ch("L", 'l', P::KeyL),
        Key::named("Enter", NamedKey::Enter, P::Enter, 1),
    ],
    &[
        Key::named("Shift", NamedKey::Shift, P::ShiftLeft, 1),
        Key::ch("Z", 'z', P::KeyZ), Key::ch("X", 'x', P::KeyX), Key::ch("C", 'c', P::KeyC),
        Key::ch("V", 'v', P::KeyV), Key::ch("B", 'b', P::KeyB), Key::ch("N", 'n', P::KeyN),
        Key::ch("M", 'm', P::KeyM),
        Key::named("Delete", NamedKey::Backspace, P::Backspace, 2),
    ],
    &[
        Key::named("Esc", NamedKey::Escape, P::Escape, 2),
        Key::named("Tab", NamedKey::Tab, P::Tab, 2),
        Key::ch("Space", ' ', P::Space),
        Key::named("Ctrl", NamedKey::Control, P::ControlLeft, 2),
    ],
];

pub const ARROWS: [Key; 4] = [
    Key::named("Up", NamedKey::ArrowUp, P::ArrowUp, 1),
    Key::named("Down", NamedKey::ArrowDown, P::ArrowDown, 1),
    Key::named("Left", NamedKey::ArrowLeft, P::ArrowLeft, 1),
    Key::named("Right", NamedKey::ArrowRight, P::ArrowRight, 1),
];

/// Every key a controller button can send, in the order the controls page
/// cycles through them: arrows and special keys, then letters, then digits.
pub fn bindable_keys() -> &'static [Key] {
    static KEYS: std::sync::OnceLock<Vec<Key>> = std::sync::OnceLock::new();
    KEYS.get_or_init(make_bindable_keys)
}

fn make_bindable_keys() -> Vec<Key> {
    let all: Vec<Key> = ROWS.iter().flat_map(|r| r.iter().copied()).collect();
    let mut out: Vec<Key> = ARROWS.to_vec();
    for name in ["Space", "Enter", "Shift", "Ctrl", "Esc", "Tab", "Delete"] {
        out.extend(all.iter().filter(|k| k.label == name).copied());
    }
    let mut letters: Vec<Key> = all.iter().filter(|k| k.label.len() == 1 && k.label.as_bytes()[0].is_ascii_alphabetic()).copied().collect();
    letters.sort_by_key(|k| k.label);
    out.extend(letters);
    out.extend(all.iter().filter(|k| k.label.len() == 1 && k.label.as_bytes()[0].is_ascii_digit()).copied());
    out
}

/// Space spans the row's leftover width.
fn key_width(row: usize, k: &Key) -> i32 {
    if k.label == "Space" {
        let others: i32 = ROWS[row].iter().filter(|o| o.label != "Space").map(|o| o.width).sum();
        10 - others
    } else {
        k.width
    }
}

pub struct OnScreenKeys {
    open: bool,
    row: usize,
    col: usize,
    /// Key tapped from the panel, released a moment later.
    pending_up: Option<(Key, Instant)>,
    /// Last touch seen while dragging: (id, x, y).
    drag: Option<(u8, u16, u16)>,
    last_draw: Instant,
    /// The player closed the keyboard themselves (so text boxes stop
    /// opening it for the rest of the game).
    pub user_closed: bool,
}

/// Where the keyboard has been dragged to (offset from its home position),
/// kept between games: (target, shown) as (x, y).
static OFFSET: std::sync::Mutex<[f32; 4]> = std::sync::Mutex::new([0.0; 4]);

const KEY: i32 = 66;
const GAP: i32 = 8;
const BOARD_W: i32 = 10 * KEY + 9 * GAP;
const PANEL_W: i32 = BOARD_W + 72;
/// Title above the keys, a row of hints below.
const PANEL_H: i32 = 5 * (KEY + GAP) + 92 + 56;
/// Home position: centred, near the bottom of the screen.
const HOME_X: i32 = (1920 - PANEL_W) / 2;
const HOME_Y: i32 = 1080 - 28 - PANEL_H;

impl OnScreenKeys {
    pub fn new() -> Self {
        OnScreenKeys {
            open: false,
            row: 2,
            col: 7,
            pending_up: None,
            drag: None,
            last_draw: Instant::now(),
            user_closed: false,
        }
    }

    /// A finger moving on the touchpad drags the open keyboard.
    fn drag_with(&mut self, touch: Option<(u8, u16, u16)>) {
        let Some((id, x, y)) = touch else {
            self.drag = None;
            return;
        };
        if let Some((pid, px, py)) = self.drag {
            if pid == id {
                let mut o = OFFSET.lock().unwrap();
                o[0] += (x as f32 - px as f32) * 1.1;
                o[1] += (y as f32 - py as f32) * 1.1;
                // Keep the whole keyboard on screen.
                o[0] = o[0].clamp(-(HOME_X as f32) + 16.0, (1920 - HOME_X - PANEL_W) as f32 - 16.0);
                o[1] = o[1].clamp(-(HOME_Y as f32) + 16.0, (1080 - HOME_Y - PANEL_H) as f32 - 8.0);
            }
        }
        self.drag = Some((id, x, y));
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens or closes the keyboard (a game's text box asking for it).
    pub fn set_open(&mut self, open: bool) {
        if open != self.open {
            self.open = open;
            self.drag = None;
        }
    }

    /// Takes the controller first. Returns the key events for the game and
    /// whether the panel used the input (so the game shouldn't get it).
    pub fn handle(&mut self, frame: &PadFrame) -> (Vec<PlayerEvent>, bool) {
        let mut events = Vec::new();
        let now = Instant::now();

        if let Some((key, at)) = self.pending_up {
            if now >= at {
                events.push(key.up());
                self.pending_up = None;
            }
        }

        if frame.just_pressed(PAD_L2) {
            self.open = !self.open;
            self.user_closed |= !self.open;
            // A finger left on the pad from before mustn't jump the keyboard.
            self.drag = None;
            return (events, true);
        }

        if self.open {
            self.drag_with(frame.touch);
            if frame.just_pressed(PAD_CIRCLE) {
                self.open = false;
                self.user_closed = true;
            }
            if frame.just_pressed(PAD_UP) {
                self.row = (self.row + ROWS.len() - 1) % ROWS.len();
            }
            if frame.just_pressed(PAD_DOWN) {
                self.row = (self.row + 1) % ROWS.len();
            }
            self.col = self.col.min(ROWS[self.row].len() - 1);
            if frame.just_pressed(PAD_LEFT) {
                self.col = (self.col + ROWS[self.row].len() - 1) % ROWS[self.row].len();
            }
            if frame.just_pressed(PAD_RIGHT) {
                self.col = (self.col + 1) % ROWS[self.row].len();
            }
            if frame.just_pressed(PAD_CROSS) {
                let key = ROWS[self.row][self.col];
                if let Some((prev, _)) = self.pending_up.take() {
                    events.push(prev.up());
                }
                events.extend(key.down());
                self.pending_up = Some((key, now + Duration::from_millis(90)));
            }
            // Shortcuts for typing: Square deletes, Triangle is a space.
            for (mask, label) in [(PAD_SQUARE, "Delete"), (PAD_TRIANGLE, "Space")] {
                if frame.just_pressed(mask) {
                    let key = *ROWS.iter().flat_map(|r| r.iter()).find(|k| k.label == label).expect("key");
                    if let Some((prev, _)) = self.pending_up.take() {
                        events.push(prev.up());
                    }
                    events.extend(key.down());
                    self.pending_up = Some((key, now + Duration::from_millis(90)));
                }
            }
            return (events, true);
        }
        (events, false)
    }

    /// Lets go of every key this sent (leaving a game).
    pub fn release_all(&mut self) -> Vec<PlayerEvent> {
        let mut events = Vec::new();
        if let Some((key, _)) = self.pending_up.take() {
            events.push(key.up());
        }
        self.open = false;
        events
    }

    pub fn draw(&mut self, cv: &mut Canvas, text: &mut Text) {
        // The shown position follows the dragged one smoothly.
        let dt = self.last_draw.elapsed().as_secs_f32().min(0.1);
        self.last_draw = Instant::now();
        let (ox, oy) = {
            let mut o = OFFSET.lock().unwrap();
            let k = 1.0 - (-dt * 16.0).exp();
            o[2] += (o[0] - o[2]) * k;
            o[3] += (o[1] - o[3]) * k;
            (o[2] as i32, o[3] as i32)
        };
        if !self.open {
            return;
        }

        let board_w = BOARD_W;
        let x0 = HOME_X + 36 + ox;
        let y0 = HOME_Y + 92 + oy;

        cv.fill_round_rect(x0 - 36, y0 - 92, PANEL_W, PANEL_H, 28, gfx::INK, 0.86);
        text.draw(cv, Weight::Bold, 26, x0, y0 - 62, "Keyboard", WHITE, 1.0);
        // A grip: swipe on the touchpad to move the keyboard.
        cv.fill_round_rect(x0 - 36 + PANEL_W / 2 - 30, y0 - 82, 60, 6, 3, WHITE, 0.3);
        // Right of the title: where it moves with.
        {
            let label = "Move";
            let lw = text.width(Weight::SemiBold, 17, label);
            let cw = text.width(Weight::SemiBold, 14, "Touchpad") + 18;
            let hx = x0 + board_w - lw - cw - 10;
            cv.stroke_round_rect(hx, y0 - 60, cw, 24, 7, 2, WHITE, 0.5);
            text.draw(cv, Weight::SemiBold, 14, hx + 9, y0 - 56, "Touchpad", WHITE, 0.75);
            text.draw(cv, Weight::SemiBold, 17, hx + cw + 10, y0 - 58, label, WHITE, 0.8);
        }

        // Hints under the keys, centred: X Send, Square Delete, ...
        let hints = [
            (Some(PadIcon::Cross), "", "Send"),
            (Some(PadIcon::Square), "", "Delete"),
            (Some(PadIcon::Triangle), "", "Space"),
            (Some(PadIcon::Circle), "", "Close"),
        ];
        let hy = y0 + 5 * (KEY + GAP) + 18;
        let widths: Vec<i32> = hints
            .iter()
            .map(|(icon, chip, label)| {
                let lead = if icon.is_some() { 32 } else { text.width(Weight::SemiBold, 14, chip) + 28 };
                lead + text.width(Weight::SemiBold, 17, label)
            })
            .collect();
        let total: i32 = widths.iter().sum::<i32>() + 26 * (hints.len() as i32 - 1);
        let mut hx = x0 + (board_w - total) / 2;
        for ((icon, chip, label), w) in hints.iter().zip(widths) {
            let lead = match icon {
                Some(i) => {
                    gfx::pad_icon(cv, *i, (hx + 12) as f32, (hy + 11) as f32, 12.0);
                    32
                }
                None => {
                    let cw = text.width(Weight::SemiBold, 14, chip) + 18;
                    cv.stroke_round_rect(hx, hy - 1, cw, 24, 7, 2, WHITE, 0.5);
                    text.draw(cv, Weight::SemiBold, 14, hx + 9, hy + 3, chip, WHITE, 0.75);
                    cw + 10
                }
            };
            text.draw(cv, Weight::SemiBold, 17, hx + lead, hy + 1, label, WHITE, 0.8);
            hx += w + 26;
        }

        for (r, row) in ROWS.iter().enumerate() {
            let mut x = x0;
            let y = y0 + r as i32 * (KEY + GAP);
            for (c, key) in row.iter().enumerate() {
                let units = key_width(r, key);
                let w = units * KEY + (units - 1) * GAP;
                let selected = r == self.row && c == self.col;
                if selected {
                    cv.glow(x, y, w, KEY, 14, 22, accent(), 0.6);
                    cv.fill_round_rect(x, y, w, KEY, 14, accent(), 1.0);
                } else {
                    cv.fill_round_rect(x, y, w, KEY, 14, WHITE, 0.09);
                }
                let size = if key.label.chars().count() > 1 { 18 } else { 28 };
                let weight = if selected { Weight::Bold } else { Weight::SemiBold };
                let tw = text.width(weight, size, key.label);
                text.draw(cv, weight, size, x + (w - tw) / 2, y + (KEY - size as i32) / 2 - 2, key.label, WHITE, 1.0);
                x += w + GAP;
            }
        }
    }
}
