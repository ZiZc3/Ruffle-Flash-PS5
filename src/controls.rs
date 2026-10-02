//! Per-game controls: what each controller button sends, and what the left
//! stick does. Kept in /data/ruffle/controls/<game>.txt as "button=Key" lines.

use std::fs;
use std::path::PathBuf;

use crate::input::{
    PAD_CIRCLE, PAD_CROSS, PAD_DOWN, PAD_L1, PAD_L3, PAD_LEFT, PAD_OPTIONS, PAD_R1, PAD_R2, PAD_RIGHT, PAD_SQUARE,
    PAD_TRIANGLE, PAD_UP,
};
use crate::keys::{self, Key};

pub const CONTROLS_DIR: &str = "/data/ruffle/controls";
const PAD_CREATE: u32 = 0x0001;

/// The buttons a game can use: (mask, name, file key). L2 (keyboard), R3
/// (cover) and the touchpad (back) stay the app's.
pub const BUTTONS: [(u32, &str, &str); 14] = [
    (PAD_CROSS, "Cross", "cross"),
    (PAD_CIRCLE, "Circle", "circle"),
    (PAD_SQUARE, "Square", "square"),
    (PAD_TRIANGLE, "Triangle", "triangle"),
    (PAD_L1, "L1", "l1"),
    (PAD_R1, "R1", "r1"),
    (PAD_R2, "R2", "r2"),
    (PAD_UP, "D-Pad Up", "up"),
    (PAD_DOWN, "D-Pad Down", "down"),
    (PAD_LEFT, "D-Pad Left", "left"),
    (PAD_RIGHT, "D-Pad Right", "right"),
    (PAD_OPTIONS, "Options", "options"),
    (PAD_CREATE, "Create", "create"),
    (PAD_L3, "L3", "l3"),
];

#[derive(Clone, Copy, PartialEq)]
pub enum Bind {
    /// The mouse's left button, at the cursor.
    Click,
    Nothing,
    Key(usize),
}

pub const STICK_MODES: [&str; 3] = ["Mouse", "Arrow keys", "WASD"];

#[derive(Clone, PartialEq)]
pub struct Controls {
    /// One per BUTTONS entry.
    pub binds: [Bind; 14],
    pub stick: u8,
}

fn key_index(label: &str) -> Bind {
    keys::bindable_keys()
        .iter()
        .position(|k| k.label.eq_ignore_ascii_case(label))
        .map(Bind::Key)
        .unwrap_or(Bind::Nothing)
}

impl Default for Controls {
    fn default() -> Self {
        let k = key_index;
        Controls {
            binds: [
                Bind::Click,
                k("Z"),
                k("Space"),
                k("Enter"),
                k("Shift"),
                k("X"),
                // O and P, as Ruffle on PC players often use for games
                // like Super Smash Flash.
                k("O"),
                k("Up"),
                k("Down"),
                k("Left"),
                k("Right"),
                k("Esc"),
                // P pauses many games; Ctrl shoots or crouches in others.
                k("P"),
                k("Ctrl"),
            ],
            stick: 0,
        }
    }
}

/// The file key of the controls every game starts from.
pub const DEFAULT_KEY: &str = "_all_games";

fn file(game_key: &str) -> PathBuf {
    PathBuf::from(CONTROLS_DIR).join(format!("{}.txt", game_key))
}

impl Bind {
    pub fn label(self) -> String {
        match self {
            Bind::Click => "Mouse click".into(),
            Bind::Nothing => "Nothing".into(),
            Bind::Key(i) => match keys::bindable_keys().get(i) {
                Some(k) => match k.label {
                    "Up" | "Down" | "Left" | "Right" => format!("Arrow {}", k.label),
                    "Delete" => "Backspace".into(),
                    l if l.len() == 1 => format!("Key {}", l),
                    l => l.into(),
                },
                None => "Nothing".into(),
            },
        }
    }

    pub fn key(self) -> Option<Key> {
        match self {
            Bind::Key(i) => keys::bindable_keys().get(i).copied(),
            _ => None,
        }
    }

    /// The next choice: Mouse click, Nothing, then every key.
    pub fn step(self, dir: i32) -> Bind {
        let n = keys::bindable_keys().len() as i32 + 2;
        let i = match self {
            Bind::Click => 0,
            Bind::Nothing => 1,
            Bind::Key(k) => k as i32 + 2,
        };
        match (i + dir).rem_euclid(n) {
            0 => Bind::Click,
            1 => Bind::Nothing,
            j => Bind::Key((j - 2) as usize),
        }
    }

    fn save_name(self) -> String {
        match self {
            Bind::Click => "click".into(),
            Bind::Nothing => "none".into(),
            Bind::Key(i) => keys::bindable_keys().get(i).map(|k| k.label.to_string()).unwrap_or_default(),
        }
    }
}

impl Controls {
    /// What a game's controls start from: the app's defaults for the
    /// "all games" set, and that set for a game.
    pub fn base_for(game_key: &str) -> Self {
        if game_key == DEFAULT_KEY {
            Controls::default()
        } else {
            Controls::load(DEFAULT_KEY)
        }
    }

    pub fn load(game_key: &str) -> Self {
        let mut c = Controls::base_for(game_key);
        let Ok(text) = fs::read_to_string(file(game_key)) else { return c };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if k == "stick" {
                c.stick = STICK_MODES.iter().position(|m| m.eq_ignore_ascii_case(v)).unwrap_or(0) as u8;
                continue;
            }
            let bind = match v {
                "click" => Bind::Click,
                "none" => Bind::Nothing,
                label => key_index(label),
            };
            if let Some(i) = BUTTONS.iter().position(|(_, _, f)| *f == k) {
                c.binds[i] = bind;
            }
        }
        c
    }

    pub fn save(&self, game_key: &str) {
        let _ = fs::create_dir_all(CONTROLS_DIR);
        // Nothing to keep when it's the same as what it starts from.
        if *self == Controls::base_for(game_key) {
            let _ = fs::remove_file(file(game_key));
            return;
        }
        let mut body = format!("stick={}\n", STICK_MODES[self.stick as usize]);
        for (i, (_, _, f)) in BUTTONS.iter().enumerate() {
            body += &format!("{}={}\n", f, self.binds[i].save_name());
        }
        if let Err(e) = fs::write(file(game_key), body) {
            println!("[Controls] can't write {}: {}", file(game_key).display(), e);
        }
    }

    /// Every button with its mask, name and bind.
    pub fn rows(&self) -> impl Iterator<Item = (u32, &'static str, Bind)> + '_ {
        BUTTONS.iter().enumerate().map(|(i, (m, n, _))| (*m, *n, self.binds[i]))
    }

    pub fn is_custom(&self, game_key: &str) -> bool {
        *self != Controls::base_for(game_key)
    }
}
