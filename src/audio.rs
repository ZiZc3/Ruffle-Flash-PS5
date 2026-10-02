use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ruffle_core::backend::audio::{
    AudioBackend, AudioMixer, DecodeError, RegisterError, SoundHandle, SoundInstanceHandle,
    SoundStreamInfo, SoundTransform, swf,
};
use ruffle_core::impl_audio_mixer_backend;

const SAMPLE_RATE: u32 = 48000;
const CHANNELS: u8 = 2;
/// Samples per output block (21 ms at 48 kHz). 256 (5 ms) crackled whenever
/// a heavy game kept the CPU busy for a moment.
const GRAIN_SIZE: u32 = 1024;

const SCE_AUDIO_OUT_PORT_TYPE_MAIN: i32 = 0;
const SCE_AUDIO_OUT_PARAM_FORMAT_S16_STEREO: u32 = 1;

unsafe extern "C" {
    fn sceAudioOutInit() -> i32;
    fn sceAudioOutOpen(
        user_id: i32,
        port_type: i32,
        index: i32,
        len: u32,
        freq: u32,
        param: u32,
    ) -> i32;
    fn sceAudioOutOutput(handle: i32, buf: *const i16) -> i32;
    fn sceAudioOutClose(handle: i32) -> i32;
}

pub struct Ps5AudioBackend {
    mixer: AudioMixer,
    playing: Arc<AtomicBool>,
    /// Cleared when the game ends: the output thread closes its port and exits.
    running: Arc<AtomicBool>,
    /// Output gain as f32 bits, for fading the game out.
    volume: Arc<AtomicU32>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Ps5AudioBackend {
    /// A handle on the output gain (0.0..1.0) that outlives handing the
    /// backend to the player.
    pub fn volume_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.volume)
    }
}

pub fn set_volume(handle: &AtomicU32, gain: f32) {
    handle.store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

impl Drop for Ps5AudioBackend {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // Wait for the port to close, so the next game can open its own.
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Ps5AudioBackend {
    pub fn new() -> Result<Self, String> {
        println!("[PS5] Initializing audio...");

        // Once per app run: a second sceAudioOutInit fails (0x8026000e), which
        // left every game after the first one muted.
        static INITIALIZED: AtomicBool = AtomicBool::new(false);
        if !INITIALIZED.load(Ordering::Relaxed) {
            let ret = unsafe { sceAudioOutInit() };
            if ret < 0 && ret != 0x8026000e_u32 as i32 {
                return Err(format!("sceAudioOutInit failed: 0x{:08x}", ret as u32));
            }
            INITIALIZED.store(true, Ordering::Relaxed);
        }

        let handle = unsafe {
            sceAudioOutOpen(
                0xFF,
                SCE_AUDIO_OUT_PORT_TYPE_MAIN,
                0,
                GRAIN_SIZE,
                SAMPLE_RATE,
                SCE_AUDIO_OUT_PARAM_FORMAT_S16_STEREO,
            )
        };
        if handle < 0 {
            return Err(format!("sceAudioOutOpen failed: 0x{:08x}", handle as u32));
        }

        println!(
            "[PS5] Audio port opened (handle={}, {}Hz, grain={})",
            handle, SAMPLE_RATE, GRAIN_SIZE
        );

        let mixer = AudioMixer::new(CHANNELS, SAMPLE_RATE);
        let playing = Arc::new(AtomicBool::new(true));

        let proxy = mixer.proxy();
        let playing_flag = Arc::clone(&playing);
        let running = Arc::new(AtomicBool::new(true));
        let running_flag = Arc::clone(&running);
        let volume = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let volume_flag = Arc::clone(&volume);

        let thread = std::thread::Builder::new()
            .name("ps5-audio".into())
            .spawn(move || {
                crate::cpu::pin_audio_thread();
                let buf_len = (GRAIN_SIZE * CHANNELS as u32) as usize;
                let mut buffer = vec![0i16; buf_len];
                // A block handed over later than it lasts is a gap you hear.
                let grain = Duration::from_secs_f64(GRAIN_SIZE as f64 / SAMPLE_RATE as f64);
                let (mut late, mut worst_mix) = (0u32, Duration::ZERO);
                let mut last_out = std::time::Instant::now();
                let mut last_report = std::time::Instant::now();
                while running_flag.load(Ordering::Relaxed) {
                    if playing_flag.load(Ordering::Relaxed) {
                        let t = std::time::Instant::now();
                        proxy.mix::<i16>(&mut buffer);
                        worst_mix = worst_mix.max(t.elapsed());
                        let gain = f32::from_bits(volume_flag.load(Ordering::Relaxed));
                        if gain < 1.0 {
                            for s in buffer.iter_mut() {
                                *s = (*s as f32 * gain) as i16;
                            }
                        }
                    } else {
                        buffer.fill(0);
                    }
                    if last_out.elapsed() > grain * 2 {
                        late += 1;
                    }
                    let ret = unsafe { sceAudioOutOutput(handle, buffer.as_ptr()) };
                    last_out = std::time::Instant::now();
                    if ret < 0 {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    if last_report.elapsed() >= Duration::from_secs(5) {
                        if late > 0 {
                            println!(
                                "[Audio] {} late blocks in 5 s (slowest mix {:.1} ms)",
                                late,
                                worst_mix.as_secs_f64() * 1000.0
                            );
                        }
                        (late, worst_mix) = (0, Duration::ZERO);
                        last_report = std::time::Instant::now();
                    }
                }
                // Drain what's queued, then free the port for the next game.
                unsafe {
                    sceAudioOutOutput(handle, core::ptr::null());
                    sceAudioOutClose(handle);
                }
                println!("[PS5] Audio port {} closed", handle);
            })
            .map_err(|e| format!("Failed to spawn audio thread: {}", e))?;

        println!("[PS5] Audio backend ready");

        Ok(Ps5AudioBackend { mixer, playing, running, volume, thread: Some(thread) })
    }
}

impl AudioBackend for Ps5AudioBackend {
    impl_audio_mixer_backend!(mixer);

    fn play(&mut self) {
        self.playing.store(true, Ordering::Relaxed);
    }

    fn pause(&mut self) {
        self.playing.store(false, Ordering::Relaxed);
    }
}
