//! The library: tabs and a grid of covers on the left, the selected game's
//! panel on the right, over the animated background; and the settings page.

use std::collections::HashMap;
use std::fs;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::badges::{Badges, Live, Session, BADGES};
use crate::covers::{self, TILE_H, TILE_W};
use crate::sounds::{self, Sfx};
use crate::controls::{self, Bind, Controls};
use crate::input::{
    MenuMouse, PadFrame, PAD_CIRCLE, PAD_CROSS, PAD_DOWN, PAD_L1, PAD_LEFT, PAD_OPTIONS, PAD_R1, PAD_RIGHT,
    PAD_SQUARE, PAD_TRIANGLE, PAD_UP,
};
use crate::settings::{self, Row, Settings};
use crate::ui::gfx::{self, Background, Canvas, Image, PadIcon, accent, accent_light, WHITE};
use crate::ui::text::{Text, Weight};

const W: i32 = 1920;
const H: i32 = 1080;
const MARGIN: i32 = 64;

const COLS: usize = 4;
const GAP: i32 = 24;
const GRID_X: i32 = MARGIN;
const GRID_Y: i32 = 150;
const GRID_BOTTOM: i32 = H - 40;
/// Rows leave room under the tiles for the selected game's name.
const ROW_H: i32 = TILE_H + 80;
/// The selected tile grows by this much (PS5 home screen style).
const SEL_SCALE: f32 = 1.07;

const PANEL_X: i32 = 1392;
const PANEL_W: i32 = W - MARGIN - PANEL_X;
const PANEL_Y: i32 = 130;
const PANEL_H: i32 = H - 48 - PANEL_Y;
const COVER_W: i32 = PANEL_W - 48;
const COVER_H: i32 = COVER_W * 5 / 8;

/// Ruffle's logo in the app's orange, transparent (ps5/art/make-art.py).
static LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");

const FAVORITES_FILE: &str = "/data/ruffle/favorites.txt";
const RECENT_FILE: &str = "/data/ruffle/recent.txt";
const SEARCH_DIRS: [&str; 6] = [
    "/data/ruffle/games",
    "/data/ruffle",
    "/mnt/usb0/ruffle",
    "/mnt/usb1/ruffle",
    "/mnt/usb0",
    "/mnt/usb1",
];

unsafe extern "C" {
    fn ruffle_ps5_clock(hour: *mut i32, minute: *mut i32) -> i32;
}

/// Something the mouse pointer can point at, as drawn last frame.
#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Tab(usize),
    /// A game, by its place in the tab's list.
    Game(usize),
    /// A settings row (index in settings::ROWS), and its "<" arrow.
    Setting(usize),
    SettingLeft(usize),
    /// A row of the controls page, and its "<" arrow.
    Editor(usize),
    EditorLeft(usize),
    /// A hint or menu row: clicking presses this controller button.
    Button(u32),
    Badge(usize),
}

/// The pointer stays this long after the mouse last moved, then fades.
const POINTER_IDLE: f32 = 4.0;

/// A button hint's controller button, for clicks on it.
fn button_of(icon: Option<PadIcon>, chip: &str) -> Option<u32> {
    match (icon, chip) {
        (Some(PadIcon::Cross), _) => Some(PAD_CROSS),
        (Some(PadIcon::Circle), _) => Some(PAD_CIRCLE),
        (Some(PadIcon::Triangle), _) => Some(PAD_TRIANGLE),
        (Some(PadIcon::Square), _) => Some(PAD_SQUARE),
        (None, "Options") => Some(PAD_OPTIONS),
        (None, "L1 R1") => Some(PAD_R1),
        _ => None,
    }
}

/// The keyboard key standing for a button hint (input.rs menu_buttons).
pub fn keyboard_key(icon: Option<PadIcon>, chip: &str) -> Option<&'static str> {
    match (icon, chip) {
        (Some(PadIcon::Cross), _) => Some("Enter"),
        (Some(PadIcon::Circle), _) => Some("Esc"),
        (Some(PadIcon::Triangle), _) => Some("F2"),
        (Some(PadIcon::Square), _) => Some("F3"),
        (None, "Options") => Some("F4"),
        (None, "L1 R1") => Some("Tab"),
        _ => None,
    }
}

/// A hint's lead at (x, centre y): the button's icon, or a key chip (the
/// keyboard's key when one is in use); returns how far the label starts.
pub fn draw_hint_lead(cv: &mut Canvas, text: &mut Text, icon: Option<PadIcon>, chip: &str, keyboard: bool, x: i32, cy: i32) -> i32 {
    let key = if keyboard { keyboard_key(icon, chip) } else { None };
    match (icon, key) {
        (Some(ic), None) => {
            gfx::pad_icon(cv, ic, (x + 18) as f32, cy as f32, 17.0);
            48
        }
        _ => {
            let chip = key.unwrap_or(chip);
            let cw = text.width(Weight::SemiBold, 15, chip) + 18;
            cv.stroke_round_rect(x, cy - 14, cw, 28, 7, 2, WHITE, 0.45);
            text.draw(cv, Weight::SemiBold, 15, x + 9, cy - 9, chip, WHITE, 0.75);
            cw + 14
        }
    }
}

struct ControlsEditor {
    key: String,
    name: String,
    controls: Controls,
    /// 0 = the left stick, then one row per button.
    sel: usize,
}

pub struct Game {
    pub path: String,
    pub name: String,
    pub key: String,
    pub size: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Recent,
    Games,
    Favorites,
    Badges,
    Settings,
}

const TABS: [(Tab, &str); 5] = [
    (Tab::Recent, "Recent"),
    (Tab::Games, "Games"),
    (Tab::Favorites, "Favorites"),
    (Tab::Badges, "Badges"),
    (Tab::Settings, "Settings"),
];
const GAMES_TAB: usize = 1;

const PLAYTIME_FILE: &str = "/data/ruffle/playtime.txt";
/// The screensaver starts after this long without input in the menus.
const SAVER_AFTER: Duration = Duration::from_secs(120);

/// The badges grid.
const BCOLS: usize = 5;
const BW: i32 = 236;
const BH: i32 = 206;
const BGAP: i32 = 21;
const BROW: i32 = BH + BGAP;

/// "3 h 20 m", "12 min", "under a minute".
fn format_play(secs: u64) -> String {
    match secs {
        0..=59 => "under a minute".into(),
        60..=3599 => format!("{} min", secs / 60),
        _ => format!("{} h {} m", secs / 3600, secs / 60 % 60),
    }
}
const TAB_COUNT: usize = TABS.len();

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// "age of war" -> "Age Of War" (file names are usually lower case).
fn title_case(s: &str) -> String {
    s.split(' ')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{:.0} KB", (bytes as f64 / 1_000.0).max(1.0))
    }
}

fn ago(secs: u64) -> String {
    let d = now_secs().saturating_sub(secs);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86400),
    }
}

pub struct Library {
    games: Vec<Game>,
    favorites: Vec<String>,
    recent: Vec<(String, u64)>,
    tab: usize,
    selected: [usize; TAB_COUNT],
    scroll: f32,
    tab_pill: (f32, f32),
    tiles: HashMap<String, Image>,
    panel_covers: HashMap<String, Image>,
    /// The selected tile at its larger size, made when the selection changes.
    big_tile: Option<(String, Image)>,
    selection_changed: Instant,
    pub settings: Settings,
    setting_sel: usize,
    /// The open category (its header's index in settings::ROWS).
    open_category: Option<usize>,
    settings_scroll: f32,
    about_logo: Option<Image>,
    /// The About page (opened at), and when it started closing.
    about_page: Option<Instant>,
    about_closing: Option<Instant>,
    about_logo_big: Option<Image>,
    tiles_made_this_frame: u32,
    /// The controls page, while it's open.
    editor: Option<ControlsEditor>,
    /// 0 = grid, 1 = list, animated between.
    view_t: f32,
    list_scroll: f32,
    editor_scroll: f32,
    held_dir: u32,
    next_repeat: Option<Instant>,
    toast: Option<(String, Instant)>,
    /// The mouse pointer, and when the mouse last moved (None: hidden).
    pointer: (f32, f32),
    pointer_moved: Option<Instant>,
    /// What can be pointed at, from the last frame drawn.
    hits: Vec<(i32, i32, i32, i32, Hit)>,
    /// Hints show keyboard keys (a USB keyboard was used last).
    pub keyboard_hints: bool,
    /// A USB mouse is plugged in: the pointer stays on screen.
    pub mouse_connected: bool,
    pointer_shown: f32,
    pub badges: Badges,
    /// Seconds played per game key.
    playtime: HashMap<String, u64>,
    /// Typed search (filters the Games tab).
    search: String,
    /// The on-screen keyboard is typing a search (main.rs draws it).
    pub search_open: bool,
    tabs_end: i32,
    last_input: Instant,
    /// The screensaver, since when.
    saver: Option<Instant>,
    badge_sel: usize,
    badge_scroll: f32,
    canvas: Canvas,
    start: Instant,
    last_render: Instant,
}

