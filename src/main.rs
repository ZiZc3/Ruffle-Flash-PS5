// std's start-up (lang_start) is skipped: it reopens missing fds 0-2 on
// /dev/null and aborts when the sandbox refuses. ps5_early.c runs first instead.
#![no_main]

// Every message goes to /data/ruffle/ruffle.log through ps5_early.c: fd 1
// didn't reach the file on the console. Defined before the modules so they
// use it too.
macro_rules! println {
    ($($arg:tt)*) => {
        $crate::log_line(&format!($($arg)*))
    };
}

mod audio;
mod badges;
mod covers;
mod cpu;
mod display;
mod hid;
mod input;
mod controls;
mod keys;
mod ps5ui;
mod quickmenu;
mod library;
mod saves;
mod settings;
mod sounds;
mod ui;

use std::any::Any;
use std::ffi::CString;
use std::fs;
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};

use ruffle_core::backend::navigator::{NullExecutor, NullNavigatorBackend};
use ruffle_core::config::Letterbox;
use ruffle_core::external::Value as ExternalValue;
use ruffle_core::{LoadBehavior, PlayerRuntime, StageAlign, StageScaleMode};
use ruffle_render::quality::StageQuality;
use ruffle_core::tag_utils::SwfMovie;
use ruffle_core::PlayerBuilder;
use ruffle_render_wgpu::backend::WgpuRenderBackend;
use ruffle_render_wgpu::target::TextureTarget;
use ruffle_video_software::backend::SoftwareVideoBackend;

use quickmenu::Action;
use ui::gfx::{self, Background, Canvas, accent, WHITE};
use ui::text::{Text, Weight};

const SCREEN_WIDTH: u32 = 1920;
const SCREEN_HEIGHT: u32 = 1080;
const FRAME_BYTES: usize = (SCREEN_WIDTH * SCREEN_HEIGHT * 4) as usize;

const FADE_IN: Duration = Duration::from_millis(350);
const FADE_OUT: Duration = Duration::from_millis(400);
/// A game without a cover gets one captured this far into its first run.
const AUTO_COVER_AFTER: Duration = Duration::from_secs(8);
/// With this folder present, R3 in a game records CAPTURE_FRAMES frames there.
const CAPTURE_DIR: &str = "/data/ruffle/capture";
const CAPTURE_FRAMES: usize = 40;
/// With this folder present, the game thread is profiled into the log.
const PROFILE_DIR: &str = "/data/ruffle/profile";

unsafe extern "C" {
    fn sceSystemServiceHideSplashScreen() -> i32;
    fn ruffle_ps5_stage(name: *const core::ffi::c_char);
    fn ruffle_ps5_log(text: *const core::ffi::c_char);
    fn ruffle_ps5_profile_register();
    fn ruffle_ps5_profile_start();
    fn ruffle_ps5_profile_stop();
    fn ruffle_ps5_profile_report(title: *const core::ffi::c_char);
    fn ruffle_ps5_notify(text: *const core::ffi::c_char);
}

fn write_log(text: &str) {
    if let Ok(c) = CString::new(text.replace('\0', " ")) {
        unsafe { ruffle_ps5_log(c.as_ptr()) };
    }
}

/// Writes a line to the log; a line repeating the one before is only counted
/// (games can warn the same thing every frame).
pub fn log_line(text: &str) {
    static LAST: std::sync::Mutex<(String, u32)> = std::sync::Mutex::new((String::new(), 0));
    let Ok(mut last) = LAST.lock() else {
        write_log(text);
        return;
    };
    if last.0 == text {
        last.1 += 1;
        return;
    }
    if last.1 > 0 {
        write_log(&format!("    (repeated {} more times)", last.1));
    }
    *last = (text.to_string(), 0);
    write_log(text);
}

fn notify(text: &str) {
    if let Ok(c) = CString::new(text.replace('\0', " ")) {
        unsafe { ruffle_ps5_notify(c.as_ptr()) };
    }
}

/// wgpu and its Vulkan layer report why an adapter or device was refused
/// through the `log` crate.
struct Ps5Logger;

impl log::Log for Ps5Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let target = metadata.target();
        // The game's own trace() output would flood the log.
        if target.starts_with("avm_trace") {
            return false;
        }
        metadata.level() <= log::Level::Info
            || (target.starts_with("wgpu") && metadata.level() <= log::Level::Debug)
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            log_line(&format!("[{}] {}: {}", record.level(), record.target(), record.args()));
        }
    }
    fn flush(&self) {}
}

static LOGGER: Ps5Logger = Ps5Logger;

fn install_diagnostics() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Debug);
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let place = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        log_line(&format!("*** Rust panic at {}: {}", place, message));
        notify(&format!("Ruffle panic: {}", message));
    }));
}

