//! `flits cut`: remove silences (and optionally filler words) from a recording.
//!
//! Audio is cut in one ffmpeg pass (atrim + concat with tiny fades so cuts don't
//! click). For video, the kept ranges become a sequence of video layers rendered by
//! the normal pipeline, with the cut audio as soundtrack.

use super::abs;
use crate::compose::captions::Word;
use crate::media::{self, ffmpeg};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

pub struct CutOpts {
    /// silence threshold in dB (e.g. -35)
    pub threshold_db: f64,
    /// only silences at least this long are shortened
    pub min_silence: f64,
    /// pause left in place of a removed silence
    pub keep: f64,
    /// words to remove (from a transcript); empty = none
    pub fillers: Vec<String>,
}

pub const DEFAULT_FILLERS: &[&str] = &["uh", "uhm", "um", "umm", "eh", "ehm", "euh", "uhh", "hm", "hmm", "mm", "ah", "eeh"];

/// Silent ranges reported by ffmpeg's silencedetect.
pub fn silences(input: &Path, threshold_db: f64, min: f64) -> Result<Vec<(f64, f64)>> {
    let out = ffmpeg()
        .arg("-i")
        .arg(input)
        .args(["-vn", "-af", &format!("silencedetect=n={threshold_db}dB:d={min}"), "-f", "null", "-"])
        .args(["-loglevel", "info"])
        .output()
        .context("running ffmpeg silencedetect")?;
    let log = String::from_utf8_lossy(&out.stderr);
    let mut res = vec![];
    let mut start = None;
    for line in log.lines() {
        if let Some(i) = line.find("silence_start: ") {
            start = line[i + 15..].split_whitespace().next().and_then(|v| v.parse::<f64>().ok());
        } else if let Some(i) = line.find("silence_end: ") {
            if let (Some(s), Some(e)) = (start.take(), line[i + 13..].split_whitespace().next().and_then(|v| v.parse::<f64>().ok())) {
                res.push((s.max(0.0), e));
            }
        }
    }
    if let Some(s) = start {
        res.push((s, f64::INFINITY));
    }
    Ok(res)
}

fn norm_word(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

/// Ranges to keep, given the removable ranges (sorted, merged) and the total length.
pub fn keep_ranges(total: f64, mut remove: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    remove.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut merged: Vec<(f64, f64)> = vec![];
    for (s, e) in remove {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    let mut keep = vec![];
    let mut t = 0.0;
    for (s, e) in merged {
        if s > t + 0.05 {
            keep.push((t, s.min(total)));
        }
        t = t.max(e);
    }
    if total > t + 0.05 {
        keep.push((t, total));
    }
    // fragments shorter than this are clicks or breaths, not speech
    keep.retain(|(s, e)| e - s >= 0.2);
    keep
}

/// Everything that should go: shortened silences plus filler words.
pub fn plan(input: &Path, total: f64, o: &CutOpts, words: &[Word]) -> Result<Vec<(f64, f64)>> {
    let mut remove: Vec<(f64, f64)> = silences(input, o.threshold_db, o.min_silence)?
        .into_iter()
        .map(|(s, e)| (s + o.keep / 2.0, e.min(total) - o.keep / 2.0))
        .filter(|(s, e)| e > s)
        .collect();
    if !o.fillers.is_empty() {
        for w in words {
            if o.fillers.contains(&norm_word(&w.text)) {
                remove.push(((w.start - 0.04).max(0.0), w.end + 0.04));
            }
        }
    }
    Ok(keep_ranges(total, remove))
}

fn codec_args(out: &Path) -> &'static [&'static str] {
    match out.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "mp3" => &["-c:a", "libmp3lame", "-q:a", "2", "-compression_level", "7"],
        "wav" => &["-c:a", "pcm_s16le"],
        "flac" => &["-c:a", "flac"],
        "opus" | "ogg" => &["-c:a", "libopus", "-b:a", "96k"],
        _ => &["-c:a", "aac", "-aac_coder", "fast", "-b:a", "192k"],
    }
}

