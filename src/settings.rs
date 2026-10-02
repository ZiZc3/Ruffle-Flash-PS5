//! App settings, kept in /data/ruffle/settings.txt as key=value lines. The
//! Display and Player options are Ruffle's own (as in Ruffle on PC).

use std::fs;

const FILE: &str = "/data/ruffle/settings.txt";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Header(&'static str),
    ScaleMode,
    ForceScale,
    Letterbox,
    Align,
    ForceAlign,
    Quality,
    FrameRate,
    PlayerVersion,
    Runtime,
    LoadBehavior,
    MaxExecution,
    DummyExternalInterface,
    SpoofSite,
    Volume,
    CursorSpeed,
    DefaultControls,
    AutoKeyboard,
    FpsCounter,
    AutoCovers,
    Waves,
    Rescan,
    ClearRecent,
    About,
}

/// Categories (headers) each followed by their settings; the list shows one
/// category open at a time. About stands on its own at the end.
pub const ROWS: [Row; 28] = [
    Row::Header("Display"),
    Row::ScaleMode,
    Row::ForceScale,
    Row::Letterbox,
    Row::Align,
    Row::ForceAlign,
    Row::Quality,
    Row::Header("Player"),
    Row::FrameRate,
    Row::PlayerVersion,
    Row::Runtime,
    Row::LoadBehavior,
    Row::MaxExecution,
    Row::DummyExternalInterface,
    Row::SpoofSite,
    Row::Header("Controls & Sound"),
    Row::DefaultControls,
    Row::CursorSpeed,
    Row::AutoKeyboard,
    Row::Volume,
    Row::Header("App"),
    Row::FpsCounter,
    Row::AutoCovers,
    Row::Waves,
    Row::Header("Library"),
    Row::Rescan,
    Row::ClearRecent,
    Row::About,
];

/// What a category is for (its panel text).
pub fn header_help(name: &str) -> &'static str {
    match name {
        "Display" => "How games fill the screen and how smooth their edges look.",
        "Player" => "How Ruffle runs games: speed, Flash version, and helpers for games that won't start.",
        "Controls & Sound" => "What the controller does in games, the on-screen keyboard, and volume.",
        "App" => "The app itself: covers, the animated background and the FPS counter.",
        "Library" => "Your game list: look for new games, or clear what you played recently.",
        _ => "",
    }
}

/// The category row `i` belongs to (its header's index); None for headers
/// and About.
pub fn category_of(i: usize) -> Option<usize> {
    if ROWS[i].is_header() || ROWS[i] == Row::About {
        return None;
    }
    (0..i).rev().find(|&j| ROWS[j].is_header())
}

/// The About page's credits: (role, who).
pub const CREDITS: [(&str, &str); 4] = [
    ("PS5 port", "AZiZ"),
    ("Ruffle", "The Ruffle team and contributors, ruffle.rs"),
    ("Vulkan on PS5", "PS5_Vulkan by Mihawk-99"),
    ("Font", "Inter by Rasmus Andersson"),
];

pub const SCALE_MODES: [&str; 4] = ["Unscaled (100%)", "Zoom to Fit", "Stretch to Fit", "Crop to Fit"];
pub const LETTERBOX: [&str; 3] = ["On", "Fullscreen Only", "Off"];
pub const ALIGNS: [&str; 9] = [
    "Center", "Top", "Bottom", "Left", "Right", "Top-Left", "Top-Right", "Bottom-Left", "Bottom-Right",
];
pub const QUALITIES: [&str; 8] = [
    "Low",
    "Medium",
    "High",
    "Best",
    "High (8x8)",
    "High (8x8) Linear",
    "High (16x16)",
    "High (16x16) Linear",
];
/// 0 = the game's own frame rate.
pub const FRAME_RATES: [u32; 8] = [0, 15, 24, 30, 40, 50, 60, 120];
/// 0 = Ruffle's default (32); then the Flash Player versions.
pub const PLAYER_VERSIONS: [u8; 13] = [0, 6, 7, 8, 9, 10, 11, 15, 20, 25, 32, 40, 51];
pub const RUNTIMES: [&str; 2] = ["Flash Player", "Adobe AIR"];
pub const LOAD_BEHAVIORS: [&str; 3] = ["Streaming", "Delayed", "Blocking"];
pub const MAX_EXECUTION: [u32; 6] = [5, 10, 15, 20, 30, 60];
pub const CURSOR_SPEEDS: [&str; 3] = ["Slow", "Normal", "Fast"];
/// Sites a sitelocked game may insist on, and where its file seems to come
/// from there (the game's own file name is appended).
pub const SPOOF_SITES: [(&str, &str); 7] = [
    ("Off", ""),
    ("Newgrounds", "https://uploads.ungrounded.net/alternate/1/"),
    ("Kongregate", "https://chat.kongregate.com/gamez/0001/0001/live/"),
    ("Armor Games", "https://cache.armorgames.com/files/games/"),
    ("Miniclip", "https://www.miniclip.com/games/en/"),
    ("Addicting Games", "https://www.addictinggames.com/games/"),
    ("Y8", "https://www.y8.com/games/"),
];

