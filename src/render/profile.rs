//! Cheap per-stage timers (summed over all workers). Printed with `FLITS_PROFILE=1`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Copy)]
pub enum Stage {
    /// pulling decoded frames out of ffmpeg
    Decode,
    /// walking the scene tree, evaluating animations
    Compose,
    /// GPU submit + wait + readback
    Gpu,
    /// software rasterization + colour conversion
    Cpu,
    /// writing frames into the encoder pipe
    Encode,
    /// encoder start/finish per chunk
    EncoderSetup,
}

const N: usize = 6;
const NAMES: [&str; N] = ["video decode", "compose", "gpu render+readback", "cpu raster+yuv", "encoder pipe", "encoder start/stop"];
static TOTALS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

pub fn enabled() -> bool {
    std::env::var_os("FLITS_PROFILE").is_some()
}

pub fn time<T>(stage: Stage, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    TOTALS[stage as usize].fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
    r
}

pub fn reset() {
    for t in &TOTALS {
        t.store(0, Ordering::Relaxed);
    }
}

/// Summary: seconds per stage summed over all workers, and share of worker time.
pub fn report(workers: usize, wall: f64) -> String {
    let secs: Vec<f64> = TOTALS.iter().map(|t| t.load(Ordering::Relaxed) as f64 / 1e9).collect();
    let busy = workers as f64 * wall;
    let mut s = format!("profile ({workers} workers x {wall:.1}s = {busy:.1} worker-seconds):\n");
    for (i, name) in NAMES.iter().enumerate() {
        if secs[i] > 0.0 {
            s += &format!("  {name:<22} {:>7.1}s  {:>5.1}%\n", secs[i], secs[i] / busy * 100.0);
        }
    }
    s
}