impl Library {
    pub fn new() -> Self {
        let mut lib = Library {
            games: Vec::new(),
            favorites: fs::read_to_string(FAVORITES_FILE)
                .map(|s| s.lines().filter(|l| !l.is_empty()).map(String::from).collect())
                .unwrap_or_default(),
            recent: fs::read_to_string(RECENT_FILE)
                .map(|s| {
                    s.lines()
                        .filter_map(|l| {
                            let (t, p) = l.split_once('\t')?;
                            Some((p.to_string(), t.parse().ok()?))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            tab: 1,
            selected: [0; TAB_COUNT],
            scroll: 0.0,
            tab_pill: (0.0, 0.0),
            tiles: HashMap::new(),
            panel_covers: HashMap::new(),
            big_tile: None,
            selection_changed: Instant::now(),
            settings: Settings::load(),
            setting_sel: 0,
            open_category: None,
            settings_scroll: 0.0,
            about_logo: None,
            about_page: None,
            about_closing: None,
            about_logo_big: None,
            tiles_made_this_frame: 0,
            editor: None,
            view_t: 0.0,
            list_scroll: 0.0,
            editor_scroll: 0.0,
            held_dir: 0,
            next_repeat: None,
            toast: None,
            pointer: (W as f32 / 2.0, H as f32 / 2.0),
            pointer_moved: None,
            hits: Vec::new(),
            keyboard_hints: false,
            mouse_connected: false,
            pointer_shown: 0.0,
            badges: Badges::load(),
            playtime: fs::read_to_string(PLAYTIME_FILE)
                .map(|s| {
                    s.lines()
                        .filter_map(|l| {
                            let (t, k) = l.split_once('\t')?;
                            Some((k.to_string(), t.parse().ok()?))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            search: String::new(),
            search_open: false,
            tabs_end: GRID_X,
            last_input: Instant::now(),
            saver: None,
            badge_sel: 0,
            badge_scroll: 0.0,
            canvas: Canvas::new(W, H),
            start: Instant::now(),
            last_render: Instant::now(),
        };
        lib.scan_files();
        gfx::set_accent(settings::ACCENTS[lib.settings.accent as usize].1);
        sounds::set_enabled(lib.settings.ui_sounds);
        lib.badges.stats.themes_mask |= 1 << lib.settings.theme;
        lib.badges.stats.accents_mask |= 1 << lib.settings.accent;
        lib.check_badges();
        lib.view_t = if lib.settings.list_view { 1.0 } else { 0.0 };
        // Open on Recent when something was played before.
        if !lib.visible_for(0).is_empty() {
            lib.tab = 0;
        }
        lib
    }

    fn add_game(&mut self, path: &std::path::Path, size: u64) {
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("swf")) {
            return;
        }
        let path_str = path.to_string_lossy().into_owned();
        if self.games.iter().any(|g| g.path == path_str) {
            return;
        }
        let name = path
            .file_stem()
            .map(|s| title_case(&s.to_string_lossy().replace(['-', '_'], " ")))
            .unwrap_or_default();
        self.games.push(Game { key: covers::game_key(&path_str), path: path_str, name, size });
    }

    pub fn scan_files(&mut self) {
        self.games.clear();
        for dir in SEARCH_DIRS {
            let Ok(rd) = fs::read_dir(dir) else { continue };
            for entry in rd.flatten() {
                let path = entry.path();
                // A game may sit in its own folder with the files it loads
                // (db/, media/...): look one folder down too.
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    if let Ok(sub) = fs::read_dir(&path) {
                        for e in sub.flatten() {
                            self.add_game(&e.path(), e.metadata().map(|m| m.len()).unwrap_or(0));
                        }
                    }
                    continue;
                }
                self.add_game(&path, entry.metadata().map(|m| m.len()).unwrap_or(0));
            }
        }
        self.games.sort_by_key(|g| g.name.to_lowercase());
        for t in 0..TAB_COUNT {
            let n = self.visible_for(t).len();
            self.selected[t] = self.selected[t].min(n.saturating_sub(1));
        }
        println!("[Library] {} games", self.games.len());
    }

    fn visible_for(&self, tab: usize) -> Vec<usize> {
        match TABS[tab].0 {
            Tab::Settings | Tab::Badges => Vec::new(),
            Tab::Games => {
                let q = self.search.to_lowercase();
                (0..self.games.len()).filter(|&i| q.is_empty() || self.games[i].name.to_lowercase().contains(&q)).collect()
            }
            Tab::Recent => self
                .recent
                .iter()
                .filter_map(|(p, _)| self.games.iter().position(|g| &g.path == p))
                .collect(),
            Tab::Favorites => (0..self.games.len())
                .filter(|&i| self.favorites.contains(&self.games[i].path))
                .collect(),
        }
    }

    fn current(&self) -> Option<&Game> {
        let vis = self.visible_for(self.tab);
        vis.get(self.selected[self.tab]).map(|&i| &self.games[i])
    }

    pub fn selected_name(&self) -> String {
        self.current().map(|g| g.name.clone()).unwrap_or_else(|| "Demo".into())
    }

    pub fn selected_key(&self) -> String {
        self.current().map(|g| g.key.clone()).unwrap_or_else(|| covers::game_key(""))
    }

    fn tab_enabled(&self, i: usize) -> bool {
        TABS[i].0 != Tab::Badges || self.settings.badges
    }

    /// Counts badges' "right now" numbers.
    fn live(&self) -> Live {
        Live {
            games_played: self.playtime.values().filter(|s| **s > 0).count() as u64,
            library_size: self.games.len() as u64,
            favorites: self.favorites.len() as u64,
        }
    }

    fn check_badges(&mut self) {
        let live = self.live();
        self.badges.check(&live);
    }

    /// The typed search, while there is one.
    pub fn searching(&self) -> bool {
        !self.search.is_empty()
    }

    /// The on-screen keyboard may open for a search here (a games tab, no
    /// page over it).
    pub fn can_search(&self) -> bool {
        self.editor.is_none()
            && self.about_page.is_none()
            && self.saver.is_none()
            && matches!(TABS[self.tab].0, Tab::Recent | Tab::Games | Tab::Favorites)
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
    }

    /// Input that went elsewhere (the on-screen keyboard) still counts as
    /// someone being there.
    pub fn touch_input(&mut self) {
        self.last_input = Instant::now();
    }

    /// After a game: its play time and what happened, for badges.
    pub fn record_session(&mut self, key: &str, s: &Session) {
        *self.playtime.entry(key.to_string()).or_insert(0) += s.secs;
        let body: String = self.playtime.iter().map(|(k, t)| format!("{}\t{}\n", t, k)).collect();
        if let Err(e) = fs::write(PLAYTIME_FILE, body) {
            println!("[Library] can't write {}: {}", PLAYTIME_FILE, e);
        }
        let st = &mut self.badges.stats;
        st.play_secs += s.secs;
        st.longest_secs = st.longest_secs.max(s.secs);
        st.covers += s.covers as u64;
        st.quick_menus += s.quick_menus as u64;
        st.restarts += s.restarts as u64;
        st.keyboard += s.keyboard as u64;
        st.mouse += s.mouse as u64;
        self.badges.save();
        self.check_badges();
        self.last_input = Instant::now();
    }

    pub fn mark_played(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        self.badges.stats.launches += 1;
        let (mut h, mut m) = (12, 0);
        if unsafe { ruffle_ps5_clock(&mut h, &mut m) } == 0 {
            if h < 5 {
                self.badges.stats.night += 1;
            } else if h < 8 {
                self.badges.stats.morning += 1;
            }
        }
        self.badges.save();
        self.check_badges();
        self.recent.retain(|(p, _)| p != path);
        self.recent.insert(0, (path.to_string(), now_secs()));
        self.recent.truncate(30);
        let body: String = self.recent.iter().map(|(p, t)| format!("{}\t{}\n", t, p)).collect();
        if let Err(e) = fs::write(RECENT_FILE, body) {
            println!("[Library] can't write {}: {}", RECENT_FILE, e);
        }
    }

    fn toggle_favorite(&mut self) {
        let Some(path) = self.current().map(|g| g.path.clone()) else { return };
        if let Some(i) = self.favorites.iter().position(|p| *p == path) {
            self.favorites.remove(i);
            self.toast = Some(("Removed from Favorites".into(), Instant::now()));
        } else {
            self.favorites.push(path);
            self.toast = Some(("Added to Favorites".into(), Instant::now()));
        }
        let body: String = self.favorites.iter().map(|p| format!("{}\n", p)).collect();
        if let Err(e) = fs::write(FAVORITES_FILE, body) {
            println!("[Library] can't write {}: {}", FAVORITES_FILE, e);
        }
        let n = self.visible_for(self.tab).len();
        self.selected[self.tab] = self.selected[self.tab].min(n.saturating_sub(1));
    }

    pub fn show_toast(&mut self, msg: &str) {
        self.toast = Some((msg.to_string(), Instant::now()));
    }

    /// The pointer is there while a mouse is plugged in (or just used), and
    /// fades away when it's unplugged.
    fn pointer_alpha(&mut self) -> f32 {
        let recent = self.pointer_moved.is_some_and(|t| t.elapsed().as_secs_f32() < POINTER_IDLE);
        let target = if self.mouse_connected || recent { 1.0 } else { 0.0 };
        self.pointer_shown += (target - self.pointer_shown) * 0.25;
        if (self.pointer_shown - target).abs() < 0.02 {
            self.pointer_shown = target;
        }
        self.pointer_shown
    }

    /// The USB mouse: moving it points (and selects what it's over),
    /// clicking acts on it. Returns the controller buttons a click stands for.
    fn pointer_input(&mut self, m: &MenuMouse, now: Instant) -> u32 {
        let moved = m.dx != 0.0 || m.dy != 0.0;
        if moved {
            self.pointer.0 = (self.pointer.0 + m.dx as f32).clamp(0.0, W as f32 - 1.0);
            self.pointer.1 = (self.pointer.1 + m.dy as f32).clamp(0.0, H as f32 - 1.0);
        }
        if moved || m.click {
            self.pointer_moved = Some(now);
        }
        if !moved && !m.click {
            return 0;
        }
        // The About page: a click anywhere goes back.
        if self.about_page.is_some() {
            return if m.click { PAD_CIRCLE } else { 0 };
        }
        let (px, py) = (self.pointer.0 as i32, self.pointer.1 as i32);
        let Some(&(_, _, _, _, hit)) =
            self.hits.iter().rev().find(|(x, y, w, h, _)| px >= *x && py >= *y && px < x + w && py < y + h)
        else {
            return 0;
        };
        // Pointing selects games, settings and controls rows.
        match hit {
            Hit::Game(vi) if self.editor.is_none() => {
                if self.selected[self.tab] != vi {
                    self.selected[self.tab] = vi;
                    self.selection_changed = now;
                }
            }
            Hit::Setting(i) | Hit::SettingLeft(i) if self.editor.is_none() => self.setting_sel = i,
            Hit::Editor(i) | Hit::EditorLeft(i) => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.sel = i;
                }
            }
            Hit::Badge(i) => self.badge_sel = i,
            _ => {}
        }
        if !m.click {
            return 0;
        }
        match hit {
            Hit::Tab(i) if self.editor.is_none() => {
                if self.tab != i {
                    self.tab = i;
                    self.selection_changed = now;
                }
                0
            }
            Hit::Game(_) | Hit::Setting(_) | Hit::Editor(_) => PAD_CROSS,
            Hit::SettingLeft(_) | Hit::EditorLeft(_) => PAD_LEFT,
            Hit::Button(b) => b,
            _ => 0,
        }
    }

    /// Returns the game to start ("" for the built-in demo). Around the
    /// menus themselves: the screensaver, typing to search, and the sounds.
    pub fn handle_input(&mut self, f: &PadFrame, mouse: &MenuMouse) -> Option<String> {
        let now = Instant::now();
        let stick = |v: u8| (v as i32 - 128).abs() > 40;
        let active = f.pressed != 0
            || f.held != 0
            || stick(f.lx)
            || stick(f.ly)
            || stick(f.rx)
            || stick(f.ry)
            || mouse.dx != 0.0
            || mouse.dy != 0.0
            || mouse.click
            || !mouse.typed.is_empty()
            || mouse.backspace;
        if active {
            self.last_input = now;
        }
        if self.saver.is_some() {
            if active {
                self.saver = None;
                sounds::play(Sfx::Wake);
            }
            return None;
        }
        if self.settings.screensaver
            && self.editor.is_none()
            && self.about_page.is_none()
            && now.duration_since(self.last_input) >= SAVER_AFTER
        {
            self.saver = Some(now);
            self.badges.stats.screensaver += 1;
            self.badges.save();
            self.check_badges();
            sounds::play(Sfx::Sleep);
            return None;
        }
        if self.search_input(f, mouse, now) {
            return None;
        }

        // What the menus looked like before, to pick a sound after.
        let before = (
            self.tab,
            self.selected[self.tab],
            self.setting_sel,
            self.open_category,
            self.editor.as_ref().map(|e| e.sel),
            self.about_page.is_some() && self.about_closing.is_none(),
            self.badge_sel,
            self.favorites.len(),
            self.settings.clone(),
        );
        let editor_value = self.editor.as_ref().map(|e| e.controls.rows().map(|(_, _, b)| b.label()).collect::<Vec<_>>().join(",") + &e.controls.stick.to_string());
        let result = self.handle_input_inner(f, mouse, now);

        if result.is_some() {
            sounds::play(Sfx::Launch);
            return result;
        }
        let editor_now = self.editor.as_ref().map(|e| e.controls.rows().map(|(_, _, b)| b.label()).collect::<Vec<_>>().join(",") + &e.controls.stick.to_string());
        let about_open = self.about_page.is_some() && self.about_closing.is_none();
        let sfx = if self.tab != before.0 {
            Some(Sfx::Tab)
        } else if about_open != before.5 || self.open_category != before.3 || self.editor.is_some() != before.4.is_some() {
            Some(if about_open && !before.5 || self.open_category.is_some() && before.3 != self.open_category || self.editor.is_some() && before.4.is_none() {
                Sfx::Select
            } else {
                Sfx::Back
            })
        } else if self.settings != before.8 || self.favorites.len() != before.7 || editor_value != editor_now {
            Some(Sfx::Toggle)
        } else if self.selected[self.tab] != before.1
            || self.setting_sel != before.2
            || self.editor.as_ref().map(|e| e.sel) != before.4
            || self.badge_sel != before.6
        {
            Some(Sfx::Move)
        } else {
            None
        };
        if let Some(sfx) = sfx {
            sounds::play(sfx);
        }

        // Settings changes count for badges, and some apply at once.
        if self.settings != before.8 {
            let st = &mut self.badges.stats;
            st.settings += 1;
            st.themes_mask |= 1 << self.settings.theme;
            st.accents_mask |= 1 << self.settings.accent;
            gfx::set_accent(settings::ACCENTS[self.settings.accent as usize].1);
            sounds::set_enabled(self.settings.ui_sounds);
            self.badges.save();
        }
        if self.settings != before.8 || self.favorites.len() != before.7 {
            self.check_badges();
        }
        None
    }

    /// Typing on a USB keyboard searches the Games tab: letters add, Backspace
    /// removes, Esc (or Circle) clears. True when it took this frame.
    fn search_input(&mut self, f: &PadFrame, m: &MenuMouse, now: Instant) -> bool {
        if self.editor.is_some() || self.about_page.is_some() {
            return false;
        }
        let mut changed = false;
        for &c in &m.typed {
            if self.search.is_empty() && !c.is_alphanumeric() {
                continue;
            }
            if self.search.is_empty() {
                self.badges.stats.searches += 1;
                self.badges.save();
            }
            if self.search.chars().count() < 32 {
                self.search.push(c);
                changed = true;
            }
        }
        if m.backspace && self.search.pop().is_some() {
            changed = true;
        }
        if !self.search.is_empty() && f.just_pressed(PAD_CIRCLE) {
            self.search.clear();
            changed = true;
        }
        if !changed {
            return false;
        }
        self.tab = GAMES_TAB;
        self.selected[GAMES_TAB] = 0;
        self.selection_changed = now;
        sounds::play(if self.search.is_empty() { Sfx::Back } else { Sfx::Move });
        self.check_badges();
        true
    }

    fn handle_input_inner(&mut self, f: &PadFrame, mouse: &MenuMouse, now: Instant) -> Option<String> {
        let clicked = self.pointer_input(mouse, now);
        let merged = PadFrame { pressed: f.pressed | clicked, ..*f };
        let f = &merged;
        let dir = self.dpad(f, now);

        // The About page: any of O, X or Options closes it (with a fade).
        if self.about_page.is_some() {
            if self.about_closing.is_none()
                && (f.just_pressed(PAD_CIRCLE) || f.just_pressed(PAD_CROSS) || f.just_pressed(PAD_OPTIONS))
            {
                self.about_closing = Some(now);
            }
            return None;
        }

        // The controls page takes the controller while it's open.
        if self.editor.is_some() {
            self.editor_input(f, dir);
            return None;
        }

        if f.just_pressed(PAD_L1) || f.just_pressed(PAD_R1) {
            // Skipping a tab that's turned off (Badges).
            loop {
                self.tab = if f.just_pressed(PAD_R1) {
                    (self.tab + 1) % TAB_COUNT
                } else {
                    (self.tab + TAB_COUNT - 1) % TAB_COUNT
                };
                if self.tab_enabled(self.tab) {
                    break;
                }
            }
            self.selection_changed = now;
        }

        if TABS[self.tab].0 == Tab::Settings {
            self.settings_input(f, dir);
            return None;
        }
        if TABS[self.tab].0 == Tab::Badges {
            let (sel, n) = (self.badge_sel, BADGES.len());
            if dir & PAD_RIGHT != 0 && (sel + 1) % BCOLS != 0 && sel + 1 < n {
                self.badge_sel += 1;
            }
            if dir & PAD_LEFT != 0 && sel % BCOLS != 0 {
                self.badge_sel -= 1;
            }
            if dir & PAD_DOWN != 0 {
                self.badge_sel = (sel + BCOLS).min(n - 1);
            }
            if dir & PAD_UP != 0 && sel >= BCOLS {
                self.badge_sel -= BCOLS;
            }
            return None;
        }

        // Square: grid of covers <-> list of names.
        if f.just_pressed(PAD_SQUARE) {
            self.settings.list_view = !self.settings.list_view;
            self.settings.save();
        }
        // Options: the selected game's controls.
        if f.just_pressed(PAD_OPTIONS) {
            if let Some(g) = self.current() {
                self.editor = Some(ControlsEditor {
                    key: g.key.clone(),
                    name: g.name.clone(),
                    controls: Controls::load(&g.key),
                    sel: 0,
                });
                return None;
            }
        }

        let n = self.visible_for(self.tab).len();
        if n > 0 && dir != 0 && self.settings.list_view {
            // The list: up and down, left and right a page.
            let sel = self.selected[self.tab] as i32;
            let step = match dir {
                d if d & PAD_DOWN != 0 => 1,
                d if d & PAD_UP != 0 => -1,
                d if d & PAD_RIGHT != 0 => 8,
                _ => -8,
            };
            let next = (sel + step).clamp(0, n as i32 - 1) as usize;
            if next != self.selected[self.tab] {
                self.selected[self.tab] = next;
                self.selection_changed = now;
            }
        } else if n > 0 && dir != 0 {
            let sel = self.selected[self.tab];
            let mut next = sel as i32;
            if dir & PAD_RIGHT != 0 && (sel + 1) % COLS != 0 {
                next += 1;
            }
            if dir & PAD_LEFT != 0 && sel % COLS != 0 {
                next -= 1;
            }
            if dir & PAD_DOWN != 0 {
                // One row down, or to the last game when the row below is shorter.
                if sel + COLS < n {
                    next += COLS as i32;
                } else if sel / COLS < (n - 1) / COLS {
                    next = n as i32 - 1;
                }
            }
            if dir & PAD_UP != 0 && sel >= COLS {
                next -= COLS as i32;
            }
            let next = next.clamp(0, n as i32 - 1) as usize;
            if next != sel {
                self.selected[self.tab] = next;
                self.selection_changed = now;
            }
        }

        if f.just_pressed(PAD_TRIANGLE) && n > 0 {
            self.toggle_favorite();
        }
        if f.just_pressed(PAD_CROSS) {
            if let Some(g) = self.current() {
                return Some(g.path.clone());
            }
            if self.games.is_empty() {
                return Some(String::new());
            }
        }
        None
    }

    /// The D-Pad direction this frame, with hold-to-repeat.
    fn dpad(&mut self, f: &PadFrame, now: Instant) -> u32 {
        let dirs = PAD_LEFT | PAD_RIGHT | PAD_UP | PAD_DOWN;
        if f.pressed & dirs != 0 {
            self.held_dir = f.pressed & dirs;
            self.next_repeat = Some(now + Duration::from_millis(340));
            return self.held_dir;
        }
        if f.held & self.held_dir != 0 {
            if self.next_repeat.is_some_and(|t| now >= t) {
                self.next_repeat = Some(now + Duration::from_millis(80));
                return self.held_dir;
            }
            return 0;
        }
        self.next_repeat = None;
        0
    }

    /// The controls page: Up/Down pick a button, Left/Right change what it
    /// sends, Square clears it, Triangle resets all, Circle or Options closes
    /// (and saves).
    fn editor_input(&mut self, f: &PadFrame, dir: u32) {
        let Some(ed) = self.editor.as_mut() else { return };
        let rows = 1 + controls::BUTTONS.len();
        if dir & PAD_DOWN != 0 {
            ed.sel = (ed.sel + 1).min(rows - 1);
        }
        if dir & PAD_UP != 0 {
            ed.sel = ed.sel.saturating_sub(1);
        }
        let step = if dir & PAD_RIGHT != 0 || f.just_pressed(PAD_CROSS) {
            1
        } else if dir & PAD_LEFT != 0 {
            -1
        } else {
            0
        };
        if step != 0 {
            if ed.sel == 0 {
                let n = controls::STICK_MODES.len() as i32;
                ed.controls.stick = (ed.controls.stick as i32 + step).rem_euclid(n) as u8;
            } else {
                let b = &mut ed.controls.binds[ed.sel - 1];
                *b = b.step(step);
            }
        }
        if f.just_pressed(PAD_SQUARE) && ed.sel > 0 {
            ed.controls.binds[ed.sel - 1] = Bind::Nothing;
        }
        if f.just_pressed(PAD_TRIANGLE) {
            ed.controls = Controls::base_for(&ed.key);
            self.toast = Some(("Controls reset".into(), Instant::now()));
        }
        if f.just_pressed(PAD_CIRCLE) || f.just_pressed(PAD_OPTIONS) {
            let ed = self.editor.take().unwrap();
            ed.controls.save(&ed.key);
            self.badges.stats.controls += 1;
            self.badges.save();
            self.check_badges();
            self.toast = Some((format!("Controls saved for {}", ed.name), Instant::now()));
        }
    }

    /// Up/Down pick a setting, Left/Right change it, Cross runs an action
    /// (or steps a value, wrapping round).
    /// The settings rows on show: every category, the open one's settings,
    /// and About.
    fn visible_settings(&self) -> Vec<usize> {
        (0..settings::ROWS.len())
            .filter(|&i| match settings::category_of(i) {
                None => true,
                Some(cat) => self.open_category == Some(cat),
            })
            .collect()
    }

    fn settings_input(&mut self, f: &PadFrame, dir: u32) {
        let vis = self.visible_settings();
        let pos = vis.iter().position(|&i| i == self.setting_sel).unwrap_or(0);
        if dir & PAD_DOWN != 0 && pos + 1 < vis.len() {
            self.setting_sel = vis[pos + 1];
        }
        if dir & PAD_UP != 0 && pos > 0 {
            self.setting_sel = vis[pos - 1];
        }
        let row = settings::ROWS[self.setting_sel];

        // Cross on a category opens it (closing the one that was open), or
        // closes it; Circle inside a category closes it.
        if row.is_header() {
            if f.just_pressed(PAD_CROSS) {
                self.open_category =
                    if self.open_category == Some(self.setting_sel) { None } else { Some(self.setting_sel) };
            }
            return;
        }
        if f.just_pressed(PAD_CIRCLE) {
            if let Some(cat) = settings::category_of(self.setting_sel) {
                self.open_category = None;
                self.setting_sel = cat;
                return;
            }
        }
        if dir & PAD_RIGHT != 0 {
            self.settings.step(row, 1);
        }
        if dir & PAD_LEFT != 0 {
            self.settings.step(row, -1);
        }
        if f.just_pressed(PAD_CROSS) {
            match row {
                Row::Rescan => {
                    // New cover files may have been copied in too.
                    self.tiles.clear();
                    self.panel_covers.clear();
                    self.big_tile = None;
                    self.scan_files();
                    self.toast = Some((format!("Found {} games", self.games.len()), Instant::now()));
                }
                Row::DefaultControls => {
                    self.editor = Some(ControlsEditor {
                        key: controls::DEFAULT_KEY.into(),
                        name: "All games".into(),
                        controls: Controls::load(controls::DEFAULT_KEY),
                        sel: 0,
                    });
                }
                Row::ClearRecent => {
                    self.recent.clear();
                    let _ = fs::write(RECENT_FILE, "");
                    self.toast = Some(("Recently played cleared".into(), Instant::now()));
                }
                Row::About => self.about_page = Some(Instant::now()),
                Row::Header(_) => {}
                _ => {
                    // Cross steps forward and wraps from the last value to the first.
                    if !self.settings.step(row, 1) {
                        while self.settings.step(row, -1) {}
                    }
                }
            }
        }
    }

    /// Drops one game's cached pictures (its cover may have just been taken).
    pub fn forget_cover(&mut self, key: &str) {
        self.tiles.remove(key);
        self.panel_covers.remove(key);
        if self.big_tile.as_ref().is_some_and(|(k, _)| k == key) {
            self.big_tile = None;
        }
    }

    /// A game's tile, made at most a few per frame so a big library opens
    /// smoothly; None while it waits its turn.
    fn tile(&mut self, text: &mut Text, idx: usize, now: bool) -> Option<&Image> {
        let key = self.games[idx].key.clone();
        if !self.tiles.contains_key(&key) {
            if self.tiles_made_this_frame >= 3 && !now {
                return None;
            }
            self.tiles_made_this_frame += 1;
            let cover = covers::load_cover(&key);
            let tile = covers::make_tile(&key, &self.games[idx].name, cover.as_ref(), text, TILE_W, TILE_H);
            self.tiles.insert(key.clone(), tile);
        }
        self.tiles.get(&key)
    }

    fn panel_cover(&mut self, text: &mut Text, key: &str, name: &str) -> &Image {
        if !self.panel_covers.contains_key(key) {
            if self.panel_covers.len() > 8 {
                self.panel_covers.clear();
            }
            let cover = covers::load_cover(key);
            let img = covers::make_tile(key, name, cover.as_ref(), text, COVER_W, COVER_H);
            self.panel_covers.insert(key.to_string(), img);
        }
        &self.panel_covers[key]
    }

    pub fn render(&mut self, text: &mut Text, bg: &Background) -> &[u8] {
        let dt = self.last_render.elapsed().as_secs_f32().min(0.1);
        self.last_render = Instant::now();
        self.tiles_made_this_frame = 0;
        let k = 1.0 - (-dt * 14.0).exp();

        let mut cv = std::mem::replace(&mut self.canvas, Canvas::new(0, 0));
        bg.draw(&mut cv, self.settings.waves);
        self.hits.clear();
        self.badges.enabled = self.settings.badges;
        if !self.tab_enabled(self.tab) {
            self.tab = TAB_COUNT - 1;
        }

        if let Some(since) = self.saver {
            self.draw_screensaver(&mut cv, text, since);
            self.badges.draw_popup(&mut cv, text);
            bg.overlay(&mut cv);
            self.canvas = cv;
            return &self.canvas.px;
        }

        if let Some(opened) = self.about_page {
            // Fades in over the settings, and back out.
            let fade = match self.about_closing {
                Some(at) => 1.0 - ease(at.elapsed().as_secs_f32() / 0.3),
                None => ease(opened.elapsed().as_secs_f32() / 0.3),
            };
            if self.about_closing.is_some() && fade <= 0.0 {
                self.about_page = None;
                self.about_closing = None;
            } else {
                self.draw_about_page(&mut cv, text, opened, fade);
                self.badges.draw_popup(&mut cv, text);
                bg.overlay(&mut cv);
                self.draw_pointer(&mut cv);
                self.canvas = cv;
                return &self.canvas.px;
            }
        }

        self.draw_tabs(&mut cv, text, k);
        self.draw_header(&mut cv, text);
        let target = if self.settings.list_view { 1.0 } else { 0.0 };
        self.view_t += (target - self.view_t) * k;
        if (self.view_t - target).abs() < 0.01 {
            self.view_t = target;
        }
        if self.editor.is_some() {
            self.draw_editor(&mut cv, text, k);
        } else if TABS[self.tab].0 == Tab::Settings {
            self.draw_settings(&mut cv, text);
        } else if TABS[self.tab].0 == Tab::Badges {
            self.draw_badges(&mut cv, text, k);
        } else {
            self.draw_search(&mut cv, text);
            let vis = self.visible_for(self.tab);
            if vis.is_empty() {
                self.draw_empty(&mut cv, text);
            } else if self.view_t < 0.5 {
                // Grid and list cross over through a quick fade.
                self.draw_grid(&mut cv, text, &vis, k, 1.0 - 2.0 * self.view_t);
            } else {
                self.draw_list(&mut cv, text, &vis, k, 2.0 * self.view_t - 1.0);
            }
            self.draw_panel(&mut cv, text);
        }
        self.draw_toast(&mut cv, text);
        self.badges.draw_popup(&mut cv, text);
        bg.overlay(&mut cv);
        self.draw_pointer(&mut cv);

        self.canvas = cv;
        &self.canvas.px
    }

    /// The search box beside the tabs: what's typed, or how to start.
    fn draw_search(&mut self, cv: &mut Canvas, text: &mut Text) {
        if self.search.is_empty() && !self.keyboard_hints && !self.search_open {
            return;
        }
        let (x, y, h) = (self.tabs_end + 24, 40, 64);
        let w = PANEL_X - 64 - x;
        if w < 200 {
            return;
        }
        let active = !self.search.is_empty() || self.search_open;
        cv.fill_rect(x, y, w, h, gfx::INK, if active { 0.6 } else { 0.35 });
        if active {
            cv.fill_rect(x, y + h - 3, w, 3, accent(), 1.0);
        }
        // A magnifying glass.
        let (cx, cy) = ((x + 34) as f32, (y + 29) as f32);
        cv.stroke_circle(cx, cy, 11.0, 3.0, WHITE, if active { 0.9 } else { 0.5 });
        cv.line(cx + 8.0, cy + 8.0, cx + 16.0, cy + 16.0, 3.5, WHITE, if active { 0.9 } else { 0.5 });
        let tx = x + 64;
        if active {
            let q = text.fit(Weight::SemiBold, 28, &self.search, w - 190);
            let tw = text.draw(cv, Weight::SemiBold, 28, tx, y + 15, &q, WHITE, 1.0);
            if self.start.elapsed().as_millis() % 1000 < 550 {
                cv.fill_rect(tx + tw + 3, y + 16, 3, 32, accent(), 1.0);
            }
            let n = self.visible_for(GAMES_TAB).len();
            let label = format!("{} found", n);
            let lw = text.width(Weight::Regular, 20, &label);
            text.draw(cv, Weight::Regular, 20, x + w - 20 - lw, y + 21, &label, WHITE, 0.6);
        } else {
            let hint = if self.keyboard_hints { "Type to search" } else { "L2 to search" };
            text.draw(cv, Weight::Regular, 24, tx, y + 18, hint, WHITE, 0.45);
        }
    }

    /// The Badges tab: a grid of medals, the chosen one in the panel.
    fn draw_badges(&mut self, cv: &mut Canvas, text: &mut Text, k: f32) {
        let live = self.live();
        let sel = self.badge_sel;
        let view = GRID_BOTTOM - GRID_Y;
        let row_top = (sel / BCOLS) as i32 * BROW;
        let mut target = self.badge_scroll;
        if (row_top as f32) < target {
            target = row_top as f32;
        }
        if (row_top + BROW) as f32 > target + view as f32 {
            target = (row_top + BROW - view) as f32;
        }
        self.badge_scroll += (target - self.badge_scroll) * k;

        for i in 0..BADGES.len() {
            let (col, row) = ((i % BCOLS) as i32, (i / BCOLS) as i32);
            let x = GRID_X + col * (BW + BGAP);
            let y = GRID_Y + row * BROW - self.badge_scroll as i32;
            let edge = ((y - (GRID_Y - 60)) as f32 / 60.0).min((H - y - 40) as f32 / 120.0).clamp(0.0, 1.0);
            if edge <= 0.0 || y + BH < GRID_Y - 40 || y > H {
                continue;
            }
            let unlocked = self.badges.unlocked_at(i).is_some();
            let selected = i == sel;
            if edge > 0.3 {
                self.hits.push((x, y, BW, BH, Hit::Badge(i)));
            }
            cv.fill_round_rect(x, y, BW, BH, 18, gfx::INK, 0.5 * edge);
            if selected {
                cv.fill_round_rect(x, y, BW, BH, 18, WHITE, 0.1 * edge);
                cv.stroke_round_rect(x - 6, y - 6, BW + 12, BH + 12, 22, 3, WHITE, edge);
            }
            let a = if unlocked || selected { 1.0 } else { 0.55 } * edge;
            gfx::medal(cv, (x + BW / 2) as f32, (y + 68) as f32, 40.0, unlocked, a, i);
            let name = text.fit(Weight::SemiBold, 22, BADGES[i].name, BW - 24);
            let nw = text.width(Weight::SemiBold, 22, &name);
            text.draw(cv, Weight::SemiBold, 22, x + (BW - nw) / 2, y + 122, &name, WHITE, a);
            if unlocked {
                let lw = text.width(Weight::SemiBold, 16, "UNLOCKED");
                text.draw(cv, Weight::SemiBold, 16, x + (BW - lw) / 2, y + 162, "UNLOCKED", gfx::GOLD, a);
            } else {
                let (now, goal) = self.badges.progress(i, &live);
                let bw = BW - 70;
                let bx = x + 35;
                cv.fill_round_rect(bx, y + 166, bw, 6, 3, WHITE, 0.15 * edge);
                let fill = (bw as u64 * now / goal.max(1)) as i32;
                if fill > 0 {
                    cv.fill_round_rect(bx, y + 166, fill, 6, 3, accent(), a);
                }
            }
        }

        // The panel: the chosen badge, big.
        cv.fill_round_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, 22, gfx::INK, 0.55);
        cv.stroke_round_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, 22, 1, WHITE, 0.1);
        let (ix, iw) = (PANEL_X + 28, PANEL_W - 56);
        let mut y = PANEL_Y + 34;
        text.draw(cv, Weight::SemiBold, 20, ix, y, "BADGES", accent(), 1.0);
        y += 30;
        let count = format!("{} of {}", self.badges.count(), BADGES.len());
        text.draw(cv, Weight::Bold, 40, ix, y, &count, WHITE, 1.0);
        y += 54;
        let total = self.badges.count() as i32 * iw / BADGES.len() as i32;
        cv.fill_round_rect(ix, y, iw, 8, 4, WHITE, 0.15);
        if total > 0 {
            cv.fill_round_rect(ix, y, total, 8, 4, gfx::GOLD, 1.0);
        }
        y += 60;
        let unlocked = self.badges.unlocked_at(sel);
        let t = self.start.elapsed().as_secs_f32();
        if unlocked.is_some() {
            gfx::rays(cv, (PANEL_X + PANEL_W / 2) as f32, (y + 90) as f32, 150.0, 12, t * 0.2, gfx::GOLD, 0.18);
        }
        gfx::medal(cv, (PANEL_X + PANEL_W / 2) as f32, (y + 90) as f32, 84.0, unlocked.is_some(), 1.0, sel);
        y += 210;
        for line in text.wrap(Weight::Bold, 34, BADGES[sel].name, iw, 2) {
            text.draw(cv, Weight::Bold, 34, ix, y, &line, WHITE, 1.0);
            y += 44;
        }
        y += 6;
        for line in text.wrap(Weight::Regular, 24, BADGES[sel].about, iw, 3) {
            text.draw(cv, Weight::Regular, 24, ix, y, &line, WHITE, 0.78);
            y += 34;
        }
        y += 16;
        match unlocked {
            Some(at) => {
                let label = format!("Unlocked {}", ago(at));
                text.draw(cv, Weight::SemiBold, 24, ix, y, &label, gfx::GOLD, 1.0);
            }
            None => {
                let (now, goal) = self.badges.progress(sel, &live);
                cv.fill_round_rect(ix, y + 8, iw - 110, 10, 5, WHITE, 0.15);
                let fill = ((iw - 110) as u64 * now / goal.max(1)) as i32;
                if fill > 0 {
                    cv.fill_round_rect(ix, y + 8, fill, 10, 5, accent(), 1.0);
                }
                let label = format!("{} / {}", now, goal);
                let lw = text.width(Weight::SemiBold, 22, &label);
                text.draw(cv, Weight::SemiBold, 22, ix + iw - lw, y, &label, WHITE, 0.85);
            }
        }
        self.draw_panel_hints(cv, text, ix, iw, &[]);
    }

    /// The screensaver: the games' covers drifting slowly across the waves
    /// at different depths, and a big clock.
    fn draw_screensaver(&mut self, cv: &mut Canvas, text: &mut Text, since: Instant) {
        let t = since.elapsed().as_secs_f32();
        let a = ease(t / 1.5);
        cv.fade(1.0 - 0.45 * a);
        let n = self.games.len().min(14);
        let mut items: Vec<(f32, usize, u64)> = (0..n)
            .map(|i| {
                let h = (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                (0.3 + 0.7 * ((h >> 20) % 100) as f32 / 100.0, i, h)
            })
            .collect();
        items.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap_or(std::cmp::Ordering::Equal));
        let span = (W + TILE_W + 300) as f32;
        for (depth, gi, h) in items {
            let speed = 10.0 + 34.0 * depth;
            let x = ((h % 10_000) as f32 / 10_000.0 * span + t * speed) % span - TILE_W as f32 - 150.0;
            let y = 60.0 + ((h >> 36) % 800) as f32 + (t * 0.21 + gi as f32).sin() * 26.0;
            let alpha = a * (0.18 + 0.55 * depth);
            if let Some(tile) = self.tile(text, gi, true) {
                cv.draw_image_rounded(tile, x as i32, y.min((H - TILE_H) as f32) as i32, 14, alpha);
            }
        }
        // The clock, breathing gently.
        let (mut hh, mut mm) = (0, 0);
        if unsafe { ruffle_ps5_clock(&mut hh, &mut mm) } == 0 {
            let clock = format!("{:02}:{:02}", hh, mm);
            let cw = text.width(Weight::Bold, 150, &clock);
            let breathe = 0.8 + 0.2 * (t * 0.8).sin();
            text.draw(cv, Weight::Bold, 150, (W - cw) / 2, 380, &clock, WHITE, a * breathe);
        }
        let label = "Ruffle Flash";
        let lw = text.width(Weight::SemiBold, 30, label);
        text.draw(cv, Weight::SemiBold, 30, (W - lw) / 2, 580, label, accent(), 0.8 * a);
    }

    fn draw_pointer(&mut self, cv: &mut Canvas) {
        let a = self.pointer_alpha();
        if a > 0.0 {
            gfx::pointer(cv, self.pointer.0, self.pointer.1, a);
        }
    }

    /// Flat tabs joined in a strip (PPSSPP style): the active one filled
    /// orange, an orange rule under the strip running along the grid.
    fn draw_tabs(&mut self, cv: &mut Canvas, text: &mut Text, k: f32) {
        let (y, h) = (40, 64);
        let pad = 32;
        let labels: Vec<i32> = TABS.iter().map(|(_, l)| text.width(Weight::SemiBold, 28, l)).collect();

        let mut x = GRID_X;
        let mut target = (0.0, 0.0);
        let mut xs = Vec::new();
        for (i, w) in labels.iter().enumerate() {
            if !self.tab_enabled(i) {
                xs.push(x);
                continue;
            }
            let tw = w + 2 * pad;
            cv.fill_rect(x, y, tw, h, gfx::INK, 0.45);
            if i > 0 {
                cv.fill_rect(x, y + 14, 1, h - 28, WHITE, 0.12);
            }
            if i == self.tab {
                target = (x as f32, tw as f32);
            }
            self.hits.push((x, y, tw, h, Hit::Tab(i)));
            xs.push(x);
            x += tw;
        }
        self.tabs_end = x;
        if self.tab_pill.1 == 0.0 {
            self.tab_pill = target;
        }
        self.tab_pill.0 += (target.0 - self.tab_pill.0) * k;
        self.tab_pill.1 += (target.1 - self.tab_pill.1) * k;
        cv.fill_rect(self.tab_pill.0 as i32, y, self.tab_pill.1.round() as i32, h, accent(), 1.0);
        cv.fill_rect(GRID_X, y + h, PANEL_X - 64 - GRID_X, 3, accent(), 1.0);

        for (i, (_, label)) in TABS.iter().enumerate() {
            if !self.tab_enabled(i) {
                continue;
            }
            let active = i == self.tab;
            text.draw(cv, Weight::SemiBold, 28, xs[i] + pad, y + 15, label, WHITE, if active { 1.0 } else { 0.7 });
        }
    }

    fn draw_grid(&mut self, cv: &mut Canvas, text: &mut Text, vis: &[usize], k: f32, alpha: f32) {
        let sel = self.selected[self.tab];
        let sel_row = (sel / COLS) as i32;
        let view = GRID_BOTTOM - GRID_Y;
        // Keep the selected row in view, scrolling smoothly.
        let row_top = sel_row * ROW_H;
        let mut target = self.scroll;
        if (row_top as f32) < target {
            target = row_top as f32;
        }
        if (row_top + ROW_H) as f32 > target + view as f32 {
            target = (row_top + ROW_H - view) as f32;
        }
        self.scroll += (target - self.scroll) * k;

        let place = |vi: usize, scroll: f32| -> (i32, i32, f32) {
            let (col, row) = ((vi % COLS) as i32, (vi / COLS) as i32);
            let x = GRID_X + col * (TILE_W + GAP);
            let y = GRID_Y + row * ROW_H - scroll as i32;
            // Rows scrolling under the tab bar or off the bottom fade.
            let fade = ((y - (GRID_Y - 70)) as f32 / 70.0).min((H - y - 60) as f32 / 120.0).clamp(0.0, 1.0);
            (x, y, fade * alpha)
        };

        // The others, dimmed, under the selected one.
        for (vi, &gi) in vis.iter().enumerate() {
            let (x, y, fade) = place(vi, self.scroll);
            if vi == sel || fade <= 0.0 || y + TILE_H < GRID_Y - 30 || y > H {
                continue;
            }
            if fade > 0.3 && y + TILE_H > GRID_Y - 20 {
                self.hits.push((x, y, TILE_W, TILE_H, Hit::Game(vi)));
            }
            match self.tile(text, gi, false) {
                Some(tile) => cv.draw_image_rounded(tile, x, y, 14, 0.5 * fade),
                // Not made yet: a placeholder for a frame or two.
                None => cv.fill_round_rect(x, y, TILE_W, TILE_H, 14, WHITE, 0.06 * fade),
            }
            self.draw_favorite_dot(cv, gi, x + TILE_W, y, fade * 0.7);
        }

        // The selected one (PS5 home screen style): larger, lifted by a
        // shadow, a white frame with a gap, a glass shine, its name below.
        let Some(&gi) = vis.get(sel) else { return };
        let (x, y, fade) = place(sel, self.scroll);
        if fade <= 0.0 {
            return;
        }
        let key = self.games[gi].key.clone();
        if self.big_tile.as_ref().is_none_or(|(k, _)| *k != key) {
            let (bw, bh) = ((TILE_W as f32 * SEL_SCALE) as i32, (TILE_H as f32 * SEL_SCALE) as i32);
            let big = self.tile(text, gi, true).expect("tile").resized(bw, bh);
            self.big_tile = Some((key, big));
        }
        let big = &self.big_tile.as_ref().unwrap().1;
        let (bw, bh) = (big.w, big.h);
        let (bx, by) = (x - (bw - TILE_W) / 2, y - (bh - TILE_H) / 2);
        self.hits.push((bx, by, bw, bh, Hit::Game(sel)));
        cv.glow(bx, by + 8, bw, bh, 16, 34, gfx::INK, 0.7 * fade);
        cv.draw_image_rounded(big, bx, by, 16, fade);
        let t = self.selection_changed.elapsed().as_secs_f32() % 4.5;
        if t < 0.9 {
            gfx::shine(cv, bx, by, bw, bh, 16, t / 0.9, 0.38 * fade);
        }
        cv.stroke_round_rect(bx - 9, by - 9, bw + 18, bh + 18, 24, 4, WHITE, fade);
        self.draw_favorite_dot(cv, gi, bx + bw, by, fade);
        let name = text.fit(Weight::SemiBold, 24, &self.games[gi].name, bw + 40);
        text.draw(cv, Weight::SemiBold, 24, bx, by + bh + 20, &name, WHITE, fade);
    }

    /// The settings list on the left, the chosen setting explained on the right.
    fn draw_settings(&mut self, cv: &mut Canvas, text: &mut Text) {
        let list_w = PANEL_X - 64 - GRID_X;
        let row_h = 66;
        let top = GRID_Y;
        let bottom = GRID_BOTTOM;
        cv.fill_round_rect(GRID_X, top - 8, list_w, bottom - top + 8, 20, gfx::INK, 0.5);

        // Keep the chosen row in view, scrolling smoothly.
        let vis = self.visible_settings();
        let pos = vis.iter().position(|&i| i == self.setting_sel).unwrap_or(0);
        let view = (bottom - top - 24) as f32;
        let sel_y = (pos as i32 * row_h) as f32;
        let mut target = self.settings_scroll;
        if sel_y - (row_h as f32) < target {
            target = (sel_y - row_h as f32).max(0.0);
        }
        if sel_y + 2.0 * row_h as f32 > target + view {
            target = sel_y + 2.0 * row_h as f32 - view;
        }
        // A little past the last row, so it clears the list's faded edge.
        let max_scroll = (vis.len() as i32 * row_h + 40) as f32 - view;
        target = target.clamp(0.0, max_scroll.max(0.0));
        self.settings_scroll += (target - self.settings_scroll) * 0.25;

        for (p, &i) in vis.iter().enumerate() {
            let row = &settings::ROWS[i];
            let y = top + 12 + p as i32 * row_h - self.settings_scroll as i32;
            if y < top - row_h || y > bottom {
                continue;
            }
            // Rows fade at the list's top and bottom edges.
            let edge = ((y - top + 30) as f32 / 40.0).min((bottom - y - row_h + 10) as f32 / 40.0).clamp(0.0, 1.0);
            if edge <= 0.0 {
                continue;
            }
            let selected = i == self.setting_sel;
            let in_category = settings::category_of(i).is_some();
            if edge > 0.3 {
                self.hits.push((GRID_X + 10, y, list_w - 20, row_h - 4, Hit::Setting(i)));
            }
            if selected {
                cv.fill_round_rect(GRID_X + 10, y, list_w - 20, row_h - 4, 14, WHITE, 0.12 * edge);
                cv.fill_round_rect(GRID_X + 10, y + 12, 5, row_h - 28, 3, accent(), edge);
            } else if p > 0 {
                cv.fill_rect(GRID_X + 30, y - 2, list_w - 60, 1, WHITE, 0.07 * edge);
            }
            let right = GRID_X + list_w - 36;
            let cy = (y + row_h / 2 - 2) as f32;

            if let Row::Header(title) = row {
                // A category: its name, how many settings, and a chevron
                // pointing down when open.
                let open = self.open_category == Some(i);
                let a = if selected || open { 1.0 } else { 0.85 } * edge;
                text.draw(cv, Weight::Bold, 28, GRID_X + 36, y + 14, title, if open { accent() } else { WHITE }, a);
                let n = (i + 1..settings::ROWS.len()).take_while(|&j| settings::category_of(j) == Some(i)).count();
                let label = format!("{} settings", n);
                let lw = text.width(Weight::Regular, 20, &label);
                text.draw(cv, Weight::Regular, 20, right - lw - 34, y + 20, &label, WHITE, 0.5 * edge);
                let cx = right as f32 - 6.0;
                if open {
                    cv.line(cx - 9.0, cy - 4.0, cx, cy + 5.0, 3.0, accent(), a);
                    cv.line(cx, cy + 5.0, cx + 9.0, cy - 4.0, 3.0, accent(), a);
                } else {
                    cv.line(cx - 4.0, cy - 9.0, cx + 5.0, cy, 3.0, WHITE, a);
                    cv.line(cx + 5.0, cy, cx - 4.0, cy + 9.0, 3.0, WHITE, a);
                }
                continue;
            }

            if *row == Row::About {
                let a = if selected { 1.0 } else { 0.85 } * edge;
                text.draw(cv, Weight::Bold, 28, GRID_X + 36, y + 14, row.label(), WHITE, a);
                continue;
            }

            let a = if selected { 1.0 } else { 0.78 } * edge;
            // Settings inside a category sit a step in.
            let lx = GRID_X + if in_category { 64 } else { 36 };
            if in_category {
                cv.fill_rect(GRID_X + 40, y + 8, 2, row_h - 16, accent(), 0.35 * edge);
            }
            text.draw(cv, Weight::SemiBold, 26, lx, y + 16, row.label(), WHITE, a);
            if row.is_action() {
                // A chevron: runs with Cross.
                let cx = right as f32 - 6.0;
                cv.line(cx - 6.0, cy - 9.0, cx + 3.0, cy, 3.0, WHITE, a);
                cv.line(cx + 3.0, cy, cx - 6.0, cy + 9.0, 3.0, WHITE, a);
            } else if *row != Row::About {
                // < value >
                let value = self.settings.value(*row);
                let vw = text.width(Weight::SemiBold, 26, &value);
                let rx = right as f32 - 6.0;
                let lx = (right - vw - 44) as f32;
                let ca = if selected { 1.0 } else { 0.35 };
                if edge > 0.3 {
                    self.hits.push((lx as i32 - 26, y, 52, row_h - 4, Hit::SettingLeft(i)));
                }
                cv.line(rx - 6.0, cy - 8.0, rx + 2.0, cy, 3.0, accent(), ca);
                cv.line(rx + 2.0, cy, rx - 6.0, cy + 8.0, 3.0, accent(), ca);
                cv.line(lx + 6.0, cy - 8.0, lx - 2.0, cy, 3.0, accent(), ca);
                cv.line(lx - 2.0, cy, lx + 6.0, cy + 8.0, 3.0, accent(), ca);
                text.draw(cv, Weight::SemiBold, 26, right - vw - 22, y + 16, &value, WHITE, a);
            }
        }

        // The panel: what the setting does.
        cv.fill_round_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, 22, gfx::INK, 0.55);
        cv.stroke_round_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, 22, 1, WHITE, 0.1);
        let row = settings::ROWS[self.setting_sel];
        let (ix, iw) = (PANEL_X + 28, PANEL_W - 56);
        if row == Row::About {
            self.draw_about(cv, text, ix, iw);
            return;
        }
        if let Row::Header(title) = row {
            self.draw_category_panel(cv, text, ix, iw, title);
            return;
        }
        let mut y = PANEL_Y + 34;
        for line in text.wrap(Weight::Bold, 36, row.label(), iw, 2) {
            text.draw(cv, Weight::Bold, 36, ix, y, &line, WHITE, 1.0);
            y += 46;
        }
        let value = self.settings.value(row);
        if !value.is_empty() {
            y += 6;
            for line in text.wrap(Weight::Bold, 44, &value, iw, 2) {
                text.draw(cv, Weight::Bold, 44, ix, y, &line, accent(), 1.0);
                y += 56;
            }
            y += 6;
        }
        y += 14;
        for line in text.wrap(Weight::Regular, 24, &row.help(&self.settings), iw, 9) {
            text.draw(cv, Weight::Regular, 24, ix, y, &line, WHITE, 0.78);
            y += 34;
        }
        let mut hint: Vec<(Option<PadIcon>, &str, &str)> = if row.is_action() {
            vec![(Some(PadIcon::Cross), "", "Run")]
        } else {
            vec![(None, "Left Right", "Change"), (Some(PadIcon::Cross), "", "Next value")]
        };
        if settings::category_of(self.setting_sel).is_some() {
            hint.push((Some(PadIcon::Circle), "", "Close category"));
        }
        self.draw_panel_hints(cv, text, ix, iw, &hint);
    }