/// Names what runs now, for the crash toast and the log (ps5_early.c).
fn stage(name: &'static str) {
    let name = Box::leak(CString::new(name).unwrap().into_boxed_c_str());
    unsafe { ruffle_ps5_stage(name.as_ptr()) };
}

#[unsafe(no_mangle)]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    run();
    0
}

fn load_swf(path: &str) -> Vec<u8> {
    if path.is_empty() {
        println!("[Ruffle] Using built-in test SWF");
        return include_bytes!("test.swf").to_vec();
    }
    match fs::read(path) {
        Ok(data) => {
            println!("[Ruffle] Loaded: {} ({} bytes)", path, data.len());
            data
        }
        Err(e) => {
            println!("[Ruffle] Failed to read {}: {}", path, e);
            println!("[Ruffle] Falling back to built-in test");
            include_bytes!("test.swf").to_vec()
        }
    }
}

/// Ruffle's "Dummy External Interface" (as desktop/src/backends/external_interface.rs):
/// tells games the web page is there and answers their calls with nothing,
/// except the page address, which is the spoofed site when one is set.
struct DummyExternalInterface {
    spoof_url: Option<String>,
}

fn is_location_href(code: &str) -> bool {
    matches!(code, "document.location.href" | "window.location.href" | "top.location.href")
}

impl ruffle_core::external::ExternalInterfaceProvider for DummyExternalInterface {
    fn call_method(
        &self,
        _context: &mut ruffle_core::context::UpdateContext<'_>,
        name: &str,
        args: &[ExternalValue],
    ) -> ExternalValue {
        if let Some(ref url) = self.spoof_url {
            // e.g. "window.location.href.toString", or eval("window.location.href")
            let asks_location = name.strip_suffix(".toString").is_some_and(is_location_href)
                || (name == "eval" && matches!(args, [ExternalValue::String(code)] if is_location_href(code)));
            if asks_location {
                return ExternalValue::String(url.clone());
            }
        }
        ExternalValue::Undefined
    }

    fn on_callback_available(&self, _name: &str) {}

    fn get_id(&self) -> Option<String> {
        None
    }
}

/// Text in games that isn't embedded in the SWF ("Times New Roman", _sans...)
/// needs device fonts; the PS5 has none, so Inter stands in for all of them.
fn register_fonts(player: &mut ruffle_core::Player) {
    use ruffle_core::backend::ui::FontDefinition;
    use ruffle_core::font::{DefaultFont, FontFileData};
    for (data, is_bold) in [(ui::text::REGULAR, false), (ui::text::BOLD, true)] {
        player.register_device_font(FontDefinition::FontFile {
            name: "Inter".into(),
            is_bold,
            is_italic: false,
            data: FontFileData::new(data.to_vec()),
            index: 0,
        });
    }
    for font in [DefaultFont::Sans, DefaultFont::Serif, DefaultFont::Typewriter] {
        player.set_default_font(font, vec!["Inter".into()]);
    }
}

/// 0..1 eased (smoothstep), for fades.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn progress(since: Instant, length: Duration) -> f32 {
    since.elapsed().as_secs_f32() / length.as_secs_f32()
}

struct App {
    display: Option<display::Ps5Display>,
    input: input::Ps5Input,
    text: Text,
    splash_hidden: bool,
    /// The games' GPU device, made by the first game and kept.
    gpu: Option<std::sync::Arc<ruffle_render_wgpu::descriptors::Descriptors>>,
    bg: Background,
}

impl App {
    /// Shows a frame; paced by the display (FIFO), or ~60 Hz without one.
    fn present(&mut self, px: &[u8]) {
        match self.display {
            Some(ref d) => unsafe { d.present_frame(px) },
            None => std::thread::sleep(Duration::from_millis(16)),
        }
        if !self.splash_hidden {
            unsafe { sceSystemServiceHideSplashScreen() };
            self.splash_hidden = true;
            println!("[Ruffle] Splash screen hidden");
        }
    }

    /// Dims `base` to black (and the game's sound with it).
    fn fade_out(&mut self, base: &[u8], volume: Option<&AtomicU32>) {
        let start = Instant::now();
        loop {
            let t = progress(start, FADE_OUT);
            let mut px = base.to_vec();
            gfx::fade(&mut px, 1.0 - ease(t));
            if let Some(v) = volume {
                audio::set_volume(v, 1.0 - ease(t));
            }
            self.present(&px);
            if t >= 1.0 {
                break;
            }
        }
    }

