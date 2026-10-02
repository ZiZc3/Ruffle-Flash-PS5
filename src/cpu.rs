//! CPU cores, as XPSemu and PS5SX2 lay them out: the sound gets a core of
//! its own (its hyperthread sibling left idle), every other thread the rest.
//! On the PS5 a new thread inherits its creator's CPUs, so without this the
//! sound could share a core with a busy game and miss its deadline (crackle).
use std::sync::atomic::{AtomicI32, Ordering};

unsafe extern "C" {
    fn pthread_self() -> usize;
    fn scePthreadGetaffinity(thread: usize, mask: *mut u64) -> i32;
    fn scePthreadSetaffinity(thread: usize, mask: u64) -> i32;
    fn cpuset_setaffinity(level: i32, which: i32, id: i64, size: usize, mask: *const u8) -> i32;
}
const CPU_LEVEL_WHICH: i32 = 3;
const CPU_WHICH_PID: i32 = 2;
const ERANGE: i32 = 34;

static AUDIO_CPU: AtomicI32 = AtomicI32::new(-1);

/// Once, at start-up: keeps the top whole core for the sound and moves every
/// thread to the CPUs left over.
pub fn init() {
    let mut all = 0u64;
    if unsafe { scePthreadGetaffinity(pthread_self(), &mut all) } != 0 || all == 0 {
        println!("[PS5] cpus: no CPU mask, layout skipped");
        return;
    }
    let Some(k) = (0..32).rev().find(|k| all & (3u64 << (2 * k)) == 3u64 << (2 * k)) else {
        println!("[PS5] cpus {:#x}: no whole core free, layout skipped", all);
        return;
    };
    let rest = all & !(3u64 << (2 * k));
    if rest == 0 {
        println!("[PS5] cpus {:#x}: one core only, layout skipped", all);
        return;
    }
    // The console's kernel may want a smaller set than FreeBSD's (PS5SX2
    // found 8, 16 or 32 bytes).
    let mut set = [0u8; 32];
    set[..8].copy_from_slice(&rest.to_le_bytes());
    let mut rc = -1;
    for size in [8usize, 16, 32] {
        rc = if unsafe { cpuset_setaffinity(CPU_LEVEL_WHICH, CPU_WHICH_PID, -1, size, set.as_ptr()) } == 0 {
            0
        } else {
            std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)
        };
        if rc != ERANGE {
            break;
        }
    }
    AUDIO_CPU.store(2 * k as i32, Ordering::Relaxed);
    println!("[PS5] cpus {:#x}: sound on CPU {}, others {:#x} (rc {})", all, 2 * k, rest, rc);
}

/// Called by the sound thread when it starts.
pub fn pin_audio_thread() {
    let cpu = AUDIO_CPU.load(Ordering::Relaxed);
    if cpu >= 0 {
        let rc = unsafe { scePthreadSetaffinity(pthread_self(), 1u64 << cpu) };
        if rc != 0 {
            println!("[PS5] sound thread to CPU {}: {:#x}", cpu, rc as u32);
        }
    }
}