    /// A category's panel: what it's for, and its settings at a glance.
    fn draw_category_panel(&mut self, cv: &mut Canvas, text: &mut Text, ix: i32, iw: i32, title: &str) {
        let mut y = PANEL_Y + 34;
        text.draw(cv, Weight::SemiBold, 20, ix, y, "CATEGORY", accent(), 1.0);
        y += 32;
        text.draw(cv, Weight::Bold, 40, ix, y, title, WHITE, 1.0);
        y += 60;
        for line in text.wrap(Weight::Regular, 24, settings::header_help(title), iw, 4) {
            text.draw(cv, Weight::Regular, 24, ix, y, &line, WHITE, 0.78);
            y += 34;
        }
        y += 18;
        let header = self.setting_sel;
        for j in header + 1..settings::ROWS.len() {
            if settings::category_of(j) != Some(header) {
                break;
            }
            let row = settings::ROWS[j];
            cv.fill_rect(ix, y - 8, iw, 1, WHITE, 0.07);
            text.draw(cv, Weight::Regular, 21, ix, y, row.label(), WHITE, 0.7);
            let value = self.settings.value(row);
            if !value.is_empty() {
                let v = text.fit(Weight::SemiBold, 21, &value, iw / 2);
                let vw = text.width(Weight::SemiBold, 21, &v);
                text.draw(cv, Weight::SemiBold, 21, ix + iw - vw, y, &v, accent(), 1.0);
            }
            y += 38;
        }
        let open = self.open_category == Some(header);
        let hint = [(Some(PadIcon::Cross), "", if open { "Close" } else { "Open" })];
        self.draw_panel_hints(cv, text, ix, iw, &hint);
    }

