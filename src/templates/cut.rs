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
    /// shorten silences (false: only words are cut)
    pub silences: bool,
    /// words/phrases to remove (from a transcript); empty = none
    pub fillers: Vec<String>,
    /// remove repeated words and stutters ("I I I think")
    pub repeats: bool,
}

pub use super::fillers::DEFAULT_FILLERS;

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

/// Time ranges for removed words (`words` sorted by start). Every run of consecutive removed
/// words goes from the middle of the pause before it to the middle of the pause after it, so
/// the pause that remains sounds natural and no breath of a removed word is left behind. A run
/// at the very start (end) reaches back to 0 (on to `total`).
pub fn word_removals(words: &[Word], removed: &[bool], total: f64) -> Vec<(f64, f64)> {
    let mut out = vec![];
    let mut i = 0;
    while i < words.len() {
        if !removed.get(i).copied().unwrap_or(false) {
            i += 1;
            continue;
        }
        let first = i;
        while i < words.len() && removed.get(i).copied().unwrap_or(false) {
            i += 1;
        }
        let last = i - 1;
        let a = if first == 0 { 0.0 } else { (words[first - 1].end + words[first].start) / 2.0 };
        let b = if i == words.len() { total } else { (words[last].end + words[i].start) / 2.0 };
        if b > a {
            out.push((a.max(0.0), b.min(total)));
        }
    }
    out
}

/// Words removed by the filler/repeat options, merged with `preset` (e.g. from an edit file).
pub fn removed_words(words: &[Word], o: &CutOpts, preset: &[bool]) -> Vec<bool> {
    let mut removed: Vec<bool> = (0..words.len()).map(|i| preset.get(i).copied().unwrap_or(false)).collect();
    let mut mark = |idx: Vec<usize>| idx.into_iter().for_each(|i| removed[i] = true);
    if !o.fillers.is_empty() {
        mark(super::fillers::find_fillers(words, &o.fillers));
    }
    if o.repeats {
        mark(super::fillers::find_repeats(words));
    }
    removed
}

/// Kept ranges from detected silences (shortened to `o.keep`) and removed words.
pub fn plan_ranges(total: f64, silences: &[(f64, f64)], o: &CutOpts, words: &[Word], removed: &[bool]) -> Vec<(f64, f64)> {
    let mut remove: Vec<(f64, f64)> = if o.silences {
        silences.iter().map(|&(s, e)| (s + o.keep / 2.0, e.min(total) - o.keep / 2.0)).filter(|(s, e)| e > s).collect()
    } else {
        vec![]
    };
    remove.extend(word_removals(words, &removed_words(words, o, removed), total));
    keep_ranges(total, remove)
}

/// Everything that should go: shortened silences, filler words, repeats and the words in
/// `removed` (one flag per word; may be empty).
pub fn plan(input: &Path, total: f64, o: &CutOpts, words: &[Word], removed: &[bool]) -> Result<Vec<(f64, f64)>> {
    let sil = if o.silences { silences(input, o.threshold_db, o.min_silence)? } else { vec![] };
    Ok(plan_ranges(total, &sil, o, words, removed))
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

    fn w(text: &str, start: f64, end: f64) -> Word {
        Word { text: text.into(), start, end, speaker: None }
    }

    fn opts() -> CutOpts {
        CutOpts { threshold_db: -35.0, min_silence: 0.6, keep: 0.2, silences: true, fillers: vec![], repeats: false }
    }

    #[test]
    fn removed_words_cut_from_pause_middle_to_pause_middle() {
        let words = vec![w("so", 0.5, 0.8), w("uhm", 1.0, 1.4), w("we", 1.6, 1.8), w("go", 2.0, 2.4)];
        // "uhm": from the middle of 0.8..1.0 to the middle of 1.4..1.6
        let r = word_removals(&words, &[false, true, false, false], 5.0);
        assert_eq!(r.len(), 1);
        assert!((r[0].0 - 0.9).abs() < 1e-9 && (r[0].1 - 1.5).abs() < 1e-9);
        // a run at the start reaches back to 0, a run at the end on to the total
        let r = word_removals(&words, &[true, false, false, true], 5.0);
        assert_eq!(r, vec![(0.0, (0.8 + 1.0) / 2.0), ((1.8 + 2.0) / 2.0, 5.0)]);
    }

    #[test]
    fn plan_combines_silences_fillers_and_repeats() {
        let words = vec![w("I", 0.0, 0.2), w("I", 0.3, 0.5), w("think", 0.6, 1.0), w("uh", 1.2, 1.5), w("yes", 4.0, 4.5)];
        let mut o = opts();
        o.fillers = super::super::fillers::for_language(Some("en"));
        o.repeats = true;
        // silence between "uh" and "yes"
        let k = plan_ranges(6.0, &[(1.5, 4.0)], &o, &words, &[]);
        // the first "I" goes (from 0 to the middle of 0.2..0.3); "uh" goes from 1.1 to the middle of 1.5..4.0
        assert!((k[0].0 - 0.25).abs() < 1e-9 && (k[0].1 - 1.1).abs() < 1e-9, "{k:?}");
        // after the silence: 0.1 s of pause is left before "yes"
        assert!((k[1].0 - 3.9).abs() < 1e-9 && (k[1].1 - 6.0).abs() < 1e-9, "{k:?}");
        assert_eq!(k.len(), 2);
        // without silence tightening, only the words go
        o.silences = false;
        o.fillers.clear();
        o.repeats = false;
        let k = plan_ranges(6.0, &[(1.5, 4.0)], &o, &words, &[false, false, false, false, true]);
        assert_eq!(k, vec![(0.0, (1.5 + 4.0) / 2.0)]);
    }
}
