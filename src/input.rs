use std::time::{Duration, Instant};

use ruffle_core::events::{MouseButton, MouseWheelDelta, PlayerEvent};

use crate::controls::{Bind, Controls};
use crate::hid::{self, Hid, HidFrame};
use crate::keys::Key;

const SCREEN_WIDTH: f64 = 1920.0;
const SCREEN_HEIGHT: f64 = 1080.0;

/// Stick travel ignored around the centre (fraction of full tilt).
const STICK_DEADZONE: f64 = 0.14;
/// Cursor speed at full tilt, in frame pixels per second.
const CURSOR_MAX_SPEED: f64 = 1700.0;
/// Speed curve exponent: small tilts aim precisely, full tilt crosses fast.
const CURSOR_CURVE: f64 = 2.2;

pub const PAD_L3: u32 = 0x0002;
pub const PAD_R3: u32 = 0x0004;
pub const PAD_OPTIONS: u32 = 0x0008;
pub const PAD_UP: u32 = 0x0010;
pub const PAD_RIGHT: u32 = 0x0020;
pub const PAD_DOWN: u32 = 0x0040;
pub const PAD_LEFT: u32 = 0x0080;
pub const PAD_L2: u32 = 0x0100;
pub const PAD_R2: u32 = 0x0200;
pub const PAD_L1: u32 = 0x0400;
pub const PAD_R1: u32 = 0x0800;
pub const PAD_TRIANGLE: u32 = 0x1000;
pub const PAD_CIRCLE: u32 = 0x2000;
pub const PAD_CROSS: u32 = 0x4000;
pub const PAD_SQUARE: u32 = 0x8000;
pub const PAD_TOUCHPAD: u32 = 0x0010_0000;

/// The start of ScePadData, with room for the rest of it (XPSemu's
/// ui/xemu-input-ps5.c layout, confirmed on the console).
#[repr(C)]
struct PadData {
    buttons: u32,
    lx: u8,
    ly: u8,
    rx: u8,
    ry: u8,
    l2: u8,
    r2: u8,
    _pad: [u8; 2],
    _rest: [u8; 256],
}

unsafe extern "C" {
    fn sceUserServiceInitialize(params: *const u8) -> i32;
    fn sceUserServiceGetInitialUser(user_id: *mut i32) -> i32;
    fn scePadInit() -> i32;
    fn scePadOpen(user_id: i32, pad_type: i32, index: i32, param: *const u8) -> i32;
    fn scePadGetHandle(user_id: i32, pad_type: i32, index: i32) -> i32;
    fn scePadReadState(handle: i32, data: *mut PadData) -> i32;
}

/// The USB mouse and typing in menus: motion in screen pixels, a left
/// click, characters typed and Backspace (for search).
#[derive(Clone, Default)]
pub struct MenuMouse {
    pub dx: f64,
    pub dy: f64,
    pub click: bool,
    pub typed: Vec<char>,
    pub backspace: bool,
}

/// One read of the controller: buttons held, and what changed since the last.
#[derive(Clone, Copy, Default)]
pub struct PadFrame {
    pub held: u32,
    pub pressed: u32,
    pub released: u32,
    pub lx: u8,
    pub ly: u8,
    pub rx: u8,
    pub ry: u8,
    /// The first finger on the touchpad: (id, x, y), x 0..~1920, y 0..~1080.
    pub touch: Option<(u8, u16, u16)>,
}

impl PadFrame {
    pub fn just_pressed(&self, mask: u32) -> bool {
        self.pressed & mask != 0
    }
}

pub struct Ps5Input {
    user: i32,
    handle: i32,
    next_open: Instant,
    cursor_x: f64,
    cursor_y: f64,
    prev_buttons: u32,
    /// The controller's own buttons last read, and whether it was used then.
    prev_pad_buttons: u32,
    pad_used: bool,
    last_cursor_update: Instant,
    hid: Hid,
    /// What the USB keyboard and mouse did in the last read.
    pub hid_frame: HidFrame,
    /// In menus the keyboard drives them like the controller (arrows, Enter,
    /// Esc...); in games its keys go to the game.
    pub keys_as_pad: bool,
    /// Typing into a search: Space and Backspace type instead of pressing
    /// Cross and Circle.
    pub text_entry: bool,
    /// Settings' cursor speed (1.0 = normal).
    pub cursor_speed: f64,
    /// Settings' mouse speed, frame pixels per mouse count.
    pub mouse_speed: f64,
    /// Right-stick scrolling not yet sent, in wheel lines.
    wheel: f64,
    last_wheel_update: Instant,
    touches_logged: u32,
    /// Left-stick directions held as keys (up, down, left, right).
    stick_held: [bool; 4],
}