    /// Button hints at the bottom of the settings panel, then "Switch tabs".
    fn draw_panel_hints(&mut self, cv: &mut Canvas, text: &mut Text, ix: i32, iw: i32, hint: &[(Option<PadIcon>, &str, &str)]) {
        let row_h = 60;
        let mut ry = PANEL_Y + PANEL_H - 16 - (hint.len() as i32 + 1) * row_h;
        for (icon, chip, label) in hint.iter().chain([(None, "L1 R1", "Switch tabs")].iter()) {
            cv.fill_rect(ix, ry, iw, 1, WHITE, 0.08);
            let cy = ry + row_h / 2;
            if let Some(b) = button_of(*icon, chip) {
                self.hits.push((ix - 12, ry, iw + 24, row_h, Hit::Button(b)));
            }
            let lead = draw_hint_lead(cv, text, *icon, chip, self.keyboard_hints, ix, cy);
            text.draw(cv, Weight::SemiBold, 26, ix + lead, cy - 16, label, WHITE, 0.85);
            ry += row_h;
        }
    }

    /// The About page: the logo in turning golden rays with a shine passing
    /// over it, the name and version, then the credits, coming in one after
    /// another.
    fn draw_about_page(&mut self, cv: &mut Canvas, text: &mut Text, opened: Instant, fade: f32) {
        const GOLD: [u8; 3] = [0xFF, 0xC8, 0x5A];
        const SIZE: i32 = 230;
        let t = opened.elapsed().as_secs_f32();
        // Each part fades in a little after the one above it.
        let part = |delay: f32| ease((t - delay) / 0.45) * fade;
        let (cx, cy) = (960.0f32, 290.0f32);

        // Golden light: a breathing glow and slowly turning rays.
        let breathe = 0.85 + 0.15 * (t * 1.6).sin();
        let a = part(0.0);
        for i in 0..14 {
            let r = 260.0 - i as f32 * 16.0;
            cv.fill_circle(cx, cy, r * breathe, GOLD, 0.022 * a);
        }
        gfx::rays(cv, cx, cy, 380.0, 14, t * 0.25, GOLD, 0.30 * a);
        gfx::rays(cv, cx, cy, 300.0, 9, -t * 0.4 + 0.3, [0xFF, 0xE6, 0xA8], 0.18 * a);

        // The logo, rising a little as it appears, with a gold shine sweeping
        // across it every few seconds.
        let logo = self.about_logo_big.get_or_insert_with(|| {
            let img = image::load_from_memory(LOGO_PNG).expect("logo").to_rgba8();
            let (w, h) = img.dimensions();
            Image::from_rgba(w, h, img.into_raw()).resized(SIZE, SIZE)
        });
        let la = part(0.1);
        let lx = cx as i32 - SIZE / 2;
        let ly = cy as i32 - SIZE / 2 + ((1.0 - la) * 24.0) as i32;
        let cycle = (t - 0.6).rem_euclid(3.5);
        let sweep = if t > 0.6 && cycle < 1.1 { Some(cycle / 1.1) } else { None };
        for iy in 0..SIZE {
            for ix in 0..SIZE {
                let s = ((iy * SIZE + ix) * 4) as usize;
                let alpha = logo.px[s + 3] as f32 / 255.0 * la;
                if alpha <= 0.0 {
                    continue;
                }
                let mut c = [logo.px[s], logo.px[s + 1], logo.px[s + 2]];
                if let Some(p) = sweep {
                    // A diagonal band of gold light.
                    let u = (ix + iy) as f32 / (2 * SIZE) as f32;
                    let band = 1.0 - ((u - (-0.2 + 1.4 * p)).abs() / 0.09);
                    if band > 0.0 {
                        let k = band * band;
                        for ch in 0..3 {
                            c[ch] = (c[ch] as f32 + (255.0 - c[ch] as f32) * k * 0.85).min(255.0) as u8;
                        }
                    }
                }
                cv.blend(lx + ix, ly + iy, c, (alpha * 255.0) as u32);
            }
        }

        // Name and version.
        let ta = part(0.35);
        let w1 = text.width(Weight::Bold, 72, "Ruffle ");
        let w2 = text.width(Weight::Bold, 72, "Flash");
        let tx = 960 - (w1 + w2) / 2;
        let ty = 455 + ((1.0 - ta) * 16.0) as i32;
        text.draw(cv, Weight::Bold, 72, tx, ty, "Ruffle ", WHITE, ta);
        text.draw(cv, Weight::Bold, 72, tx + w1, ty, "Flash", accent(), ta);
        let version = format!("Version {}  \u{00B7}  for PS5", env!("CARGO_PKG_VERSION"));
        let vw = text.width(Weight::Regular, 26, &version);
        text.draw(cv, Weight::Regular, 26, 960 - vw / 2, ty + 96, &version, WHITE, 0.6 * ta);

        let da = part(0.5);
        let about = Row::About.help(&self.settings);
        let lines = text.wrap(Weight::Regular, 26, &about, 900, 2);
        for (i, line) in lines.iter().enumerate() {
            let lw = text.width(Weight::Regular, 26, line);
            text.draw(cv, Weight::Regular, 26, 960 - lw / 2, 610 + i as i32 * 36, line, WHITE, 0.8 * da);
        }

        // Credits: one column each, centred under the logo.
        let col_w = 380;
        let total = col_w * settings::CREDITS.len() as i32;
        let x0 = 960 - total / 2;
        let cy0 = 720;
        cv.fill_rect(x0 + 40, cy0 - 26, total - 80, 1, WHITE, 0.12 * part(0.6));
        for (i, (role, who)) in settings::CREDITS.iter().enumerate() {
            let ca = part(0.65 + i as f32 * 0.1);
            let col_x = x0 + i as i32 * col_w;
            let rl = role.to_uppercase();
            let rw = text.width(Weight::SemiBold, 18, &rl);
            text.draw(cv, Weight::SemiBold, 18, col_x + (col_w - rw) / 2, cy0, &rl, accent(), ca);
            for (j, line) in text.wrap(Weight::SemiBold, 26, who, col_w - 40, 2).iter().enumerate() {
                let lw = text.width(Weight::SemiBold, 26, line);
                text.draw(cv, Weight::SemiBold, 26, col_x + (col_w - lw) / 2, cy0 + 32 + j as i32 * 34, line, WHITE, ca);
            }
        }

        let fa = part(1.1);
        let folders = "Games  /data/ruffle/games      Covers  /data/ruffle/covers      Saves  /data/ruffle/saves";
        let fw = text.width(Weight::Regular, 20, folders);
        text.draw(cv, Weight::Regular, 20, 960 - fw / 2, 905, folders, WHITE, 0.5 * fa);

        // Back.
        let ba = part(1.2);
        let label = if self.keyboard_hints { "Esc  Back" } else { "Back" };
        let lw = text.width(Weight::SemiBold, 24, label);
        if self.keyboard_hints {
            text.draw(cv, Weight::SemiBold, 24, 960 - lw / 2, 989, label, WHITE, 0.85 * ba);
        } else {
            let bx = 960 - (34 + 12 + lw) / 2;
            gfx::pad_icon(cv, PadIcon::Circle, (bx + 16) as f32, 1003.0, 16.0 * ba.max(0.01));
            text.draw(cv, Weight::SemiBold, 24, bx + 46, 989, label, WHITE, 0.85 * ba);
        }
    }

