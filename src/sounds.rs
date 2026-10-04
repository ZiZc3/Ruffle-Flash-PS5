//! Menu sounds: soft, quiet chimes made on the fly (no sound files), played
//! on an audio port of their own. Kept calm and low on purpose.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};

const RATE: f32 = 48000.0;
const GRAIN: usize = 512;
/// The loudest any menu sound gets (of full scale).
const LEVEL: f32 = 0.11;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sfx {
    /// Selection moved.
    Move,
    /// Tab switched.
    Tab,
    /// Opened or picked something.
    Select,
    /// Went back or closed.
    Back,
    /// A setting changed, a favorite toggled.
    Toggle,
    /// A game starts.
    Launch,
    /// A badge unlocked.
    Badge,
    /// The screensaver begins / ends.
    Sleep,
    Wake,
}

unsafe extern "C" {
    fn sceAudioOutOpen(user_id: i32, port_type: i32, index: i32, len: u32, freq: u32, param: u32) -> i32;
    fn sceAudioOutOutput(handle: i32, buf: *const i16) -> i32;
}

static ENABLED: AtomicBool = AtomicBool::new(true);
static QUEUE: OnceLock<Mutex<Sender<Sfx>>> = OnceLock::new();

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Plays a menu sound (when they're on).
pub fn play(sfx: Sfx) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(q) = QUEUE.get() {
        if let Ok(q) = q.lock() {
            let _ = q.send(sfx);
        }
    }
}

/// One soft bell-like note: a sine with a quiet octave, a smooth attack and
/// a long fade.
fn note(out: &mut Vec<f32>, start: f32, freq: f32, gain: f32, attack: f32, decay: f32) {
    let s0 = (start * RATE) as usize;
    let len = ((attack + decay * 5.0) * RATE) as usize;
    if out.len() < s0 + len {
        out.resize(s0 + len, 0.0);
    }
    for i in 0..len {
        let t = i as f32 / RATE;
        let env = if t < attack { (t / attack).powi(2) } else { (-(t - attack) / decay).exp() };
        let v = (TAU * freq * t).sin() + 0.18 * (TAU * freq * 2.0 * t).sin() * (-t / (decay * 0.4)).exp();
        out[s0 + i] += v * env * gain;
    }
}

fn synth(sfx: Sfx) -> Vec<f32> {
    let mut o = Vec::new();
    match sfx {
        Sfx::Move => note(&mut o, 0.0, 1318.5, 0.22, 0.002, 0.025),
        Sfx::Toggle => note(&mut o, 0.0, 987.8, 0.3, 0.003, 0.04),
        Sfx::Tab => {
            note(&mut o, 0.0, 659.3, 0.3, 0.006, 0.06);
            note(&mut o, 0.035, 987.8, 0.22, 0.006, 0.07);
        }
        Sfx::Select => {
            note(&mut o, 0.0, 784.0, 0.32, 0.004, 0.07);
            note(&mut o, 0.055, 1174.7, 0.3, 0.004, 0.1);
        }
        Sfx::Back => {
            note(&mut o, 0.0, 987.8, 0.28, 0.004, 0.06);
            note(&mut o, 0.05, 659.3, 0.26, 0.004, 0.09);
        }
        Sfx::Launch => {
            // A warm chord swelling in and floating away.
            for (i, f) in [261.6, 329.6, 392.0, 523.3].iter().enumerate() {
                note(&mut o, i as f32 * 0.03, *f, 0.22, 0.09, 0.32);
            }
        }
        Sfx::Badge => {
            // A gentle rising sparkle, then a soft shimmer.
            for (i, f) in [784.0, 987.8, 1174.7, 1568.0].iter().enumerate() {
                note(&mut o, i as f32 * 0.085, *f, 0.3, 0.004, 0.16);
            }
            note(&mut o, 0.36, 2093.0, 0.12, 0.02, 0.3);
            note(&mut o, 0.36, 1568.0, 0.14, 0.02, 0.35);
        }
        Sfx::Sleep => {
            note(&mut o, 0.0, 392.0, 0.25, 0.25, 0.4);
            note(&mut o, 0.12, 329.6, 0.2, 0.25, 0.45);
        }
        Sfx::Wake => {
            note(&mut o, 0.0, 329.6, 0.2, 0.08, 0.2);
            note(&mut o, 0.07, 523.3, 0.22, 0.08, 0.25);
        }
    }
    o
}

/// Opens the menu-sound port and its thread.
pub fn init() {
    if QUEUE.get().is_some() {
        return;
    }
    if let Err(e) = crate::audio::init_audio_out() {
        println!("[Sounds] {}", e);
        return;
    }
    let handle = unsafe { sceAudioOutOpen(0xFF, 0, 0, GRAIN as u32, 48000, 1) };
    if handle < 0 {
        println!("[Sounds] no audio port: {:#x}", handle);
        return;
    }
    let (tx, rx) = channel::<Sfx>();
    let _ = QUEUE.set(Mutex::new(tx));
    let spawned = std::thread::Builder::new().name("ui-sounds".into()).spawn(move || {
        // Sounds still playing: (samples, position).
        let mut voices: Vec<(Arc<Vec<f32>>, usize)> = Vec::new();
        let mut cache: Vec<(Sfx, Arc<Vec<f32>>)> = Vec::new();
        let mut buf = [0i16; GRAIN * 2];
        loop {
            // Idle: wait for a sound without spinning.
            let first = if voices.is_empty() {
                match rx.recv() {
                    Ok(s) => Some(s),
                    Err(_) => return,
                }
            } else {
                None
            };
            for sfx in first.into_iter().chain(rx.try_iter()) {
                let clip = match cache.iter().find(|(s, _)| *s == sfx) {
                    Some((_, c)) => c.clone(),
                    None => {
                        let c = Arc::new(synth(sfx));
                        cache.push((sfx, c.clone()));
                        c
                    }
                };
                // The same sound again (fast scrolling) restarts it.
                voices.retain(|(c, _)| !Arc::ptr_eq(c, &clip));
                voices.push((clip, 0));
            }
            for i in 0..GRAIN {
                let mut v = 0.0;
                for (clip, pos) in voices.iter() {
                    if let Some(s) = clip.get(pos + i) {
                        v += s;
                    }
                }
                // Soft limit, then the quiet menu level.
                let s = (v.tanh() * LEVEL * 32767.0) as i16;
                buf[i * 2] = s;
                buf[i * 2 + 1] = s;
            }
            for (_, pos) in voices.iter_mut() {
                *pos += GRAIN;
            }
            voices.retain(|(c, pos)| *pos < c.len());
            unsafe { sceAudioOutOutput(handle, buf.as_ptr()) };
            if voices.is_empty() {
                // One silent block, so the port doesn't hold the last one.
                buf.fill(0);
                unsafe { sceAudioOutOutput(handle, buf.as_ptr()) };
            }
        }
    });
    match spawned {
        Ok(_) => println!("[Sounds] ready (port {})", handle),
        Err(_) => println!("[Sounds] couldn't start"),
    }
}