/// The keys the left stick sends: up, down, left, right.
fn stick_keys_for(mode: u8) -> [Key; 4] {
    let find = |l: &str| *crate::keys::bindable_keys().iter().find(|k| k.label == l).expect("key");
    match mode {
        2 => [find("W"), find("S"), find("A"), find("D")],
        _ => crate::keys::ARROWS,
    }
}

/// ScePadData's touch data, which starts 52 bytes in (40 into `_rest`):
/// touchNum u8, 7 reserved bytes, then touch[0] = { x u16, y u16, id u8, ... }.
fn read_touch(rest: &[u8; 256]) -> Option<(u8, u16, u16)> {
    let count = rest[40];
    if count == 0 || count > 2 {
        return None;
    }
    let x = u16::from_le_bytes([rest[48], rest[49]]);
    let y = u16::from_le_bytes([rest[50], rest[51]]);
    Some((rest[52], x, y))
}

impl Ps5Input {
    pub fn new() -> Result<Self, String> {
        let rc_user = unsafe { sceUserServiceInitialize(core::ptr::null()) };
        let rc_pad = unsafe { scePadInit() };
        println!("[PS5] pad: user service {:#x}, pad {:#x}", rc_user, rc_pad);

        let mut input = Ps5Input {
            user: -1,
            handle: -1,
            next_open: Instant::now(),
            cursor_x: SCREEN_WIDTH / 2.0,
            cursor_y: SCREEN_HEIGHT / 2.0,
            prev_buttons: 0,
            prev_pad_buttons: 0,
            pad_used: false,
            last_cursor_update: Instant::now(),
            hid: Hid::open(-1),
            hid_frame: HidFrame::default(),
            keys_as_pad: true,
            text_entry: false,
            cursor_speed: 1.0,
            mouse_speed: 1.2,
            wheel: 0.0,
            last_wheel_update: Instant::now(),
            touches_logged: 0,
            stick_held: [false; 4],
        };
        input.open();
        if input.user < 0 && unsafe { sceUserServiceGetInitialUser(&mut input.user) } != 0 {
            input.user = -1;
        }
        input.hid = Hid::open(input.user);
        Ok(input)
    }

    /// Opens the first user's controller; again every two seconds until it's there.
    fn open(&mut self) {
        if self.handle >= 0 || Instant::now() < self.next_open {
            return;
        }
        self.next_open = Instant::now() + Duration::from_secs(2);

        if self.user < 0 && unsafe { sceUserServiceGetInitialUser(&mut self.user) } != 0 {
            self.user = -1;
            return;
        }
        self.handle = unsafe { scePadOpen(self.user, 0, 0, core::ptr::null()) };
        if self.handle < 0 {
            self.handle = unsafe { scePadGetHandle(self.user, 0, 0) };
        }
        println!("[PS5] pad: user {}, handle {}", self.user, self.handle);
    }

    /// Reads the controller, and the USB keyboard and mouse (in menus the
    /// keyboard presses the controller's buttons too).
    pub fn read(&mut self) -> PadFrame {
        self.open();
        self.hid_frame = self.hid.read();
        let mut data: PadData = unsafe { core::mem::zeroed() };
        if self.handle < 0 || unsafe { scePadReadState(self.handle, &mut data) } != 0 {
            // No controller: sticks centred, nothing held.
            data = unsafe { core::mem::zeroed() };
            (data.lx, data.ly, data.rx, data.ry) = (128, 128, 128, 128);
        }
        self.pad_used = data.buttons & !self.prev_pad_buttons != 0;
        self.prev_pad_buttons = data.buttons;
        let mut buttons = data.buttons;
        let mut extra_presses = 0;
        if self.keys_as_pad {
            let (held, tapped) = self.menu_buttons();
            buttons |= held;
            extra_presses = tapped;
        }
        let frame = PadFrame {
            held: buttons,
            pressed: (buttons & !self.prev_buttons) | extra_presses,
            released: !buttons & self.prev_buttons,
            lx: data.lx,
            ly: data.ly,
            rx: data.rx,
            ry: data.ry,
            touch: read_touch(&data._rest),
        };
        if frame.touch.is_some() && self.touches_logged < 3 {
            self.touches_logged += 1;
            println!("[PS5] touch {:?}", frame.touch);
        }
        self.prev_buttons = buttons;
        frame
    }

