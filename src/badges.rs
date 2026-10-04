//! Badges: little goals for playing (first game, an hour of Flash, night
//! owl...), unlocked with a popup and a chime. Stats in
//! /data/ruffle/stats.txt, unlocked badges in /data/ruffle/badges.txt.

use std::collections::VecDeque;
use std::fs;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::sounds::{self, Sfx};
use crate::ui::gfx::{self, Canvas, WHITE};
use crate::ui::text::{Text, Weight};

const STATS_FILE: &str = "/data/ruffle/stats.txt";
const BADGES_FILE: &str = "/data/ruffle/badges.txt";
const POPUP_TIME: f32 = 4.2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Launches,
    PlayMinutes,
    LongestMinutes,
    GamesPlayed,
    LibrarySize,
    Favorites,
    Covers,
    QuickMenus,
    Restarts,
    Keyboard,
    Mouse,
    Searches,
    Themes,
    Accents,
    Settings,
    Screensaver,
    Controls,
    Night,
    Morning,
    Badges,
}

pub struct Badge {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub metric: Metric,
    pub target: u64,
}

const fn b(id: &'static str, name: &'static str, about: &'static str, metric: Metric, target: u64) -> Badge {
    Badge { id, name, about, metric, target }
}

use Metric as M;

pub const BADGES: [Badge; 34] = [
    b("first-flight", "First Flight", "Play your first game.", M::Launches, 1),
    b("regular", "Regular", "Start 10 games.", M::Launches, 10),
    b("arcade-rat", "Arcade Rat", "Start 50 games.", M::Launches, 50),
    b("flash-legend", "Flash Legend", "Start 200 games.", M::Launches, 200),
    b("warm-up", "Warm Up", "Play for 10 minutes in total.", M::PlayMinutes, 10),
    b("hour", "Hour of Flash", "Play for an hour in total.", M::PlayMinutes, 60),
    b("marathon", "Marathon", "Play for 10 hours in total.", M::PlayMinutes, 600),
    b("living-2007", "Living in 2007", "Play for 50 hours in total.", M::PlayMinutes, 3000),
    b("sit-tight", "Sit Tight", "Play one game for 30 minutes straight.", M::LongestMinutes, 30),
    b("deep-dive", "Deep Dive", "Play one game for 2 hours straight.", M::LongestMinutes, 120),
    b("explorer", "Explorer", "Play 5 different games.", M::GamesPlayed, 5),
    b("collector", "Collector", "Play 15 different games.", M::GamesPlayed, 15),
    b("archivist", "Archivist", "Play 40 different games.", M::GamesPlayed, 40),
    b("full-shelf", "Full Shelf", "Have 25 games in your library.", M::LibrarySize, 25),
    b("hoarder", "Hoarder", "Have 100 games in your library.", M::LibrarySize, 100),
    b("first-love", "First Love", "Add a game to your favorites.", M::Favorites, 1),
    b("curator", "Curator", "Have 10 favorite games.", M::Favorites, 10),
    b("photographer", "Photographer", "Take a game's cover yourself.", M::Covers, 1),
    b("gallery", "Gallery", "Take 10 covers yourself.", M::Covers, 10),
    b("breather", "Take a Breather", "Open the quick menu in a game.", M::QuickMenus, 1),
    b("pause-master", "Pause Master", "Open the quick menu 25 times.", M::QuickMenus, 25),
    b("again", "Again!", "Restart a game from the quick menu.", M::Restarts, 1),
    b("never-give-up", "Never Give Up", "Restart games 10 times.", M::Restarts, 10),
    b("typist", "Typist", "Play with a USB keyboard.", M::Keyboard, 1),
    b("point-click", "Point and Click", "Play with a USB mouse.", M::Mouse, 1),
    b("seeker", "Seeker", "Search your library.", M::Searches, 1),
    b("night-owl", "Night Owl", "Start a game between midnight and 5 am.", M::Night, 1),
    b("early-bird", "Early Bird", "Start a game between 5 and 8 in the morning.", M::Morning, 1),
    b("fashionista", "Fashionista", "Try every theme.", M::Themes, 4),
    b("painter", "Painter", "Try 5 accent colours.", M::Accents, 5),
    b("tinkerer", "Tinkerer", "Change 10 settings.", M::Settings, 10),
    b("daydreamer", "Daydreamer", "Let the screensaver start.", M::Screensaver, 1),
    b("remapper", "Remapper", "Save controls for a game.", M::Controls, 1),
    b("completionist", "Completionist", "Unlock 30 other badges.", M::Badges, 30),
];