#[derive(Clone, PartialEq)]
pub struct Settings {
    pub scale_mode: u8,
    pub force_scale: bool,
    pub letterbox: u8,
    pub align: u8,
    pub force_align: bool,
    pub quality: u8,
    pub frame_rate: u8,
    pub player_version: u8,
    pub runtime: u8,
    pub load_behavior: u8,
    pub max_execution: u8,
    pub dummy_external_interface: bool,
    pub spoof_site: u8,
    /// 0..=10, tenths.
    pub volume: u8,
    pub cursor_speed: u8,
    pub auto_keyboard: bool,
    /// The library shows a list instead of the cover grid.
    pub list_view: bool,
    pub fps_counter: bool,
    pub auto_covers: bool,
    pub waves: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            scale_mode: 1,
            force_scale: false,
            letterbox: 0,
            align: 0,
            force_align: false,
            quality: 2,
            frame_rate: 0,
            player_version: 0,
            runtime: 0,
            load_behavior: 0,
            max_execution: 2,
            dummy_external_interface: false,
            spoof_site: 0,
            volume: 10,
            cursor_speed: 1,
            auto_keyboard: true,
            list_view: true,
            fps_counter: false,
            auto_covers: true,
            waves: true,
        }
    }
}

impl Row {
    pub fn is_header(self) -> bool {
        matches!(self, Row::Header(_))
    }

    /// An action row (Cross runs it) rather than a value.
    pub fn is_action(self) -> bool {
        matches!(self, Row::Rescan | Row::ClearRecent | Row::DefaultControls)
    }