    /// The controller buttons the keyboard and mouse stand for in menus:
    /// (held, pressed just now). Arrows = D-Pad, Enter/Space = Cross,
    /// Esc/Backspace = Circle, F2 = Triangle, F3 = Square, F4 = Options,
    /// Tab/Shift+Tab and PageDown/PageUp = R1/L1 (letters type a search);
    /// the wheel steps up and down and the right button goes back.
    fn menu_buttons(&self) -> (u32, u32) {
        const MAP: [(u16, u32); 11] = [
            (hid::KEY_UP, PAD_UP),
            (hid::KEY_DOWN, PAD_DOWN),
            (hid::KEY_LEFT, PAD_LEFT),
            (hid::KEY_RIGHT, PAD_RIGHT),
            (hid::KEY_ENTER, PAD_CROSS),
            (hid::KEY_SPACE, PAD_CROSS),
            (hid::KEY_ESCAPE, PAD_CIRCLE),
            (hid::KEY_BACKSPACE, PAD_CIRCLE),
            (hid::KEY_F2, PAD_TRIANGLE),
            (hid::KEY_F3, PAD_SQUARE),
            (hid::KEY_F4, PAD_OPTIONS),
        ];
        let f = &self.hid_frame;
        let mut held_keys = self.hid.held_keys();
        if self.text_entry {
            held_keys.retain(|&k| k != hid::KEY_SPACE && k != hid::KEY_BACKSPACE);
        }
        let mut held = 0;
        for (usage, button) in MAP {
            if held_keys.contains(&usage) {
                held |= button;
            }
        }
        let mut tapped = 0;
        for &(usage, down) in &f.keys {
            if !down || (self.text_entry && (usage == hid::KEY_SPACE || usage == hid::KEY_BACKSPACE)) {
                continue;
            }
            // A held key is already in `held` (the menus repeat it
            // themselves); this catches taps shorter than one read.
            if let Some(&(_, button)) = MAP.iter().find(|(u, _)| *u == usage) {
                if !held_keys.contains(&usage) {
                    tapped |= button;
                }
            }
            match usage {
                hid::KEY_TAB => tapped |= if f.shift { PAD_L1 } else { PAD_R1 },
                hid::KEY_PAGE_UP => tapped |= PAD_L1,
                hid::KEY_PAGE_DOWN => tapped |= PAD_R1,
                _ => {}
            }
        }
        if f.wheel > 0 {
            tapped |= PAD_UP;
        } else if f.wheel < 0 {
            tapped |= PAD_DOWN;
        }
        if f.pressed & hid::MOUSE_RIGHT != 0 {
            tapped |= PAD_CIRCLE;
        }
        (held, tapped)
    }

    /// Ruffle events from the USB mouse and keyboard in a game: the mouse
    /// moves the same cursor as the stick and clicks, its wheel scrolls, and
    /// keys go straight to the game.
    pub fn hid_events(&mut self) -> Vec<PlayerEvent> {
        let mut events = Vec::new();
        let (dx, dy, wheel, pressed, released) = {
            let f = &self.hid_frame;
            (f.dx, f.dy, f.wheel, f.pressed, f.released)
        };
        if dx != 0 || dy != 0 {
            self.cursor_x = (self.cursor_x + dx as f64 * self.mouse_speed).clamp(0.0, SCREEN_WIDTH - 1.0);
            self.cursor_y = (self.cursor_y + dy as f64 * self.mouse_speed).clamp(0.0, SCREEN_HEIGHT - 1.0);
            events.push(PlayerEvent::MouseMove { x: self.cursor_x, y: self.cursor_y });
        }
        for (bit, button) in [
            (hid::MOUSE_LEFT, MouseButton::Left),
            (hid::MOUSE_RIGHT, MouseButton::Right),
            (hid::MOUSE_MIDDLE, MouseButton::Middle),
        ] {
            if pressed & bit != 0 {
                events.push(PlayerEvent::MouseDown { x: self.cursor_x, y: self.cursor_y, button, index: None });
            }
            if released & bit != 0 {
                events.push(PlayerEvent::MouseUp { x: self.cursor_x, y: self.cursor_y, button });
            }
        }
        if wheel != 0 {
            events.push(PlayerEvent::MouseWheel { delta: MouseWheelDelta::Lines(wheel as f64) });
        }
        hid::key_events(&self.hid_frame, &mut events);
        events
    }