/// Counters kept across runs.
#[derive(Default)]
pub struct Stats {
    pub launches: u64,
    pub play_secs: u64,
    pub longest_secs: u64,
    pub covers: u64,
    pub quick_menus: u64,
    pub restarts: u64,
    pub keyboard: u64,
    pub mouse: u64,
    pub searches: u64,
    pub themes_mask: u64,
    pub accents_mask: u64,
    pub settings: u64,
    pub screensaver: u64,
    pub controls: u64,
    pub night: u64,
    pub morning: u64,
}

/// Counts the library knows right now.
#[derive(Clone, Copy, Default)]
pub struct Live {
    pub games_played: u64,
    pub library_size: u64,
    pub favorites: u64,
}

/// What happened in one game session.
#[derive(Clone, Copy, Default)]
pub struct Session {
    pub secs: u64,
    pub covers: u32,
    pub quick_menus: u32,
    pub restarts: u32,
    pub keyboard: bool,
    pub mouse: bool,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub struct Badges {
    pub stats: Stats,
    /// Unlock time per badge (0 = locked).
    unlocked: [u64; BADGES.len()],
    queue: VecDeque<usize>,
    popup: Option<(usize, Instant)>,
    /// Settings > App > Badges: off drops the popups.
    pub enabled: bool,
}

impl Badges {
    pub fn load() -> Badges {
        let mut s = Stats::default();
        if let Ok(text) = fs::read_to_string(STATS_FILE) {
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                let v: u64 = v.trim().parse().unwrap_or(0);
                match k.trim() {
                    "launches" => s.launches = v,
                    "play_secs" => s.play_secs = v,
                    "longest_secs" => s.longest_secs = v,
                    "covers" => s.covers = v,
                    "quick_menus" => s.quick_menus = v,
                    "restarts" => s.restarts = v,
                    "keyboard" => s.keyboard = v,
                    "mouse" => s.mouse = v,
                    "searches" => s.searches = v,
                    "themes_mask" => s.themes_mask = v,
                    "accents_mask" => s.accents_mask = v,
                    "settings" => s.settings = v,
                    "screensaver" => s.screensaver = v,
                    "controls" => s.controls = v,
                    "night" => s.night = v,
                    "morning" => s.morning = v,
                    _ => {}
                }
            }
        }
        let mut unlocked = [0u64; BADGES.len()];
        if let Ok(text) = fs::read_to_string(BADGES_FILE) {
            for line in text.lines() {
                let Some((id, t)) = line.split_once('\t') else { continue };
                if let Some(i) = BADGES.iter().position(|b| b.id == id) {
                    unlocked[i] = t.trim().parse().unwrap_or(1).max(1);
                }
            }
        }
        Badges { stats: s, unlocked, queue: VecDeque::new(), popup: None, enabled: true }
    }

    pub fn save(&self) {
        let s = &self.stats;
        let fields = [
            ("launches", s.launches),
            ("play_secs", s.play_secs),
            ("longest_secs", s.longest_secs),
            ("covers", s.covers),
            ("quick_menus", s.quick_menus),
            ("restarts", s.restarts),
            ("keyboard", s.keyboard),
            ("mouse", s.mouse),
            ("searches", s.searches),
            ("themes_mask", s.themes_mask),
            ("accents_mask", s.accents_mask),
            ("settings", s.settings),
            ("screensaver", s.screensaver),
            ("controls", s.controls),
            ("night", s.night),
            ("morning", s.morning),
        ];
        let body: String = fields.iter().map(|(k, v)| format!("{}={}\n", k, v)).collect();
        let _ = fs::write(STATS_FILE, body);
        let body: String = BADGES
            .iter()
            .zip(self.unlocked.iter())
            .filter(|(_, t)| **t > 0)
            .map(|(b, t)| format!("{}\t{}\n", b.id, t))
            .collect();
        let _ = fs::write(BADGES_FILE, body);
    }

