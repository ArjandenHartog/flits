//! Beat tracking for music: spectral-flux onsets, tempo by autocorrelation and
//! dynamic-programming beat placement (Ellis 2007). Good enough to cut on the beat.

use super::audio::decode_audio;
use anyhow::Result;
use rustfft::{num_complex::Complex, FftPlanner};
use std::path::Path;

const RATE: u32 = 22050;
const N: usize = 1024;
const HOP: usize = 256;

/// Onset strength per hop (rectified log-spectral flux, mean removed).
fn onset_envelope(pcm: &[f32]) -> Vec<f32> {
    let frames = pcm.len().saturating_sub(N) / HOP;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(N);
    let window: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
    let mut prev = vec![0f32; N / 2];
    let mut buf = vec![Complex::new(0f32, 0f32); N];
    let mut env = Vec::with_capacity(frames);
    for f in 0..frames {
        for i in 0..N {
            buf[i] = Complex::new(pcm[f * HOP + i] * window[i], 0.0);
        }
        fft.process(&mut buf);
        let mut flux = 0.0;
        for b in 1..N / 2 {
            let m = (1.0 + 10.0 * buf[b].norm()).ln();
            flux += (m - prev[b]).max(0.0);
            prev[b] = m;
        }
        env.push(flux);
    }
    // remove the local mean (~0.5 s) and keep the positive part
    let w = (0.5 * RATE as f32 / HOP as f32) as usize;
    let mut out = vec![0f32; env.len()];
    let mut sum = 0.0;
    for i in 0..env.len() {
        sum += env[i];
        if i >= w {
            sum -= env[i - w];
        }
        let mean = sum / (i.min(w - 1) + 1) as f32;
        out[i] = (env[i] - mean).max(0.0);
    }
    let max = out.iter().cloned().fold(1e-9, f32::max);
    out.iter().map(|v| v / max).collect()
}

/// Beat period in hops, by autocorrelation weighted toward ~120 BPM.
fn tempo(env: &[f32]) -> f32 {
    let hps = RATE as f32 / HOP as f32;
    let (lo, hi) = ((hps * 60.0 / 190.0) as usize, (hps * 60.0 / 60.0) as usize);
    let mut best = (0.0f32, hps * 0.5);
    for lag in lo..=hi.min(env.len().saturating_sub(1)) {
        let ac: f32 = env.iter().zip(&env[lag..]).map(|(a, b)| a * b).sum();
        let bpm = 60.0 * hps / lag as f32;
        let w = (-0.5 * ((bpm / 120.0).log2() / 0.9).powi(2)).exp();
        if ac * w > best.0 {
            best = (ac * w, lag as f32);
        }
    }
    best.1
}

/// Beat times in seconds within [start, start + dur) of the file.
pub fn beats(path: &Path, start: f64, dur: f64) -> Result<Vec<f64>> {
    let pcm = decode_audio(path, start, dur, RATE)?;
    let env = onset_envelope(&pcm);
    if env.len() < 16 {
        return Ok(vec![]);
    }
    let period = tempo(&env);
    // dynamic programming: score = onset + best previous score with a tempo penalty
    let tightness = 100.0f32;
    let mut score = env.clone();
    let mut back = vec![usize::MAX; env.len()];
    for t in 0..env.len() {
        let lo = (t as f32 - 2.0 * period).max(0.0) as usize;
        let hi = (t as f32 - period / 2.0).max(0.0) as usize;
        let mut best = (f32::MIN, usize::MAX);
        for (p, sp) in score.iter().enumerate().take(hi).skip(lo) {
            let d = ((t - p) as f32 / period).ln();
            let s = sp - tightness * d * d;
            if s > best.0 {
                best = (s, p);
            }
        }
        if best.1 != usize::MAX {
            score[t] = env[t] + best.0.max(0.0);
            back[t] = best.1;
        }
    }
    let tail = (env.len() as f32 - period).max(0.0) as usize;
    let mut t = (tail..env.len()).max_by(|a, b| score[*a].partial_cmp(&score[*b]).unwrap()).unwrap_or(env.len() - 1);
    let mut out = vec![];
    loop {
        out.push(t as f64 * HOP as f64 / RATE as f64);
        if back[t] == usize::MAX {
            break;
        }
        t = back[t];
    }
    out.reverse();
    Ok(out)
}

/// Tempo in BPM implied by a beat list.
pub fn bpm(beats: &[f64]) -> Option<f64> {
    if beats.len() < 3 {
        return None;
    }
    let mut d: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    d.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some(60.0 / d[d.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_tempo_of_click_track() {
        // 120 BPM clicks
        let mut pcm = vec![0f32; RATE as usize * 10];
        for b in 0..20 {
            let s = (b as f64 * 0.5 * RATE as f64) as usize;
            for i in 0..200 {
                pcm[s + i] = ((i as f32 * 0.3).sin()) * (1.0 - i as f32 / 200.0);
            }
        }
        let env = onset_envelope(&pcm);
        let period_s = tempo(&env) * HOP as f32 / RATE as f32;
        assert!((period_s - 0.5).abs() < 0.03, "period {period_s}");
    }
}