    /// Moves the cursor with the mouse only (the game is paused under a menu).
    pub fn move_pointer(&mut self) {
        let (dx, dy) = (self.hid_frame.dx, self.hid_frame.dy);
        self.cursor_x = (self.cursor_x + dx as f64 * self.mouse_speed).clamp(0.0, SCREEN_WIDTH - 1.0);
        self.cursor_y = (self.cursor_y + dy as f64 * self.mouse_speed).clamp(0.0, SCREEN_HEIGHT - 1.0);
    }

    /// The mouse for menus, at the settings' mouse speed.
    pub fn menu_mouse(&self) -> MenuMouse {
        let f = &self.hid_frame;
        // Characters typed: letters, digits and a few signs (Space only once
        // a search has started; the library ignores a leading one).
        let typed = f
            .keys
            .iter()
            .filter(|(_, down)| *down)
            .filter_map(|&(u, _)| hid::key_for(u, f.shift, f.caps).and_then(|(_, c)| c))
            .filter(|c| c.is_alphanumeric() || " -.'&!:".contains(*c))
            .filter(|c| *c != ' ' || self.text_entry)
            .collect();
        MenuMouse {
            dx: f.dx as f64 * self.mouse_speed,
            dy: f.dy as f64 * self.mouse_speed,
            click: f.pressed & hid::MOUSE_LEFT != 0,
            typed,
            backspace: self.text_entry && f.key_pressed(hid::KEY_BACKSPACE),
        }
    }

    /// Which was used in the last read: Some(true) the keyboard, Some(false)
    /// the controller, None neither (for the hints' icons or keys).
    pub fn last_used_keyboard(&self) -> Option<bool> {
        if self.hid_frame.keys.iter().any(|&(_, down)| down) {
            Some(true)
        } else if self.pad_used {
            Some(false)
        } else {
            None
        }
    }

    /// A USB mouse is plugged in: the menus show their pointer.
    pub fn mouse_connected(&self) -> bool {
        self.hid.mouse_connected()
    }

    /// A USB mouse has been moved or clicked since the app started.
    pub fn mouse_used(&self) -> bool {
        self.hid.mouse_seen()
    }

    /// A USB keyboard has been typed on (the on-screen one isn't needed).
    pub fn keyboard_plugged(&self) -> bool {
        self.hid.keyboard_seen()
    }