    /// One frame of the loading screen: the app's background, the game's
    /// name and a spinner.
    fn loading_frame(&mut self, cv: &mut Canvas, name: &str, since: Instant, waves: bool) {
        self.bg.draw(cv, waves);
        let title = self.text.fit(Weight::Bold, 60, name, 1920 - 400);
        let tw = self.text.width(Weight::Bold, 60, &title);
        self.text.draw(cv, Weight::Bold, 60, (1920 - tw) / 2, 400, &title, WHITE, 1.0);
        let t = since.elapsed().as_secs_f32();
        gfx::spinner(cv, 960.0, 556.0, 34.0, 7.0, t, accent());
        let lw = self.text.width(Weight::SemiBold, 24, "Loading");
        self.text.draw(cv, Weight::SemiBold, 24, (1920 - lw) / 2, 622, "Loading", WHITE, 0.7);
        self.bg.overlay(cv);
        cv.fade(ease(t / FADE_IN.as_secs_f32()));
    }
}

/// The menus' background for a theme (Ember keeps its original base).
fn theme_background(theme: u8) -> gfx::Image {
    if theme == 0 {
        covers::make_backdrop("~empty", None)
    } else {
        gfx::theme_base(theme)
    }
}

fn run() {
    install_diagnostics();
    stage("rust main");
    println!("=== Ruffle Flash PS5 v{} ===", env!("CARGO_PKG_VERSION"));
    cpu::init();

    stage("display init");
    let display = match unsafe { display::Ps5Display::new() } {
        Ok(d) => {
            println!("[Ruffle] PS5 display initialized");
            Some(d)
        }
        Err(e) => {
            println!("[Ruffle] Display init failed: {} (running headless)", e);
            None
        }
    };

    stage("controller init");
    let input = input::Ps5Input::new().expect("controller");
    sounds::init();

    let mut library = library::Library::new();
    let theme = library.settings.theme;
    let mut app = App {
        display,
        input,
        text: Text::new(),
        splash_hidden: false,
        gpu: None,
        bg: Background::new(theme_background(theme), theme),
    };

    loop {
        stage("game library");
        let path = library_session(&mut app, &mut library);
        let name = library.selected_name();
        let key = library.selected_key();
        library.mark_played(&path);
        let mut settings = library.settings.clone();
        app.input.cursor_speed = settings.cursor_multiplier();
        // Restart (from the quick menu) plays it again from the start.
        let mut session = badges::Session::default();
        while game_session(&mut app, &path, &name, &key, &mut settings, &mut session) == GameEnd::Restart {
            session.restarts += 1;
            library.forget_cover(&key);
        }
        library.record_session(&key, &session);
        if settings.volume != library.settings.volume {
            library.settings.volume = settings.volume;
            library.settings.save();
        }
        app.input.keys_as_pad = true;
        library.forget_cover(&key);
        library.scan_files();
    }
}