    /// About: the logo, the app's name and version, credits and folders.
    fn draw_about(&mut self, cv: &mut Canvas, text: &mut Text, ix: i32, iw: i32) {
        let logo = self.about_logo.get_or_insert_with(|| {
            let img = image::load_from_memory(LOGO_PNG).expect("logo").to_rgba8();
            let (w, h) = img.dimensions();
            Image::from_rgba(w, h, img.into_raw()).resized(150, 150)
        });
        let lx = PANEL_X + (PANEL_W - logo.w) / 2;
        let ly = PANEL_Y + 36;
        // A soft orange glow behind it.
        for i in 0..12 {
            let r = 100.0 - i as f32 * 7.0;
            cv.fill_circle((lx + logo.w / 2) as f32, (ly + logo.h / 2) as f32, r, accent(), 0.018);
        }
        cv.draw_image(logo, lx, ly);

        let mut y = ly + logo.h + 22;
        let title_w = text.width(Weight::Bold, 36, "Ruffle ") + text.width(Weight::Bold, 36, "Flash");
        let tx = PANEL_X + (PANEL_W - title_w) / 2;
        let w = text.draw(cv, Weight::Bold, 36, tx, y, "Ruffle ", WHITE, 1.0);
        text.draw(cv, Weight::Bold, 36, tx + w, y, "Flash", accent(), 1.0);
        y += 50;
        let version = format!("Version {} for PS5", env!("CARGO_PKG_VERSION"));
        let vw = text.width(Weight::Regular, 22, &version);
        text.draw(cv, Weight::Regular, 22, PANEL_X + (PANEL_W - vw) / 2, y, &version, WHITE, 0.6);
        y += 44;

        for line in text.wrap(Weight::Regular, 22, &Row::About.help(&self.settings), iw, 3) {
            text.draw(cv, Weight::Regular, 22, ix, y, &line, WHITE, 0.78);
            y += 31;
        }
        y += 18;
        for (role, who) in settings::CREDITS {
            cv.fill_rect(ix, y - 10, iw, 1, WHITE, 0.08);
            text.draw(cv, Weight::SemiBold, 18, ix, y, &role.to_uppercase(), accent(), 1.0);
            y += 26;
            for line in text.wrap(Weight::SemiBold, 24, who, iw, 2) {
                text.draw(cv, Weight::SemiBold, 24, ix, y, &line, WHITE, 0.95);
                y += 32;
            }
            y += 16;
        }
        y += 4;
        for line in ["Games  /data/ruffle/games", "Covers  /data/ruffle/covers", "Saves  /data/ruffle/saves"] {
            text.draw(cv, Weight::Regular, 20, ix, y, line, WHITE, 0.55);
            y += 28;
        }
    }

