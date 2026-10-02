use std::time::{Duration, Instant};

use ruffle_core::events::{MouseButton, MouseWheelDelta, PlayerEvent};

use crate::controls::{Bind, Controls};
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

    // libSceKeyboard, loaded at run time by ps5_early.c.
    fn ruffle_ps5_keyboard_load() -> i32;
    fn ruffle_ps5_keyboard_init() -> i32;
    fn ruffle_ps5_keyboard_open(user_id: i32, kb_type: i32, index: i32, param: *const u8) -> i32;
    fn ruffle_ps5_keyboard_read_state(handle: i32, data: *mut u8) -> i32;
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
    last_cursor_update: Instant,
    keyboard: Option<KeyboardProbe>,
    /// Settings' cursor speed (1.0 = normal).
    pub cursor_speed: f64,
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
            last_cursor_update: Instant::now(),
            keyboard: None,
            cursor_speed: 1.0,
            wheel: 0.0,
            last_wheel_update: Instant::now(),
            touches_logged: 0,
            stick_held: [false; 4],
        };
        input.open();
        input.keyboard = KeyboardProbe::open(input.user);
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

    /// Reads the controller (and logs the keyboard probe).
    pub fn read(&mut self) -> PadFrame {
        self.open();
        if let Some(kb) = self.keyboard.as_mut() {
            kb.poll();
        }
        if self.handle < 0 {
            return PadFrame::default();
        }
        let mut data: PadData = unsafe { core::mem::zeroed() };
        if unsafe { scePadReadState(self.handle, &mut data) } != 0 {
            return PadFrame::default();
        }
        let frame = PadFrame {
            held: data.buttons,
            pressed: data.buttons & !self.prev_buttons,
            released: !data.buttons & self.prev_buttons,
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
        self.prev_buttons = data.buttons;
        frame
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
        events
    }

    /// Where the mouse cursor is, in frame (and Ruffle viewport) pixels.
    pub fn cursor(&self) -> (f64, f64) {
        (self.cursor_x, self.cursor_y)
    }
}

/// USB keyboard, experimental: libSceKeyboard has no public header, so this
/// only logs what sceKeyboardReadState returns whenever it changes, to learn
/// its layout from a console run with a keyboard plugged in.
struct KeyboardProbe {
    handle: i32,
    last: Vec<u8>,
    last_rc: i32,
    logged: u32,
    polls: u32,
    /// Bytes that change by themselves (timestamps, counters), learned over
    /// the first second, left out when looking for key presses.
    noisy: [bool; 96],
}

impl KeyboardProbe {
    fn open(user: i32) -> Option<Self> {
        if unsafe { ruffle_ps5_keyboard_load() } != 0 {
            println!("[PS5] keyboard: libSceKeyboard couldn't be loaded");
            return None;
        }
        let rc_init = unsafe { ruffle_ps5_keyboard_init() };
        let handle = unsafe { ruffle_ps5_keyboard_open(user, 0, 0, core::ptr::null()) };
        println!("[PS5] keyboard: init {:#x}, open(user {}) -> {:#x}", rc_init, user, handle);
        if handle < 0 {
            return None;
        }
        Some(KeyboardProbe {
            handle,
            last: Vec::new(),
            last_rc: i32::MIN,
            logged: 0,
            polls: 0,
            noisy: [false; 96],
        })
    }

    fn poll(&mut self) {
        if self.logged >= 300 {
            return;
        }
        let mut buf = [0u8; 256];
        let rc = unsafe { ruffle_ps5_keyboard_read_state(self.handle, buf.as_mut_ptr()) };
        let data = &buf[..96];
        self.polls += 1;

        let first = self.last.is_empty();
        let mut changed = rc != self.last_rc;
        if !first {
            for (i, (a, b)) in data.iter().zip(self.last.iter()).enumerate() {
                if a != b {
                    if self.polls <= 60 {
                        self.noisy[i] = true;
                    } else if !self.noisy[i] {
                        changed = true;
                    }
                }
            }
        }
        if first || changed || self.polls == 61 {
            let hex: Vec<String> = data
                .iter()
                .enumerate()
                .map(|(i, b)| if self.noisy[i] { "..".to_string() } else { format!("{:02x}", b) })
                .collect();
            println!("[PS5] keyboard state rc {:#x}: {}", rc, hex.join(" "));
            self.logged += 1;
        }
        self.last_rc = rc;
        self.last = data.to_vec();
    }
}
