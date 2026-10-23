//! Audio spectrum bars (audiogram), analysed once up front.

use crate::media::audio;
use anyhow::Result;
use rayon::prelude::*;
use rustfft::{num_complex::Complex, FftPlanner};
use std::path::Path;

pub struct Spectrum {
    /// per output frame: `bars` values in 0..1
    pub frames: Vec<Vec<f32>>,
    pub bars: usize,
}

impl Spectrum {
    pub fn at(&self, frame: i64) -> Option<&[f32]> {
        if self.frames.is_empty() {
            return None;
        }
        Some(&self.frames[frame.clamp(0, self.frames.len() as i64 - 1) as usize])
    }
}

pub fn analyse(src: &Path, trim: f64, dur: f64, fps: f64, bars: usize, gain: f32) -> Result<Spectrum> {
    const RATE: u32 = 22050;
    const N: usize = 2048;
    let pcm = audio::decode_audio(src, trim, dur, RATE)?;
    let nframes = (dur * fps).ceil() as usize;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(N);
    let window: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
    // log-spaced band edges 60 Hz .. 8 kHz
    let (lo, hi) = (60f32, 8000f32);
    let bin_hz = RATE as f32 / N as f32;
    let edges: Vec<usize> = (0..=bars)
        .map(|i| {
            let f = lo * (hi / lo).powf(i as f32 / bars as f32);
            ((f / bin_hz) as usize).clamp(1, N / 2 - 1)
        })
        .collect();

    let raw: Vec<Vec<f32>> = (0..nframes)
        .into_par_iter()
        .map_init(
            || vec![Complex::new(0f32, 0f32); N],
            |buf, f| {
                let center = (f as f64 / fps * RATE as f64) as isize;
                for i in 0..N {
                    let idx = center - (N as isize / 2) + i as isize;
                    let s = if idx >= 0 && (idx as usize) < pcm.len() { pcm[idx as usize] } else { 0.0 };
                    buf[i] = Complex::new(s * window[i], 0.0);
                }
                fft.process(buf);
                (0..bars)
                    .map(|b| {
                        let (a, z) = (edges[b], edges[b + 1].max(edges[b] + 1));
                        let m = buf[a..z].iter().map(|c| c.norm()).fold(0.0f32, f32::max);
                        // dB, then map -60..0 dB to 0..1 (with gentle tilt so highs show up)
                        let tilt = 1.0 + b as f32 / bars as f32 * 1.5;
                        let db = 20.0 * (m * tilt * gain / (N as f32 / 4.0) + 1e-9).log10();
                        ((db + 60.0) / 60.0).clamp(0.0, 1.0)
                    })
                    .collect()
            },
        )
        .collect();

    // fast attack, slower release
    let mut frames = Vec::with_capacity(raw.len());
    let mut prev = vec![0f32; bars];
    for r in raw {
        for b in 0..bars {
            let v = r[b];
            prev[b] = if v > prev[b] { prev[b] + (v - prev[b]) * 0.7 } else { prev[b] * 0.82 + v * 0.18 };
        }
        frames.push(prev.iter().map(|v| v.powf(1.4)).collect());
    }
    Ok(Spectrum { frames, bars })
}