    fn draw_favorite_dot(&self, cv: &mut Canvas, gi: usize, right: i32, top: i32, alpha: f32) {
        if self.favorites.contains(&self.games[gi].path) {
            cv.fill_circle((right - 20) as f32, (top + 20) as f32, 10.0, accent(), alpha);
            cv.fill_circle((right - 20) as f32, (top + 20) as f32, 3.5, WHITE, alpha);
        }
    }

    fn draw_empty(&mut self, cv: &mut Canvas, text: &mut Text) {
        let no_match = format!("Nothing is called \"{}\". Backspace to change it, Esc to clear.", self.search);
        let (title, line) = match TABS[self.tab].0 {
            Tab::Games if !self.search.is_empty() => ("No matches", no_match.as_str()),
            Tab::Favorites => ("No favorites yet", "Press the triangle button on a game to keep it here."),
            Tab::Recent => ("Nothing played yet", "The games you play show up here."),
            Tab::Games | Tab::Settings | Tab::Badges => ("No games yet", "Copy .swf files to /data/ruffle/games or a USB drive's ruffle folder."),
        };
        text.draw(cv, Weight::Bold, 60, GRID_X, 360, title, WHITE, 1.0);
        let lines = text.wrap(Weight::Regular, 26, line, PANEL_X - GRID_X - 80, 2);
        for (i, l) in lines.iter().enumerate() {
            text.draw(cv, Weight::Regular, 26, GRID_X, 448 + i as i32 * 38, l, WHITE, 0.75);
        }
    }

