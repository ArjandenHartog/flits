//! Audio: decoding to PCM and mixing the scene's soundtrack.

use super::{ffmpeg, run};
use crate::compose::Prepared;
use anyhow::{bail, Context, Result};
use std::path::Path;

/// Decode mono f32 PCM at `rate` for [start, start+dur).
pub fn decode_audio(path: &Path, start: f64, dur: f64, rate: u32) -> Result<Vec<f32>> {
    let out = ffmpeg()
        .args(["-ss", &format!("{start:.3}"), "-t", &format!("{dur:.3}")])
        .arg("-i")
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", &rate.to_string(), "-f", "f32le", "-"])
        .output()
        .context("running ffmpeg")?;
    if !out.status.success() {
        bail!("ffmpeg could not decode audio from {}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect())
}

pub fn mix(p: &Prepared, out: &Path) -> Result<bool> {
    if p.audio.is_empty() {
        return Ok(false);
    }
    let mut cmd = ffmpeg();
    cmd.arg("-y");
    let mut filters = Vec::new();
    for (i, a) in p.audio.iter().enumerate() {
        if a.looped {
            cmd.args(["-stream_loop", "-1"]);
        }
        cmd.args(["-ss", &format!("{:.3}", a.trim), "-t", &format!("{:.3}", a.dur)]).arg("-i").arg(&a.path);
        let mut f = format!("[{i}:a]aresample=48000,aformat=channel_layouts=stereo,volume={:.3}", a.volume);
        if a.fade_in > 0.0 {
            f += &format!(",afade=t=in:st=0:d={:.3}", a.fade_in);
        }
        if a.fade_out > 0.0 {
            f += &format!(",afade=t=out:st={:.3}:d={:.3}", (a.dur - a.fade_out).max(0.0), a.fade_out);
        }
        let ms = (a.start * 1000.0).round() as i64;
        if ms > 0 {
            f += &format!(",adelay={ms}:all=1");
        }
        f += &format!("[a{i}]");
        filters.push(f);
    }
    let ins: String = (0..p.audio.len()).map(|i| format!("[a{i}]")).collect();
    filters.push(format!("{ins}amix=inputs={}:normalize=0:duration=longest,apad,atrim=0:{:.3}[out]", p.audio.len(), p.duration));
    cmd.args(["-filter_complex", &filters.join(";"), "-map", "[out]", "-c:a", "aac", "-b:a", "192k"]).arg(out);
    run(&mut cmd, "audio mix")?;
    Ok(true)
}