/// Write the kept ranges of `input`'s audio to `out` (format from the extension).
/// Streams: one ffmpeg decodes to PCM, Rust drops the removed parts with 8 ms fades at
/// every edit, another ffmpeg encodes. Memory use is constant, speed ~ decode + encode.
pub fn cut_audio(input: &Path, keep: &[(f64, f64)], out: &Path) -> Result<()> {
    let info = media::probe(input)?;
    use std::io::{Read, Write};
    use std::process::Stdio;
    if keep.is_empty() {
        bail!("nothing left after cutting");
    }
    // native rate and (up to stereo) channels, 16-bit: the least data through the pipes
    let rate = info.audio_rate.max(8000) as f64;
    let ch = info.audio_channels.clamp(1, 2) as usize;
    let (rate_s, ch_s) = ((rate as u32).to_string(), ch.to_string());
    let mut dec = ffmpeg()
        .arg("-i")
        .arg(input)
        .args(["-vn", "-ac", &ch_s, "-ar", &rate_s, "-f", "s16le", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("starting decoder")?;
    let mut enc = ffmpeg()
        .args(["-y", "-f", "s16le", "-ac", &ch_s, "-ar", &rate_s, "-i", "-"])
        .args(codec_args(out))
        .arg(out)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting encoder")?;
    let ranges: Vec<(u64, u64)> = keep.iter().map(|(s, e)| ((s * rate) as u64, (e * rate) as u64)).collect();
    let fade = (0.008 * rate) as u64;
    let mut src = std::io::BufReader::with_capacity(1 << 20, dec.stdout.take().unwrap());
    let mut dst = std::io::BufWriter::with_capacity(1 << 20, enc.stdin.take().unwrap());
    const BLOCK: usize = 1 << 15; // frames per block
    let mut block = vec![0i16; BLOCK * ch];
    let mut pos: u64 = 0; // source frame index of block[0]
    let mut r = 0;
    loop {
        // fill a block (the last one may be short)
        let bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut block);
        let mut got = 0;
        while got < bytes.len() {
            match src.read(&mut bytes[got..])? {
                0 => break,
                n => got += n,
            }
        }
        let frames = got / (2 * ch);
        if frames == 0 {
            break;
        }
        let end = pos + frames as u64;
        // write every kept range overlapping this block; only the 8 ms edges are touched
        while r < ranges.len() && ranges[r].0 < end {
            let (a, b) = ranges[r];
            let (s0, s1) = (a.max(pos), b.min(end));
            if s1 > s0 {
                let gain = |p: u64| (p - a).min(b - 1 - p) as f32 / fade as f32;
                for p in (s0..s1).filter(|&p| p < a + fade || p + fade >= b) {
                    let g = gain(p).min(1.0);
                    let o = (p - pos) as usize * ch;
                    block[o..o + ch].iter_mut().for_each(|v| *v = (*v as f32 * g) as i16);
                }
                let seg = &block[(s0 - pos) as usize * ch..(s1 - pos) as usize * ch];
                dst.write_all(bytemuck::cast_slice(seg))?;
            }
            if b > end {
                break;
            }
            r += 1;
        }
        pos = end;
        if r >= ranges.len() || frames < BLOCK {
            break;
        }
    }
    drop(dst);
    let _ = dec.kill();
    let _ = dec.wait();
    let o = enc.wait_with_output()?;
    if !o.status.success() {
        bail!("encoding failed: {}", String::from_utf8_lossy(&o.stderr));
    }
    Ok(())
}

/// Scene that plays the kept ranges of a video back to back, with `audio` as soundtrack.
pub fn cut_video_scene(input: &Path, keep: &[(f64, f64)], audio: &Path) -> Result<Value> {
    let info = media::probe(input)?;
    let (mut w, mut h) = (info.width as f64, info.height as f64);
    let long = w.max(h);
    if long > 3840.0 {
        w *= 3840.0 / long;
        h *= 3840.0 / long;
    }
    let even = |v: f64| ((v / 2.0).round() * 2.0) as u32;
    let fps = if info.fps > 1.0 && info.fps <= 120.0 { (info.fps * 1000.0).round() / 1000.0 } else { 30.0 };
    let src = abs(input);
    let kids: Vec<Value> =
        keep.iter().map(|(s, e)| json!({"type": "video", "src": src, "trim": s, "duration": e - s, "volume": 0})).collect();
    let total: f64 = keep.iter().map(|(s, e)| e - s).sum();
    Ok(json!({
        "width": even(w), "height": even(h), "fps": fps, "duration": total,
        "audio": [{"src": abs(audio)}],
        "layers": [{"type": "group", "sequence": true, "children": kids}]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_gaps_between_removals() {
        let k = keep_ranges(10.0, vec![(2.0, 3.0), (2.5, 4.0), (8.0, 12.0)]);
        assert_eq!(k, vec![(0.0, 2.0), (4.0, 8.0)]);
        assert_eq!(keep_ranges(5.0, vec![]), vec![(0.0, 5.0)]);
    }

    #[test]
    fn normalizes_filler_words() {
        assert_eq!(norm_word(" Uhm,"), "uhm");
    }
}