    /// Ruffle Flash (PS5) and the clock, above the panel.
    fn draw_header(&mut self, cv: &mut Canvas, text: &mut Text) {
        let y = 50;
        let mut x = PANEL_X + 4;
        x += text.draw(cv, Weight::Bold, 40, x, y, "Ruffle", WHITE, 1.0) + 10;
        // "Flash" glows for a moment every few seconds.
        let t = self.start.elapsed().as_secs_f32() % 6.0;
        let glow = if t < 0.8 { (t / 0.8 * std::f32::consts::PI).sin() } else { 0.0 };
        if glow > 0.0 {
            for (dx, dy) in [(-2, 0), (2, 0), (0, -2), (0, 2)] {
                text.draw(cv, Weight::Bold, 40, x + dx, y + dy, "Flash", accent_light(), 0.25 * glow);
            }
        }
        x += text.draw(cv, Weight::Bold, 40, x, y, "Flash", accent(), 1.0) + 12;
        cv.stroke_round_rect(x, y + 12, 46, 26, 7, 2, WHITE, 0.5);
        text.draw(cv, Weight::SemiBold, 15, x + 9, y + 16, "PS5", WHITE, 0.8);

        let (mut h, mut m) = (0, 0);
        if unsafe { ruffle_ps5_clock(&mut h, &mut m) } == 0 {
            let clock = format!("{:02}:{:02}", h, m);
            let cw = text.width(Weight::SemiBold, 28, &clock);
            text.draw(cv, Weight::SemiBold, 28, W - MARGIN - cw, y + 8, &clock, WHITE, 0.9);
        }
    }