    pub fn count(&self) -> usize {
        self.unlocked.iter().filter(|t| **t > 0).count()
    }

    pub fn unlocked_at(&self, i: usize) -> Option<u64> {
        Some(self.unlocked[i]).filter(|t| *t > 0)
    }

    fn value(&self, metric: Metric, live: &Live) -> u64 {
        let s = &self.stats;
        match metric {
            M::Launches => s.launches,
            M::PlayMinutes => s.play_secs / 60,
            M::LongestMinutes => s.longest_secs / 60,
            M::GamesPlayed => live.games_played,
            M::LibrarySize => live.library_size,
            M::Favorites => live.favorites,
            M::Covers => s.covers,
            M::QuickMenus => s.quick_menus,
            M::Restarts => s.restarts,
            M::Keyboard => s.keyboard,
            M::Mouse => s.mouse,
            M::Searches => s.searches,
            M::Themes => s.themes_mask.count_ones() as u64,
            M::Accents => s.accents_mask.count_ones() as u64,
            M::Settings => s.settings,
            M::Screensaver => s.screensaver,
            M::Controls => s.controls,
            M::Night => s.night,
            M::Morning => s.morning,
            M::Badges => self.count() as u64 - self.unlocked_at(BADGES.len() - 1).map_or(0, |_| 1),
        }
    }

    /// (now, goal) for badge `i`.
    pub fn progress(&self, i: usize, live: &Live) -> (u64, u64) {
        let b = &BADGES[i];
        (self.value(b.metric, live).min(b.target), b.target)
    }

    /// Unlocks every badge whose goal is met (queueing their popups) and
    /// saves.
    pub fn check(&mut self, live: &Live) {
        let mut changed = false;
        loop {
            let mut more = false;
            for i in 0..BADGES.len() {
                if self.unlocked[i] == 0 && self.value(BADGES[i].metric, live) >= BADGES[i].target {
                    self.unlocked[i] = now_secs().max(1);
                    self.queue.push_back(i);
                    println!("[Badges] unlocked: {}", BADGES[i].name);
                    more = true;
                    changed = true;
                }
            }
            // Completionist counts the others.
            if !more {
                break;
            }
        }
        if changed {
            self.save();
        }
    }

    /// The unlock popup at the top of the screen, one badge at a time.
    pub fn draw_popup(&mut self, cv: &mut Canvas, text: &mut Text) {
        if !self.enabled {
            self.queue.clear();
            self.popup = None;
            return;
        }
        if self.popup.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs_f32() > POPUP_TIME) {
            self.popup = None;
        }
        if self.popup.is_none() {
            if let Some(i) = self.queue.pop_front() {
                self.popup = Some((i, Instant::now()));
                sounds::play(Sfx::Badge);
            }
        }
        let Some((i, at)) = self.popup else { return };
        let t = at.elapsed().as_secs_f32();
        let a = ease((t / 0.35).min((POPUP_TIME - t) / 0.4));
        let badge = &BADGES[i];
        let (w, h) = (620, 116);
        let x = (cv.w - w) / 2;
        let y = -h + ((40 + h) as f32 * a) as i32;
        cv.glow(x, y + 8, w, h, 24, 30, gfx::INK, 0.6 * a);
        cv.fill_round_rect(x, y, w, h, 24, gfx::INK, 0.9 * a);
        cv.stroke_round_rect(x, y, w, h, 24, 2, gfx::GOLD, 0.55 * a);
        // A gold shine passes once the card has landed.
        if (0.4..1.4).contains(&t) {
            gfx::shine(cv, x, y, w, h, 24, (t - 0.4) / 1.0, 0.25 * a);
        }
        gfx::medal(cv, (x + 66) as f32, (y + h / 2) as f32, 38.0, true, a, i);
        text.draw(cv, Weight::SemiBold, 18, x + 124, y + 20, "BADGE UNLOCKED", gfx::GOLD, a);
        let name = text.fit(Weight::Bold, 32, badge.name, w - 150);
        text.draw(cv, Weight::Bold, 32, x + 124, y + 42, &name, WHITE, a);
        let about = text.fit(Weight::Regular, 20, badge.about, w - 150);
        text.draw(cv, Weight::Regular, 20, x + 124, y + 82, &about, WHITE, 0.7 * a);
    }
}