    pub fn label(self) -> &'static str {
        match self {
            Row::Header(h) => h,
            Row::ScaleMode => "Scale mode",
            Row::ForceScale => "Force scale mode",
            Row::Letterbox => "Letterbox",
            Row::Align => "Stage alignment",
            Row::ForceAlign => "Force alignment",
            Row::Quality => "Quality",
            Row::FrameRate => "Frame rate",
            Row::PlayerVersion => "Player version",
            Row::Runtime => "Player runtime",
            Row::LoadBehavior => "Load behavior",
            Row::MaxExecution => "Max script time",
            Row::DummyExternalInterface => "Dummy External Interface",
            Row::SpoofSite => "Pretend to be on",
            Row::Volume => "Game volume",
            Row::CursorSpeed => "Cursor speed",
            Row::AutoKeyboard => "Keyboard for text boxes",
            Row::DefaultControls => "Default controls",
            Row::FpsCounter => "FPS counter",
            Row::AutoCovers => "Automatic covers",
            Row::Waves => "Animated background",
            Row::Rescan => "Rescan games",
            Row::ClearRecent => "Clear recently played",
            Row::About => "About",
        }
    }

    pub fn help(self, s: &Settings) -> String {
        let text = match self {
            Row::Header(_) => "",
            Row::ScaleMode => match s.scale_mode {
                0 => "Unscaled (100%): shows the game at its original size, without any zoom.",
                1 => "Zoom to Fit: zooms the game to fill the screen as much as possible without cropping, keeping its shape.",
                2 => "Stretch to Fit: fills the whole screen, ignoring the game's shape.",
                _ => "Crop to Fit: fills the whole screen keeping the game's shape, cropping its edges if needed.",
            },
            Row::ForceScale => "Stops the game from changing the scale mode, locking it to the one chosen above. Some games pick their own.",
            Row::Letterbox => "Black bars around the game where it doesn't fill the screen; hides anything drawn outside the game's area. Fullscreen Only: only when the game asks for fullscreen.",
            Row::Align => "Where the game sits on the screen when it doesn't fill it (mostly with Unscaled).",
            Row::ForceAlign => "Stops the game from changing its alignment, locking it to the one chosen above.",
            Row::Quality => "Smoothing of edges. Low can help heavy games run faster; the 8x8 and 16x16 modes are the smoothest.",
            Row::FrameRate => "Runs the game at a frame rate of your choice instead of its own. Many games run faster or slower with it.",
            Row::PlayerVersion => "The Flash Player version the game sees. Some old games only work with an older version.",
            Row::Runtime => "Pretend to be Flash Player in a browser, or Adobe AIR (for games made as AIR apps).",
            Row::LoadBehavior => "Streaming: the game starts while loading, preloaders play. Delayed: starts once loaded. Blocking: loads everything before the first frame.",
            Row::MaxExecution => "How long a game's script may run before it's stopped as stuck.",
            Row::DummyExternalInterface => "Tells games the web page they expect is there, answering its calls with nothing. Some games wait for it and won't start without it.",
            Row::SpoofSite => "Makes a game believe it runs on this site. Many games only start on the site they were made for and show a \"play on our site\" screen anywhere else.",
            Row::Volume => "Volume of every game's sound and music.",
            Row::CursorSpeed => "How fast the left stick moves the mouse cursor in games.",
            Row::DefaultControls => "What each button does in every game that has no controls of its own. Out of the box: Cross clicks, Circle Z, Square Space, Triangle Enter, L1 Shift, R1 X, R2 O, D-Pad arrows, Options Esc, Create P, L3 Ctrl. A game's own controls (Options on it in the library) start from these.",
            Row::AutoKeyboard => "Opens the on-screen keyboard by itself when you click a box in a game you can type in (a name, a code). Close it with O and it stays closed for that game.",
            Row::FpsCounter => "Shows the game's frame rate in the corner while playing.",
            Row::AutoCovers => "Saves a picture of a game as its cover 8 seconds into its first run. R3 in a game always retakes it.",
            Row::Waves => "The flowing light waves behind the menus.",
            Row::Rescan => "Looks again for .swf files in /data/ruffle, its games folder and USB drives.",
            Row::ClearRecent => "Empties the Recent tab. Saves and favorites are kept.",
            Row::About => "Flash games on PS5, played by Ruffle, the open-source Flash Player emulator.",
        };
        text.to_string()
    }
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Settings::default();
        let Ok(text) = fs::read_to_string(FILE) else { return s };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let n: u8 = v.trim().parse().unwrap_or(0);
            match k.trim() {
                "scale_mode" => s.scale_mode = n.min(3),
                "force_scale" => s.force_scale = n != 0,
                "letterbox" => s.letterbox = n.min(2),
                "align" => s.align = n.min(8),
                "force_align" => s.force_align = n != 0,
                "quality" => s.quality = n.min(7),
                "frame_rate" => s.frame_rate = n.min(FRAME_RATES.len() as u8 - 1),
                "player_version" => s.player_version = n.min(PLAYER_VERSIONS.len() as u8 - 1),
                "runtime" => s.runtime = n.min(1),
                "load_behavior" => s.load_behavior = n.min(2),
                "max_execution" => s.max_execution = n.min(MAX_EXECUTION.len() as u8 - 1),
                "dummy_external_interface" => s.dummy_external_interface = n != 0,
                "spoof_site" => s.spoof_site = n.min(SPOOF_SITES.len() as u8 - 1),
                "volume" => s.volume = n.min(10),
                "cursor_speed" => s.cursor_speed = n.min(2),
                "auto_keyboard" => s.auto_keyboard = n != 0,
                // (The old "list_view" key is ignored: list is the default now.)
                "library_list" => s.list_view = n != 0,
                "fps_counter" => s.fps_counter = n != 0,
                "auto_covers" => s.auto_covers = n != 0,
                "waves" => s.waves = n != 0,
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let fields: [(&str, u8); 20] = [
            ("auto_keyboard", self.auto_keyboard as u8),
            ("library_list", self.list_view as u8),
            ("dummy_external_interface", self.dummy_external_interface as u8),
            ("spoof_site", self.spoof_site),
            ("scale_mode", self.scale_mode),
            ("force_scale", self.force_scale as u8),
            ("letterbox", self.letterbox),
            ("align", self.align),
            ("force_align", self.force_align as u8),
            ("quality", self.quality),
            ("frame_rate", self.frame_rate),
            ("player_version", self.player_version),
            ("runtime", self.runtime),
            ("load_behavior", self.load_behavior),
            ("max_execution", self.max_execution),
            ("volume", self.volume),
            ("cursor_speed", self.cursor_speed),
            ("fps_counter", self.fps_counter as u8),
            ("auto_covers", self.auto_covers as u8),
            ("waves", self.waves as u8),
        ];
        let body: String = fields.iter().map(|(k, v)| format!("{}={}\n", k, v)).collect();
        if let Err(e) = fs::write(FILE, body) {
            println!("[Settings] can't write {}: {}", FILE, e);
        }
    }

    pub fn value(&self, row: Row) -> String {
        let on = |b: bool| if b { "On" } else { "Off" }.to_string();
        match row {
            Row::ScaleMode => SCALE_MODES[self.scale_mode as usize].into(),
            Row::ForceScale => on(self.force_scale),
            Row::Letterbox => LETTERBOX[self.letterbox as usize].into(),
            Row::Align => ALIGNS[self.align as usize].into(),
            Row::ForceAlign => on(self.force_align),
            Row::Quality => QUALITIES[self.quality as usize].into(),
            Row::FrameRate => match FRAME_RATES[self.frame_rate as usize] {
                0 => "Game's own".into(),
                f => format!("{} fps", f),
            },
            Row::PlayerVersion => match PLAYER_VERSIONS[self.player_version as usize] {
                0 => "Default (32)".into(),
                v => v.to_string(),
            },
            Row::Runtime => RUNTIMES[self.runtime as usize].into(),
            Row::LoadBehavior => LOAD_BEHAVIORS[self.load_behavior as usize].into(),
            Row::MaxExecution => format!("{} s", MAX_EXECUTION[self.max_execution as usize]),
            Row::DummyExternalInterface => on(self.dummy_external_interface),
            Row::SpoofSite => SPOOF_SITES[self.spoof_site as usize].0.into(),
            Row::Volume => format!("{}%", self.volume as u32 * 10),
            Row::CursorSpeed => CURSOR_SPEEDS[self.cursor_speed as usize].into(),
            Row::AutoKeyboard => on(self.auto_keyboard),
            Row::FpsCounter => on(self.fps_counter),
            Row::AutoCovers => on(self.auto_covers),
            Row::Waves => on(self.waves),
            Row::Header(_) | Row::Rescan | Row::ClearRecent | Row::DefaultControls | Row::About => String::new(),
        }
    }

    /// Steps a value left (-1) or right (+1); true when it changed.
    pub fn step(&mut self, row: Row, dir: i32) -> bool {
        fn step(v: &mut u8, count: usize, dir: i32) {
            *v = (*v as i32 + dir).clamp(0, count as i32 - 1) as u8;
        }
        let before = self.clone();
        match row {
            Row::ScaleMode => step(&mut self.scale_mode, SCALE_MODES.len(), dir),
            Row::ForceScale => self.force_scale = dir > 0,
            Row::Letterbox => step(&mut self.letterbox, LETTERBOX.len(), dir),
            Row::Align => step(&mut self.align, ALIGNS.len(), dir),
            Row::ForceAlign => self.force_align = dir > 0,
            Row::Quality => step(&mut self.quality, QUALITIES.len(), dir),
            Row::FrameRate => step(&mut self.frame_rate, FRAME_RATES.len(), dir),
            Row::PlayerVersion => step(&mut self.player_version, PLAYER_VERSIONS.len(), dir),
            Row::Runtime => step(&mut self.runtime, RUNTIMES.len(), dir),
            Row::LoadBehavior => step(&mut self.load_behavior, LOAD_BEHAVIORS.len(), dir),
            Row::MaxExecution => step(&mut self.max_execution, MAX_EXECUTION.len(), dir),
            Row::DummyExternalInterface => self.dummy_external_interface = dir > 0,
            Row::SpoofSite => step(&mut self.spoof_site, SPOOF_SITES.len(), dir),
            Row::Volume => step(&mut self.volume, 11, dir),
            Row::CursorSpeed => step(&mut self.cursor_speed, CURSOR_SPEEDS.len(), dir),
            Row::AutoKeyboard => self.auto_keyboard = dir > 0,
            Row::FpsCounter => self.fps_counter = dir > 0,
            Row::AutoCovers => self.auto_covers = dir > 0,
            Row::Waves => self.waves = dir > 0,
            _ => return false,
        }
        let changed = *self != before;
        if changed {
            self.save();
        }
        changed
    }

    /// The URL a game should believe it was loaded from, if spoofing.
    pub fn spoofed_url(&self, game_path: &str) -> Option<String> {
        let (_, base) = SPOOF_SITES[self.spoof_site as usize];
        if base.is_empty() {
            return None;
        }
        let file = std::path::Path::new(game_path)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "game.swf".into());
        Some(format!("{}{}", base, file))
    }

    pub fn cursor_multiplier(&self) -> f64 {
        [0.6, 1.0, 1.5][self.cursor_speed as usize]
    }
}