    /// The frosted panel: cover, title, details and the actions as menu rows.
    fn draw_panel(&mut self, cv: &mut Canvas, text: &mut Text) {
        cv.fill_round_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, 22, gfx::INK, 0.55);
        cv.stroke_round_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, 22, 1, WHITE, 0.1);

        let inner_x = PANEL_X + 24;
        let inner_w = PANEL_W - 48;
        let (key, name, path, size) = match self.current() {
            Some(g) => (g.key.clone(), g.name.clone(), g.path.clone(), Some(g.size)),
            None => ("~empty".to_string(), "Ruffle Flash".to_string(), String::new(), None),
        };

        let cover = self.panel_cover(text, &key, &name);
        cv.draw_image_rounded(cover, inner_x, PANEL_Y + 24, 14, 1.0);

        let mut y = PANEL_Y + 24 + COVER_H + 26;
        for line in text.wrap(Weight::Bold, 34, &name, inner_w, 2) {
            text.draw(cv, Weight::Bold, 34, inner_x, y, &line, WHITE, 1.0);
            y += 44;
        }
        y += 8;

        if size.is_some() && self.favorites.contains(&path) {
            let w = text.width(Weight::SemiBold, 16, "FAVORITE") + 22;
            cv.fill_round_rect(inner_x, y, w, 28, 14, accent(), 1.0);
            text.draw(cv, Weight::SemiBold, 16, inner_x + 11, y + 5, "FAVORITE", WHITE, 1.0);
            y += 42;
        }
        let details: Vec<String> = match size {
            Some(bytes) => vec![
                "Shockwave Flash".into(),
                format_size(bytes),
                match self.recent.iter().find(|(p, _)| *p == path) {
                    Some((_, t)) => format!("Played {}", ago(*t)),
                    None => "Not played yet".into(),
                },
            ]
            .into_iter()
            .chain(self.playtime.get(&key).filter(|s| **s > 0).map(|s| format!("{} played", format_play(*s))))
            .collect(),
            None => vec!["Flash games on PS5".into(), "Games go in /data/ruffle/games".into()],
        };
        for d in details {
            text.draw(cv, Weight::Regular, 22, inner_x, y, &d, WHITE, 0.7);
            y += 32;
        }

        // Menu rows at the bottom of the panel.
        let fav = size.is_some() && self.favorites.contains(&path);
        let mut rows: Vec<(Option<PadIcon>, &str, String)> = Vec::new();
        if size.is_some() || self.games.is_empty() {
            rows.push((Some(PadIcon::Cross), "", if size.is_some() { "Play".into() } else { "Play the demo".into() }));
        }
        if size.is_some() {
            rows.push((Some(PadIcon::Triangle), "", if fav { "Remove from Favorites".into() } else { "Add to Favorites".into() }));
        }
        rows.push((
            Some(PadIcon::Square),
            "",
            if self.settings.list_view { "Grid view".into() } else { "List view".into() },
        ));
        if size.is_some() {
            rows.push((None, "Options", "Controls".into()));
        }
        if !self.keyboard_hints {
            rows.push((None, "L2", "Search".into()));
        }
        rows.push((None, "L1 R1", "Switch tabs".into()));

        let row_h = 58;
        let mut ry = PANEL_Y + PANEL_H - 16 - rows.len() as i32 * row_h;
        for (i, (icon, chip, label)) in rows.iter().enumerate() {
            if i == 0 {
                cv.fill_round_rect(PANEL_X + 12, ry + 4, PANEL_W - 24, row_h - 8, 14, WHITE, 0.08);
            } else {
                cv.fill_rect(inner_x, ry, inner_w, 1, WHITE, 0.08);
            }
            let cy = ry + row_h / 2;
            if let Some(b) = button_of(*icon, chip) {
                self.hits.push((PANEL_X + 12, ry, PANEL_W - 24, row_h, Hit::Button(b)));
            }
            let lead = draw_hint_lead(cv, text, *icon, chip, self.keyboard_hints, inner_x, cy);
            text.draw(cv, Weight::SemiBold, 26, inner_x + lead, cy - 16, label, WHITE, if i == 0 { 1.0 } else { 0.85 });
            ry += row_h;
        }
    }

    /// The list view: one row per game, name on the left and details on the
    /// right, the selected row lit like the settings list.
    fn draw_list(&mut self, cv: &mut Canvas, text: &mut Text, vis: &[usize], k: f32, alpha: f32) {
        let list_w = PANEL_X - 64 - GRID_X;
        let row_h = 74;
        let (top, bottom) = (GRID_Y, GRID_BOTTOM);
        let sel = self.selected[self.tab];

        let view = (bottom - top - 24) as f32;
        let sel_y = (sel as i32 * row_h) as f32;
        let mut target = self.list_scroll;
        if sel_y - row_h as f32 * 0.5 < target {
            target = (sel_y - row_h as f32 * 0.5).max(0.0);
        }
        if sel_y + 1.5 * row_h as f32 > target + view {
            target = sel_y + 1.5 * row_h as f32 - view;
        }
        let max_scroll = (vis.len() as i32 * row_h + 20) as f32 - view;
        target = target.clamp(0.0, max_scroll.max(0.0));
        self.list_scroll += (target - self.list_scroll) * k;

        cv.fill_round_rect(GRID_X, top - 8, list_w, bottom - top + 8, 20, gfx::INK, 0.5 * alpha);
        for (vi, &gi) in vis.iter().enumerate() {
            let y = top + 12 + vi as i32 * row_h - self.list_scroll as i32;
            if y < top - row_h || y > bottom {
                continue;
            }
            let edge = ((y - top + 30) as f32 / 40.0).min((bottom - y - row_h + 10) as f32 / 40.0).clamp(0.0, 1.0);
            let a = edge * alpha;
            if a <= 0.0 {
                continue;
            }
            let selected = vi == sel;
            if a > 0.3 {
                self.hits.push((GRID_X + 10, y, list_w - 20, row_h - 4, Hit::Game(vi)));
            }
            if selected {
                cv.fill_round_rect(GRID_X + 10, y, list_w - 20, row_h - 4, 14, WHITE, 0.12 * a);
                cv.fill_round_rect(GRID_X + 10, y + 14, 5, row_h - 32, 3, accent(), a);
            } else if vi > 0 {
                cv.fill_rect(GRID_X + 30, y - 2, list_w - 60, 1, WHITE, 0.07 * a);
            }
            let g = &self.games[gi];
            let mut details = match self.recent.iter().find(|(p, _)| *p == g.path) {
                Some((_, t)) => format!("{}   \u{00B7}   {}", format_size(g.size), ago(*t)),
                None => format_size(g.size),
            };
            if let Some(secs) = self.playtime.get(&g.key).filter(|s| **s > 0) {
                details = format!("{}   \u{00B7}   {}", format_play(*secs), details);
            }
            let dw = text.width(Weight::Regular, 22, &details);
            let right = GRID_X + list_w - 36;
            text.draw(cv, Weight::Regular, 22, right - dw, y + 22, &details, WHITE, 0.6 * a);
            let mut nx = GRID_X + 36;
            if self.favorites.contains(&g.path) {
                cv.fill_circle((nx + 6) as f32, (y + row_h / 2 - 2) as f32, 6.0, accent(), a);
                nx += 26;
            }
            let name = text.fit(Weight::SemiBold, 28, &g.name, right - dw - 40 - nx);
            text.draw(cv, Weight::SemiBold, 28, nx, y + 18, &name, WHITE, if selected { a } else { 0.8 * a });
        }
    }

    /// A game's controls: the left stick and each button, what it sends.
    fn draw_editor(&mut self, cv: &mut Canvas, text: &mut Text, k: f32) {
        let Some(ed) = self.editor.as_ref() else { return };
        let list_w = PANEL_X - 64 - GRID_X;
        let row_h = 62;
        let (top, bottom) = (GRID_Y, GRID_BOTTOM);
        let rows: Vec<(String, String)> = std::iter::once((
            "Left stick".to_string(),
            controls::STICK_MODES[ed.controls.stick as usize].to_string(),
        ))
        .chain(ed.controls.rows().map(|(_, name, bind)| (name.to_string(), bind.label())))
        .collect();
        let sel = ed.sel;
        let game_name = ed.name.clone();

        let view = (bottom - top - 24) as f32;
        let sel_y = (sel as i32 * row_h) as f32;
        let mut target = self.editor_scroll;
        if sel_y - (row_h as f32) < target {
            target = (sel_y - row_h as f32).max(0.0);
        }
        if sel_y + 2.0 * row_h as f32 > target + view {
            target = sel_y + 2.0 * row_h as f32 - view;
        }
        let max_scroll = (rows.len() as i32 * row_h + 20) as f32 - view;
        target = target.clamp(0.0, max_scroll.max(0.0));
        self.editor_scroll += (target - self.editor_scroll) * k;

        cv.fill_round_rect(GRID_X, top - 8, list_w, bottom - top + 8, 20, gfx::INK, 0.5);
        for (i, (name, value)) in rows.iter().enumerate() {
            let y = top + 12 + i as i32 * row_h - self.editor_scroll as i32;
            if y < top - row_h || y > bottom {
                continue;
            }
            let edge = ((y - top + 30) as f32 / 40.0).min((bottom - y - row_h + 10) as f32 / 40.0).clamp(0.0, 1.0);
            if edge <= 0.0 {
                continue;
            }
            let selected = i == sel;
            if edge > 0.3 {
                self.hits.push((GRID_X + 10, y, list_w - 20, row_h - 4, Hit::Editor(i)));
            }
            if selected {
                cv.fill_round_rect(GRID_X + 10, y, list_w - 20, row_h - 4, 14, WHITE, 0.12 * edge);
                cv.fill_round_rect(GRID_X + 10, y + 12, 5, row_h - 28, 3, accent(), edge);
            } else if i > 0 {
                cv.fill_rect(GRID_X + 30, y - 2, list_w - 60, 1, WHITE, 0.07 * edge);
            }
            let a = if selected { 1.0 } else { 0.78 } * edge;
            text.draw(cv, Weight::SemiBold, 26, GRID_X + 36, y + 14, name, WHITE, a);
            let right = GRID_X + list_w - 36;
            let vw = text.width(Weight::SemiBold, 26, value);
            let cy = (y + row_h / 2 - 2) as f32;
            let (rx, lx) = (right as f32 - 6.0, (right - vw - 44) as f32);
            let ca = if selected { 1.0 } else { 0.35 } * edge;
            if edge > 0.3 {
                self.hits.push((lx as i32 - 26, y, 52, row_h - 4, Hit::EditorLeft(i)));
            }
            cv.line(rx - 6.0, cy - 8.0, rx + 2.0, cy, 3.0, accent(), ca);
            cv.line(rx + 2.0, cy, rx - 6.0, cy + 8.0, 3.0, accent(), ca);
            cv.line(lx + 6.0, cy - 8.0, lx - 2.0, cy, 3.0, accent(), ca);
            cv.line(lx - 2.0, cy, lx + 6.0, cy + 8.0, 3.0, accent(), ca);
            let color = if value == "Nothing" { [0x99, 0x99, 0x99] } else { WHITE };
            text.draw(cv, Weight::SemiBold, 26, right - vw - 22, y + 14, value, color, a);
        }

        // The panel: the game, the chosen row, and the buttons.
        cv.fill_round_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, 22, gfx::INK, 0.55);
        cv.stroke_round_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, 22, 1, WHITE, 0.1);
        let (ix, iw) = (PANEL_X + 28, PANEL_W - 56);
        let mut y = PANEL_Y + 34;
        text.draw(cv, Weight::SemiBold, 20, ix, y, "CONTROLS", accent(), 1.0);
        y += 34;
        for line in text.wrap(Weight::Bold, 34, &game_name, iw, 2) {
            text.draw(cv, Weight::Bold, 34, ix, y, &line, WHITE, 1.0);
            y += 44;
        }
        y += 24;
        let (name, value) = &rows[sel];
        text.draw(cv, Weight::SemiBold, 24, ix, y, name, WHITE, 0.7);
        y += 36;
        for line in text.wrap(Weight::Bold, 40, value, iw, 2) {
            text.draw(cv, Weight::Bold, 40, ix, y, &line, accent(), 1.0);
            y += 50;
        }
        y += 14;
        let help = if sel == 0 {
            "Mouse moves the cursor. Arrow keys or WASD make the stick play as keys, and the right stick moves the cursor instead."
        } else {
            "Each game keeps its own controls. Most games use arrows, Space, Z, X or letters; their help screen tells which."
        };
        for line in text.wrap(Weight::Regular, 22, help, iw, 5) {
            text.draw(cv, Weight::Regular, 22, ix, y, &line, WHITE, 0.7);
            y += 31;
        }

        let hints: [(Option<PadIcon>, &str, &str); 4] = [
            (None, "Left Right", "Change"),
            (Some(PadIcon::Square), "", "Set to nothing"),
            (Some(PadIcon::Triangle), "", "Reset all"),
            (Some(PadIcon::Circle), "", "Save and close"),
        ];
        let row_h = 58;
        let mut ry = PANEL_Y + PANEL_H - 16 - hints.len() as i32 * row_h;
        for (icon, chip, label) in hints {
            cv.fill_rect(ix, ry, iw, 1, WHITE, 0.08);
            let cy = ry + row_h / 2;
            if let Some(b) = button_of(icon, chip) {
                self.hits.push((ix - 12, ry, iw + 24, row_h, Hit::Button(b)));
            }
            let lead = draw_hint_lead(cv, text, icon, chip, self.keyboard_hints, ix, cy);
            text.draw(cv, Weight::SemiBold, 26, ix + lead, cy - 16, label, WHITE, 0.85);
            ry += row_h;
        }
    }

    fn draw_toast(&mut self, cv: &mut Canvas, text: &mut Text) {
        let Some((msg, at)) = self.toast.clone() else { return };
        let t = at.elapsed().as_secs_f32();
        if t > 2.2 {
            self.toast = None;
            return;
        }
        let a = ease((t / 0.2).min((2.2 - t) / 0.3));
        let w = text.width(Weight::SemiBold, 24, &msg) + 56;
        let (x, y) = (GRID_X, H - 40 - 56 + ((1.0 - a) * 20.0) as i32);
        cv.fill_round_rect(x, y, w, 56, 28, gfx::INK, 0.85 * a);
        cv.fill_circle((x + 26) as f32, (y + 28) as f32, 6.0, accent(), a);
        text.draw(cv, Weight::SemiBold, 24, x + 42, y + 14, &msg, WHITE, a);
    }
}