/// The game library, fading in; returns the chosen game after fading out.
fn library_session(app: &mut App, library: &mut library::Library) -> String {
    let start = Instant::now();
    // L2: the on-screen keyboard, typing a search.
    let mut osk = keys::OnScreenKeys::new();
    let mut cv = Canvas::new(SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32);
    loop {
        app.input.mouse_speed = library.settings.mouse_multiplier();
        app.input.text_entry = library.searching();
        if library.settings.theme != app.bg.theme {
            let theme = library.settings.theme;
            app.bg.set_theme(theme_background(theme), theme);
        }
        let frame = app.input.read();
        for notice in app.input.hid_frame.notices.clone() {
            library.show_toast(notice);
        }
        if let Some(keyboard) = app.input.last_used_keyboard() {
            library.keyboard_hints = keyboard;
        }
        library.mouse_connected = app.input.mouse_connected();
        let mut mouse = app.input.menu_mouse();
        let mut frame = frame;
        if library.can_search() || osk.is_open() {
            let was_open = osk.is_open();
            let (events, used) = osk.handle(&frame);
            if osk.is_open() != was_open {
                sounds::play(if osk.is_open() { sounds::Sfx::Select } else { sounds::Sfx::Back });
            }
            use ruffle_core::events::{LogicalKey, NamedKey, PlayerEvent};
            for e in events {
                match e {
                    PlayerEvent::TextInput { codepoint } => mouse.typed.push(codepoint),
                    PlayerEvent::KeyDown { key } => match key.logical_key {
                        LogicalKey::Named(NamedKey::Backspace) => mouse.backspace = true,
                        LogicalKey::Named(NamedKey::Enter) => osk.set_open(false),
                        LogicalKey::Named(NamedKey::Escape) => {
                            library.clear_search();
                            osk.set_open(false);
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            if used {
                // The keyboard took the controller this frame.
                library.touch_input();
                frame = input::PadFrame { lx: 128, ly: 128, rx: 128, ry: 128, ..Default::default() };
            }
        }
        library.search_open = osk.is_open();
        if let Some(path) = library.handle_input(&frame, &mouse) {
            println!("[Ruffle] Selected: {}", if path.is_empty() { "(built-in)" } else { &path });
            let base = library.render(&mut app.text, &app.bg).to_vec();
            app.fade_out(&base, None);
            return path;
        }
        // Copy the frame only while it fades in.
        let t = progress(start, FADE_IN);
        if osk.is_open() {
            cv.px.copy_from_slice(library.render(&mut app.text, &app.bg));
            osk.draw(&mut cv, &mut app.text);
            app.present(&cv.px);
        } else if t < 1.0 {
            let mut px = library.render(&mut app.text, &app.bg).to_vec();
            gfx::fade(&mut px, ease(t));
            app.present(&px);
        } else {
            let px = library.render(&mut app.text, &app.bg);
            match app.display {
                Some(ref d) => unsafe { d.present_frame(px) },
                None => std::thread::sleep(Duration::from_millis(16)),
            }
        }
    }
}

/// A short message over the game ("Cover saved").
fn draw_game_toast(cv: &mut Canvas, text: &mut Text, msg: &str, since: Instant) {
    let t = since.elapsed().as_secs_f32();
    if t > 2.2 {
        return;
    }
    let a = ease((t / 0.2).min((2.2 - t) / 0.3));
    let w = text.width(Weight::SemiBold, 24, msg) + 56;
    let x = 1920 - 96 - w;
    cv.fill_round_rect(x, 56, w, 56, 28, gfx::INK, 0.8 * a);
    cv.fill_circle((x + 26) as f32, 84.0, 6.0, accent(), a);
    text.draw(cv, Weight::SemiBold, 24, x + 42, 70, msg, WHITE, a);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GameEnd {
    Library,
    Restart,
}

/// Plays one game until it's left from the quick menu (the touchpad opens
/// it), then fades back out.
fn game_session(
    app: &mut App,
    path: &str,
    name: &str,
    key: &str,
    settings: &mut settings::Settings,
    session: &mut badges::Session,
) -> GameEnd {
    app.input.keys_as_pad = false;
    app.input.text_entry = false;
    app.input.mouse_speed = settings.mouse_multiplier();
    // The file is read and parsed (and, the first time, the GPU device made)
    // on a worker while the loading screen's spinner turns.
    stage("load swf");
    let need_gpu = app.gpu.is_none();
    let job_path = path.to_string();
    let job = std::thread::spawn(move || {
        let swf_data = load_swf(&job_path);
        let url = if job_path.is_empty() {
            "file:///data/ruffle/test.swf".to_string()
        } else {
            format!("file://{}", job_path)
        };
        let movie = SwfMovie::from_data(&swf_data, url, None, None).map_err(|e| format!("can't read it: {}", e))?;
        // One GPU device for the app's life: games after the first froze when
        // each made (and dropped) its own.
        let gpu = if need_gpu {
            Some(
                WgpuRenderBackend::<TextureTarget>::offscreen_descriptors(
                    wgpu::Backends::VULKAN,
                    wgpu::PowerPreference::HighPerformance,
                )
                .map_err(|e| format!("renderer failed: {}", e))?,
            )
        } else {
            None
        };
        Ok::<_, String>((movie, gpu))
    });

    let shown = Instant::now();
    let mut cv = Canvas::new(SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32);
    // At least a moment of spinner, so the screen doesn't just flash by.
    while !job.is_finished() || shown.elapsed() < Duration::from_millis(700) {
        app.loading_frame(&mut cv, name, shown, settings.waves);
        app.present(&cv.px);
    }
    let (movie, gpu) = match job.join() {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            println!("*** {}: {}", path, e);
            notify(&format!("Ruffle: {}: {}", name, e));
            app.fade_out(&cv.px, None);
            return GameEnd::Library;
        }
        Err(_) => {
            notify(&format!("Ruffle: loading {} crashed", name));
            app.fade_out(&cv.px, None);
            return GameEnd::Library;
        }
    };
    if gpu.is_some() {
        app.gpu = gpu;
    }
    app.fade_out(&cv.px, None);
    let (movie_w, movie_h) = (movie.width().to_pixels() as f32, movie.height().to_pixels() as f32);
    println!(
        "[Ruffle] SWF: {}x{} @ {:.0} fps",
        movie_w,
        movie_h,
        movie.frame_rate().to_f32(),
    );

    stage("audio init");
    let (audio, volume) = match audio::Ps5AudioBackend::new() {
        Ok(a) => {
            let v = a.volume_handle();
            (Some(a), Some(v))
        }
        Err(e) => {
            println!("[Ruffle] Audio init failed: {} (muted)", e);
            (None, None)
        }
    };

    // Ruffle draws straight at the frame size and letterboxes the movie
    // itself: right aspect, and the cursor's screen coordinates are Ruffle's
    // viewport coordinates.
    stage("wgpu renderer init");
    let descriptors = app.gpu.clone().expect("gpu device");
    let renderer = match WgpuRenderBackend::<TextureTarget>::for_offscreen_with(descriptors, (SCREEN_WIDTH, SCREEN_HEIGHT)) {
        Ok(r) => r,
        Err(e) => {
            println!("*** wgpu renderer failed: {:?}", e);
            notify(&format!("Ruffle: renderer failed: {}", e));
            return GameEnd::Library;
        }
    };

    // Ruffle's background work (Loader.loadBytes, sounds, XML, ads...) runs
    // on this executor, every frame; without it such games wait forever.
    // Relative loads resolve beside the game's file.
    let mut executor = NullExecutor::new();
    let game_dir = std::path::Path::new(path).parent().filter(|_| !path.is_empty());
    let navigator = match game_dir.map(|d| NullNavigatorBackend::with_base_path(d, &executor)) {
        Some(Ok(n)) => n,
        Some(Err(e)) => {
            println!("[Ruffle] base path for {}: {} (relative loads off)", path, e);
            NullNavigatorBackend::with_executor(&executor)
        }
        None => NullNavigatorBackend::with_executor(&executor),
    };

    stage("player init");
    let ui_state = std::sync::Arc::new(ps5ui::UiState::default());
    let controls = controls::Controls::load(key);
    if controls.is_custom(key) {
        println!("[Ruffle] Custom controls for {}", key);
    }
    let mut builder = PlayerBuilder::new()
        .with_navigator(navigator)
        .with_movie(movie)
        .with_renderer(renderer)
        .with_video(SoftwareVideoBackend::new())
        .with_viewport_dimensions(SCREEN_WIDTH, SCREEN_HEIGHT, 1.0)
        .with_quality(
            [
                StageQuality::Low,
                StageQuality::Medium,
                StageQuality::High,
                StageQuality::Best,
                StageQuality::High8x8,
                StageQuality::High8x8Linear,
                StageQuality::High16x16,
                StageQuality::High16x16Linear,
            ][settings.quality as usize],
        )
        .with_ui(ps5ui::Ps5Ui { state: ui_state.clone() })
        .with_storage(Box::new(saves::DiskStorage::new(key)))
        .with_autoplay(true);
    // Ruffle's own display and player options (as in Ruffle on PC).
    let scale = [StageScaleMode::NoScale, StageScaleMode::ShowAll, StageScaleMode::ExactFit, StageScaleMode::NoBorder]
        [settings.scale_mode as usize];
    let align = [
        StageAlign::empty(),
        StageAlign::TOP,
        StageAlign::BOTTOM,
        StageAlign::LEFT,
        StageAlign::RIGHT,
        StageAlign::TOP | StageAlign::LEFT,
        StageAlign::TOP | StageAlign::RIGHT,
        StageAlign::BOTTOM | StageAlign::LEFT,
        StageAlign::BOTTOM | StageAlign::RIGHT,
    ][settings.align as usize];
    builder = builder
        .with_scale_mode(scale, settings.force_scale)
        .with_letterbox([Letterbox::On, Letterbox::Fullscreen, Letterbox::Off][settings.letterbox as usize])
        .with_align(align, settings.force_align)
        .with_player_runtime([PlayerRuntime::FlashPlayer, PlayerRuntime::AIR][settings.runtime as usize])
        .with_load_behavior(
            [LoadBehavior::Streaming, LoadBehavior::Delayed, LoadBehavior::Blocking][settings.load_behavior as usize],
        )
        .with_max_execution_duration(Duration::from_secs(
            settings::MAX_EXECUTION[settings.max_execution as usize] as u64,
        ));
    // A console app is always full screen; games see that, and Letterbox
    // "Fullscreen Only" does what it says.
    builder = builder.with_fullscreen(true);
    let spoof_url = settings.spoofed_url(path);
    if spoof_url.is_some() {
        builder = builder.with_spoofed_url(spoof_url.clone());
    }
    if settings.dummy_external_interface {
        builder = builder.with_external_interface(Box::new(DummyExternalInterface { spoof_url }));
    }
    match settings::FRAME_RATES[settings.frame_rate as usize] {
        0 => {}
        fps => builder = builder.with_frame_rate(Some(fps as f64)),
    }
    match settings::PLAYER_VERSIONS[settings.player_version as usize] {
        0 => {}
        v => builder = builder.with_player_version(Some(v)),
    }
    if let Some(a) = audio {
        builder = builder.with_audio(a);
    }
    let player = builder.build();
    {
        let mut p = player.lock().expect("player lock");
        register_fonts(&mut p);
        p.set_volume(settings.volume as f32 / 10.0);
    }
    // A captured cover is the movie's own area: zoomed to fit, the frame
    // minus the bars; otherwise the whole frame.
    let (cover_w, cover_h) = if settings.scale_mode == 1 {
        (movie_w, movie_h)
    } else {
        (SCREEN_WIDTH as f32, SCREEN_HEIGHT as f32)
    };
    println!("[Ruffle] Player started");
    stage("game loop");
    // Where the game thread's time goes, in the log every minute and at the
    // end; only with a /data/ruffle/profile folder (a developer's tool).
    let profiling = std::path::Path::new(PROFILE_DIR).is_dir();
    if profiling {
        unsafe {
            ruffle_ps5_profile_register();
            ruffle_ps5_profile_start();
        }
    }
    let mut profile_since = Instant::now();

    let mut keyboard = keys::OnScreenKeys::new();
    let mut keyboard_request: Option<(bool, Instant)> = None;
    let mut auto_opened = false;
    let mut last_frame = vec![0u8; FRAME_BYTES];
    let mut cv = Canvas::new(SCREEN_WIDTH as i32, SCREEN_HEIGHT as i32);
    let started = Instant::now();
    let mut last_tick = Instant::now();
    let mut renders: u64 = 0;
    let mut presents: u64 = 0;
    let mut need_auto_cover = settings.auto_covers && !path.is_empty() && !covers::has_cover(key);
    let mut fps_shown = 0.0f32;
    let mut fps_frames = 0u32;
    let mut fps_since = Instant::now();
    let mut toast: Option<(&str, Instant)> = None;
    // Time spent in the quick menu doesn't count as play time.
    let mut paused_for = Duration::ZERO;
    let mut menu_opened_at: Option<Instant> = None;
    let mut menu: Option<quickmenu::QuickMenu> = None;
    let mut end = GameEnd::Library;
    // The frame to fade out from when the game ends (the menu's, if left from it).
    let mut exit_frame: Option<Vec<u8>> = None;
    // The menu's hints show keyboard keys after the keyboard was used.
    let mut menu_hints = app.input.keyboard_plugged();
    let (mut ticks, mut drawn, mut capture_failed) = (0u64, 0u64, 0u64);
    let mut last_stats = Instant::now();
    // Where a frame's time goes (ms, summed over the 5 s): the game's code,
    // the jobs it started, drawing, reading the picture back, our overlays,
    // and showing it.
    let mut spent = [0.0f64; 6];
    let ms = |since: Instant| since.elapsed().as_secs_f64() * 1000.0;
    // wgpu's Debug lines were only wanted while it found the GPU.
    log::set_max_level(log::LevelFilter::Info);
    let capture_mode = std::path::Path::new(CAPTURE_DIR).is_dir();
    let (mut capturing, mut capture) = (false, Vec::<(f64, Vec<u8>)>::new());

    loop {
        let pad = app.input.read();
        for &notice in &app.input.hid_frame.notices {
            toast = Some((notice, Instant::now()));
        }
        if menu.is_none() {
            let hf = &app.input.hid_frame;
            session.keyboard |= hf.keys.iter().any(|&(_, down)| down);
            session.mouse |= hf.mouse_moved() || hf.pressed != 0;
        }

        // The quick menu, over the paused game.
        if let Some(m) = menu.as_mut() {
            app.input.move_pointer();
            let mouse = app.input.menu_mouse();
            let action = m.handle(&pad, &mouse, app.input.cursor());
            if let Action::Volume(v) = action {
                settings.volume = v;
                player.lock().expect("player lock").set_volume(v as f32 / 10.0);
            }
            match action {
                Action::None | Action::Volume(_) => {
                    if let Some(k) = app.input.last_used_keyboard() {
                        menu_hints = k;
                    }
                    let pointer = if app.input.mouse_connected() || app.input.hid_frame.mouse_moved() {
                        Some(app.input.cursor())
                    } else {
                        None
                    };
                    m.draw(&mut cv, &mut app.text, menu_hints, pointer);
                    app.present(&cv.px);
                    continue;
                }
                Action::Resume | Action::Cover => {
                    if action == Action::Cover && renders > 0 {
                        let (frame, key) = (last_frame.clone(), key.to_string());
                        std::thread::spawn(move || covers::save_cover(&key, &frame, cover_w, cover_h));
                        toast = Some(("Cover saved", Instant::now()));
                        need_auto_cover = false;
                        session.covers += 1;
                    }
                    if let Some(at) = menu_opened_at.take() {
                        paused_for += at.elapsed();
                    }
                    menu = None;
                    app.input.keys_as_pad = false;
                    player.lock().expect("player lock").set_is_playing(true);
                    last_tick = Instant::now();
                    println!("[Ruffle] Quick menu: resumed");
                    continue;
                }
                Action::Restart | Action::Library => {
                    end = if action == Action::Restart { GameEnd::Restart } else { GameEnd::Library };
                    println!("[Ruffle] Quick menu: {}", if end == GameEnd::Restart { "restart" } else { "back to the library" });
                    exit_frame = Some(cv.px.clone());
                    break;
                }
            }
        }

        // The touchpad (or Esc / Pause on a keyboard) opens the quick menu.
        // While the on-screen keyboard is open the touchpad drags it instead.
        let menu_key = app.input.hid_frame.key_pressed(hid::KEY_ESCAPE) || app.input.hid_frame.key_pressed(hid::KEY_PAUSE);
        if (pad.just_pressed(input::PAD_TOUCHPAD) && !keyboard.is_open()) || menu_key {
            println!("[Ruffle] Quick menu opened");
            session.quick_menus += 1;
            menu_opened_at = Some(Instant::now());
            sounds::play(sounds::Sfx::Select);
            let mut p = player.lock().expect("player lock");
            for e in keyboard.release_all().into_iter().chain(app.input.release_all(&controls)) {
                p.handle_event(e);
            }
            p.set_is_playing(false);
            drop(p);
            if keyboard.is_open() {
                keyboard.set_open(false);
            }
            app.input.keys_as_pad = true;
            menu = Some(quickmenu::QuickMenu::new(&last_frame, name, settings.volume));
            continue;
        }

        // Frame capture for tuning Flash Frame Generation: with a
        // /data/ruffle/capture folder, R3 records the next frames instead of
        // taking a cover.
        if capture_mode && pad.just_pressed(input::PAD_R3) && capture.is_empty() && !capturing {
            capturing = true;
            toast = Some(("Capturing frames...", Instant::now()));
        }

        // Covers: one taken automatically on the first run, R3 retakes it.
        let auto = need_auto_cover && renders > 0 && started.elapsed() >= AUTO_COVER_AFTER;
        if auto || (!capture_mode && pad.just_pressed(input::PAD_R3) && renders > 0) {
            // Cropped and encoded off the game loop, so the game doesn't hitch.
            let (frame, key) = (last_frame.clone(), key.to_string());
            std::thread::spawn(move || covers::save_cover(&key, &frame, cover_w, cover_h));
            if !auto {
                toast = Some(("Cover saved", Instant::now()));
                session.covers += 1;
            }
            need_auto_cover = false;
        }

        // A text box asking for the keyboard opens it, once the focus has
        // settled for a moment (some games flick focus on and off), unless
        // the player closed it themselves.
        if let Some(want) = ui_state.take_keyboard_request() {
            keyboard_request = Some((want, Instant::now()));
        }
        let was_open = keyboard.is_open();
        if let Some((want, at)) = keyboard_request {
            if at.elapsed() >= Duration::from_millis(400) {
                keyboard_request = None;
                // Not when a USB keyboard is there to type on.
                let allowed = settings.auto_keyboard && !keyboard.user_closed && !app.input.keyboard_plugged();
                if allowed && want && !was_open {
                    println!("[Ruffle] Text box focused: opening the keyboard");
                    keyboard.set_open(true);
                    auto_opened = true;
                } else if !want && was_open && auto_opened {
                    keyboard.set_open(false);
                    auto_opened = false;
                }
            }
        }

        let (mut events, used) = keyboard.handle(&pad);
        if keyboard.is_open() && !was_open {
            events.extend(app.input.release_all(&controls));
        }
        if !keyboard.is_open() {
            auto_opened = false;
        }
        if !used && !keyboard.is_open() {
            events.extend(app.input.game_events(&pad, &controls));
        }
        // A USB mouse and keyboard play the game whatever the controller does.
        events.extend(app.input.hid_events());

        {
            let mut p = player.lock().expect("player lock");
            for e in events {
                p.handle_event(e);
            }
            let now = Instant::now();
            let dt = (now - last_tick).min(Duration::from_millis(100));
            last_tick = now;
            p.tick(ruffle_core::FloatDuration::from_std(dt));
            drop(p);
            spent[0] += ms(now);
            let t = Instant::now();
            executor.run();
            spent[1] += ms(t);
            let mut p = player.lock().expect("player lock");

            ticks += 1;
            if p.needs_render() {
                let t = Instant::now();
                p.render();
                spent[2] += ms(t);
                drawn += 1;
                let renderer: &dyn Any = p.renderer();
                if let Some(wgpu_backend) = renderer.downcast_ref::<WgpuRenderBackend<TextureTarget>>() {
                    // Straight into the frame we show, copied on 8 threads.
                    let t = Instant::now();
                    let captured = wgpu_backend.capture_frame_into(&mut last_frame);
                    spent[3] += ms(t);
                    if !captured {
                        capture_failed += 1;
                    } else {
                        renders += 1;
                        fps_frames += 1;
                        if capturing {
                            capture.push((started.elapsed().as_secs_f64(), last_frame.clone()));
                            if capture.len() == CAPTURE_FRAMES {
                                capturing = false;
                                // Raw RGBA 1920x1080 per frame, plus when each was made.
                                let frames = std::mem::take(&mut capture);
                                std::thread::spawn(move || {
                                    let mut times = String::new();
                                    for (i, (t, px)) in frames.iter().enumerate() {
                                        let _ = std::fs::write(format!("{}/frame_{:02}.rgba", CAPTURE_DIR, i), px);
                                        times.push_str(&format!("{} {:.4}\n", i, t));
                                    }
                                    let _ = std::fs::write(format!("{}/times.txt", CAPTURE_DIR), times);
                                    println!("[Ruffle] Captured {} frames to {}", frames.len(), CAPTURE_DIR);
                                });
                                toast = Some(("Frames captured", Instant::now()));
                            }
                        }
                        if renders == 1 {
                            println!("[Ruffle] First game frame rendered");
                        }
                        if renders % 300 == 0 {
                            let secs = started.elapsed().as_secs_f64();
                            println!(
                                "[Ruffle] {} game frames ({:.1} fps), {} presents ({:.1} fps)",
                                renders,
                                renders as f64 / secs,
                                presents,
                                presents as f64 / secs
                            );
                        }
                    }
                }
            }
        }

        if last_stats.elapsed() >= Duration::from_secs(5) {
            let p = player.lock().expect("player lock");
            println!(
                "[Game] 5 s: {} ticks, {} frames drawn, {} read back, {} readbacks failed, root frame {:?}, playing {}",
                ticks,
                drawn,
                drawn - capture_failed,
                capture_failed,
                p.current_frame(),
                p.is_playing()
            );
            drop(p);
            let per = |v: f64| v / ticks.max(1) as f64;
            println!(
                "[Game] ms per loop: game {:.1}, jobs {:.1}, draw {:.1} | per frame drawn: readback {:.1} | overlay {:.1}, present {:.1}",
                per(spent[0]),
                per(spent[1]),
                per(spent[2]),
                spent[3] / drawn.max(1) as f64,
                per(spent[4]),
                per(spent[5]),
            );
            spent = [0.0; 6];
            if profiling && profile_since.elapsed() >= Duration::from_secs(60) {
                unsafe {
                    ruffle_ps5_profile_report(c"last minute".as_ptr());
                    ruffle_ps5_profile_start();
                }
                profile_since = Instant::now();
            }
            (ticks, drawn, capture_failed) = (0, 0, 0);
            last_stats = Instant::now();
        }

        let t = Instant::now();
        cv.px.copy_from_slice(&last_frame);
        // No arrow when the game hides the mouse (it draws its own), or
        // when the left stick plays as keys and the cursor sits idle.
        if !keyboard.is_open() && !ui_state.mouse_hidden() {
            let (cx, cy) = app.input.cursor();
            gfx::pointer(&mut cv, cx as f32, cy as f32, 1.0);
        }
        keyboard.draw(&mut cv, &mut app.text);
        if let Some((msg, at)) = toast {
            draw_game_toast(&mut cv, &mut app.text, msg, at);
        }
        if settings.fps_counter {
            let secs = fps_since.elapsed().as_secs_f32();
            if secs >= 0.5 {
                fps_shown = fps_frames as f32 / secs;
                fps_frames = 0;
                fps_since = Instant::now();
            }
            let label = format!("{:.0} FPS", fps_shown);
            let w = app.text.width(Weight::SemiBold, 22, &label) + 28;
            cv.fill_round_rect(24, 24, w, 40, 20, gfx::INK, 0.75);
            app.text.draw(&mut cv, Weight::SemiBold, 22, 38, 32, &label, WHITE, 1.0);
        }
        cv.fade(ease(progress(started, FADE_IN)));
        spent[4] += ms(t);
        let t = Instant::now();
        app.present(&cv.px);
        spent[5] += ms(t);
        presents += 1;
    }

    // Let go of everything, fade picture and sound together, then end the game.
    {
        let mut p = player.lock().expect("player lock");
        for e in keyboard.release_all().into_iter().chain(app.input.release_all(&controls)) {
            p.handle_event(e);
        }
    }
    app.fade_out(exit_frame.as_deref().unwrap_or(&last_frame), volume.as_deref());
    {
        let mut p = player.lock().expect("player lock");
        p.flush_shared_objects();
        p.set_is_playing(false);
    }
    drop(player);
    if profiling {
        unsafe {
            ruffle_ps5_profile_report(c"until the game closed".as_ptr());
            ruffle_ps5_profile_stop();
        }
    }
    let in_menu = menu_opened_at.map_or(Duration::ZERO, |at| at.elapsed());
    session.secs += started.elapsed().saturating_sub(paused_for + in_menu).as_secs();
    println!("[Ruffle] Game closed");
    end
}