    /// Moves the cursor with the left stick, by the time since the last call,
    /// so it glides at the display's rate whatever the game's frame rate.
    /// Returns a MouseMove when it moved.
    fn move_cursor(&mut self, stick_x: u8, stick_y: u8) -> Option<PlayerEvent> {
        let now = Instant::now();
        let dt = (now - self.last_cursor_update).as_secs_f64().min(0.05);
        self.last_cursor_update = now;

        let sx = (stick_x as f64 - 128.0) / 127.0;
        let sy = (stick_y as f64 - 128.0) / 127.0;
        let magnitude = (sx * sx + sy * sy).sqrt().min(1.0);
        if magnitude <= STICK_DEADZONE {
            return None;
        }
        let scaled = ((magnitude - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).powf(CURSOR_CURVE);
        let speed = CURSOR_MAX_SPEED * self.cursor_speed * scaled * dt;
        self.cursor_x = (self.cursor_x + sx / magnitude * speed).clamp(0.0, SCREEN_WIDTH - 1.0);
        self.cursor_y = (self.cursor_y + sy / magnitude * speed).clamp(0.0, SCREEN_HEIGHT - 1.0);
        Some(PlayerEvent::MouseMove {
            x: self.cursor_x,
            y: self.cursor_y,
        })
    }

    fn bind_down(&self, bind: Bind, events: &mut Vec<PlayerEvent>) {
        match bind {
            Bind::Click => events.push(PlayerEvent::MouseDown {
                x: self.cursor_x,
                y: self.cursor_y,
                button: MouseButton::Left,
                index: None,
            }),
            Bind::Key(_) => events.extend(bind.key().map(|k| k.down()).unwrap_or_default()),
            Bind::Nothing => {}
        }
    }

    fn bind_up(&self, bind: Bind, events: &mut Vec<PlayerEvent>) {
        match bind {
            Bind::Click => events.push(PlayerEvent::MouseUp {
                x: self.cursor_x,
                y: self.cursor_y,
                button: MouseButton::Left,
            }),
            Bind::Key(_) => events.extend(bind.key().map(|k| k.up())),
            Bind::Nothing => {}
        }
    }

    /// Ruffle events for a game, by its controls: the cursor (left stick, or
    /// right stick when the left one is keys), each button's key or click,
    /// and the mouse wheel on the right stick.
    pub fn game_events(&mut self, frame: &PadFrame, controls: &Controls) -> Vec<PlayerEvent> {
        let mut events = Vec::new();
        let stick_keys = controls.stick != 0;
        let (cx, cy) = if stick_keys { (frame.rx, frame.ry) } else { (frame.lx, frame.ly) };
        if let Some(moved) = self.move_cursor(cx, cy) {
            events.push(moved);
        }

        // Right stick up/down: the mouse wheel, up to ~12 lines a second
        // (when it isn't moving the cursor).
        let now = Instant::now();
        let dt = (now - self.last_wheel_update).as_secs_f64().min(0.05);
        self.last_wheel_update = now;
        let tilt = (frame.ry as f64 - 128.0) / 127.0;
        if !stick_keys && tilt.abs() > STICK_DEADZONE * 2.0 {
            self.wheel -= tilt * 12.0 * dt;
            let lines = self.wheel.trunc();
            if lines != 0.0 {
                self.wheel -= lines;
                events.push(PlayerEvent::MouseWheel { delta: MouseWheelDelta::Lines(lines) });
            }
        } else {
            self.wheel = 0.0;
        }

        for (mask, _, bind) in controls.rows() {
            if frame.pressed & mask != 0 {
                self.bind_down(bind, &mut events);
            }
            if frame.released & mask != 0 {
                self.bind_up(bind, &mut events);
            }
        }

        // Left stick as four keys, with a little hysteresis.
        if stick_keys {
            let keys = stick_keys_for(controls.stick);
            let sx = (frame.lx as f64 - 128.0) / 127.0;
            let sy = (frame.ly as f64 - 128.0) / 127.0;
            let dirs = [sy < -0.5, sy > 0.5, sx < -0.5, sx > 0.5];
            let keep = [sy < -0.35, sy > 0.35, sx < -0.35, sx > 0.35];
            for i in 0..4 {
                let now_held = if self.stick_held[i] { keep[i] } else { dirs[i] };
                if now_held != self.stick_held[i] {
                    if now_held {
                        events.extend(keys[i].down());
                    } else {
                        events.push(keys[i].up());
                    }
                    self.stick_held[i] = now_held;
                }
            }
        }
        events
    }

    /// Releases everything the game may think is held (leaving a game, or
    /// opening the on-screen keyboard mid-press).
    pub fn release_all(&mut self, controls: &Controls) -> Vec<PlayerEvent> {
        let mut events = vec![PlayerEvent::MouseUp {
            x: self.cursor_x,
            y: self.cursor_y,
            button: MouseButton::Left,
        }];
        for (_, _, bind) in controls.rows() {
            if let Some(k) = bind.key() {
                events.push(k.up());
            }
        }
        if controls.stick != 0 {
            for (i, k) in stick_keys_for(controls.stick).iter().enumerate() {
                if self.stick_held[i] {
                    events.push(k.up());
                }
            }
        }
        self.stick_held = [false; 4];
        for button in [MouseButton::Right, MouseButton::Middle] {
            events.push(PlayerEvent::MouseUp { x: self.cursor_x, y: self.cursor_y, button });
        }
        for usage in self.hid.held_keys() {
            if let Some((key, _)) = hid::key_for(usage, false, false) {
                events.push(PlayerEvent::KeyUp { key });
            }
        }
        events
    }

    /// Where the mouse cursor is, in frame (and Ruffle viewport) pixels.
    pub fn cursor(&self) -> (f64, f64) {
        (self.cursor_x, self.cursor_y)
    }
}
