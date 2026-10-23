//! Word-timed animated captions from a transcript.

use crate::gfx::Sprite;
use crate::paint::{sprite_op, Clip, Painter};
use crate::scene::anim::Ease;
use crate::scene::color::Rgba;
use crate::text::{layout, render_word, TextStyle};
use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tiny_skia::Transform;

#[derive(Clone, Debug, Default)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
    /// speaker label from diarization (e.g. "SPEAKER_00"), if any
    pub speaker: Option<String>,
}

/// One transcript segment (a whisper segment or a subtitle cue) with its words.
#[derive(Clone, Debug, Default)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub speaker: Option<String>,
    pub words: Vec<Word>,
}

/// A transcript with its segment structure and (when known) the spoken language.
#[derive(Clone, Debug, Default)]
pub struct Transcript {
    pub language: Option<String>,
    pub segments: Vec<Segment>,
}

impl Transcript {
    /// All words in time order.
    pub fn words(&self) -> Vec<Word> {
        let mut words: Vec<Word> = self.segments.iter().flat_map(|s| s.words.iter().cloned()).collect();
        words.sort_by(|a, b| a.start.total_cmp(&b.start));
        words
    }
}

fn parse_ts(s: &str) -> Option<f64> {
    // 00:01:02,345 or 00:01:02.345 or 01:02.345
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let mut t = 0.0;
    for p in &parts {
        t = t * 60.0 + p.parse::<f64>().ok()?;
    }
    Some(t)
}

/// Load words from our/faster-whisper JSON, or from .srt/.vtt (word times interpolated).
pub fn load_transcript(path: &Path) -> Result<Vec<Word>> {
    Ok(load_transcript_full(path)?.words())
}

fn json_speaker(v: &Value) -> Option<String> {
    v["speaker"].as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Load a transcript with segments, speakers and language (JSON, .srt or .vtt).
pub fn load_transcript_full(path: &Path) -> Result<Transcript> {
    let txt = std::fs::read_to_string(path).with_context(|| format!("reading transcript {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut tr = Transcript::default();
    if ext == "json" {
        let v: Value = serde_json::from_str(&txt)?;
        tr.language = v["language"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
        let word = |w: &Value, seg_speaker: &Option<String>| -> Option<Word> {
            let text = w["word"].as_str().or(w["text"].as_str()).unwrap_or("").trim().to_string();
            match (w["start"].as_f64(), w["end"].as_f64()) {
                (Some(start), Some(end)) if !text.is_empty() => {
                    Some(Word { text, start, end, speaker: json_speaker(w).or_else(|| seg_speaker.clone()) })
                }
                _ => None,
            }
        };
        // a flat word list ({"words": [...]}, e.g. `flits transcribe --words-only`)
        if let Some(ws) = v["words"].as_array() {
            let words: Vec<Word> = ws.iter().filter_map(|w| word(w, &None)).collect();
            if let (Some(a), Some(b)) = (words.first(), words.last()) {
                let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                tr.segments.push(Segment { start: a.start, end: b.end, text, speaker: None, words });
            }
        }
        for s in v["segments"].as_array().map(Vec::as_slice).unwrap_or_default() {
            let speaker = json_speaker(s);
            let text = s["text"].as_str().unwrap_or("").trim().to_string();
            let mut words: Vec<Word> =
                s["words"].as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(|w| word(w, &speaker)).collect();
            let (a, b) = (s["start"].as_f64(), s["end"].as_f64());
            if words.is_empty() {
                if let (Some(a), Some(b)) = (a, b) {
                    spread(&text, a, b, &mut words);
                    words.iter_mut().for_each(|w| w.speaker = speaker.clone());
                }
            }
            let (Some(first), Some(last)) = (words.first(), words.last()) else { continue };
            let (start, end) = (a.unwrap_or(first.start), b.unwrap_or(last.end));
            tr.segments.push(Segment { start, end, text, speaker, words });
        }
    } else if ext == "srt" || ext == "vtt" {
        let mut lines = txt.lines().peekable();
        while let Some(l) = lines.next() {
            if let Some((a, b)) = l.split_once("-->") {
                let (Some(a), Some(b)) = (parse_ts(a), parse_ts(b.split_whitespace().next().unwrap_or(""))) else {
                    continue;
                };
                let mut text = String::new();
                while let Some(n) = lines.peek() {
                    if n.trim().is_empty() {
                        break;
                    }
                    text.push(' ');
                    text.push_str(n);
                    lines.next();
                }
                let mut words = vec![];
                spread(&text, a, b, &mut words);
                if !words.is_empty() {
                    tr.segments.push(Segment { start: a, end: b, text: text.trim().to_string(), speaker: None, words });
                }
            }
        }
    } else {
        bail!("unsupported transcript format '{}' (json, srt, vtt)", path.display());
    }
    tr.segments.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(tr)
}

fn spread(text: &str, a: f64, b: f64, out: &mut Vec<Word>) {
    let toks: Vec<&str> = text.split_whitespace().collect();
    let total: usize = toks.iter().map(|t| t.chars().count() + 1).sum();
    let mut t = a;
    for tok in toks {
        let d = (b - a) * (tok.chars().count() + 1) as f64 / total.max(1) as f64;
        out.push(Word { text: tok.to_string(), start: t, end: t + d, speaker: None });
        t += d;
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum CapKind {
    Karaoke,
    Pop,
    Word,
    Line,
}

impl CapKind {
    pub fn parse(s: Option<&str>) -> Result<CapKind> {
        Ok(match s.unwrap_or("karaoke") {
            "karaoke" | "highlight" => CapKind::Karaoke,
            "pop" | "tiktok" => CapKind::Pop,
            "word" | "single" => CapKind::Word,
            "line" | "plain" | "subtitle" => CapKind::Line,
            o => bail!("unknown captions style '{o}' (karaoke, pop, word, line)"),
        })
    }
}

struct CapWord {
    start: f64,
    end: f64,
    cx: f32,
    cy: f32,
    normal: Arc<Sprite>,
    hi: Arc<Sprite>,
}

struct CapGroup {
    start: f64,
    end: f64,
    cx: f32,
    cy: f32,
    words: Vec<CapWord>,
}

pub struct Captions {
    groups: Vec<CapGroup>,
    kind: CapKind,
}

pub struct CapOptions {
    pub kind: CapKind,
    pub style: TextStyle,
    pub highlight: Rgba,
    pub highlight_bg: Option<Rgba>,
    pub max_words: usize,
    pub uppercase: bool,
    pub box_w: f32,
    pub box_h: f32,
    /// oversampling: style/box are in pixels at q x the logical size
    pub q: f32,
    /// text colour per speaker (in order of first appearance); empty = `style.color` for everyone
    pub speaker_colors: Vec<Rgba>,
}

/// Index of every word's speaker in order of first appearance (None = no speaker label).
fn speaker_indices(words: &[Word]) -> Vec<Option<usize>> {
    let mut seen: Vec<&str> = vec![];
    words
        .iter()
        .map(|w| {
            let s = w.speaker.as_deref()?;
            Some(seen.iter().position(|x| *x == s).unwrap_or_else(|| {
                seen.push(s);
                seen.len() - 1
            }))
        })
        .collect()
}

fn logical(mut s: Sprite, q: f32) -> Sprite {
    s.w /= q;
    s.h /= q;
    s.ppu = q;
    s
}

/// `words` times are already layer-local (seconds since the layer starts).
pub fn build(words: &[Word], dur: f64, o: &CapOptions) -> Result<Captions> {
    let maxw = if o.kind == CapKind::Word { 1 } else { o.max_words.max(1) };
    let max_chars = (maxw * 9).max(12);
    // 1. group words into on-screen chunks
    let speakers = speaker_indices(words);
    // the normal text colour of a word: its speaker's colour when `speaker_colors` is set
    let color_of = |si: Option<usize>| match (si, o.speaker_colors.len()) {
        (Some(i), n) if n > 0 => o.speaker_colors[i % n],
        _ => o.style.color,
    };
    let mut chunks: Vec<Vec<(Word, Option<usize>)>> = vec![];
    for (w, si) in words.iter().zip(&speakers).filter(|(w, _)| w.end > 0.0 && w.start < dur) {
        let text = if o.uppercase { w.text.to_uppercase() } else { w.text.clone() };
        let w = Word { text, ..w.clone() };
        let new_chunk = match chunks.last() {
            None => true,
            Some(c) => {
                let (last, last_si) = c.last().unwrap();
                let chars: usize = c.iter().map(|x| x.0.text.chars().count() + 1).sum();
                c.len() >= maxw
                    || last_si != si
                    || w.start - last.end > 0.6
                    || chars + w.text.chars().count() > max_chars
                    || last.text.ends_with(['.', '?', '!', ':'])
                    || (last.text.ends_with(',') && c.len() > maxw / 2)
            }
        };
        if new_chunk {
            chunks.push(vec![(w, *si)]);
        } else {
            chunks.last_mut().unwrap().push((w, *si));
        }
    }
    // 2. unique word sprites (text + speaker colour), rendered in parallel
    let ckey = |si: Option<usize>| if o.speaker_colors.is_empty() { None } else { si.map(|i| i % o.speaker_colors.len()) };
    let mut uniq: Vec<(String, Option<usize>)> = chunks.iter().flatten().map(|(w, si)| (w.text.clone(), ckey(*si))).collect();
    uniq.sort();
    uniq.dedup();
    let pill = o.highlight_bg;
    let sprites: HashMap<(String, Option<usize>), (Arc<Sprite>, Arc<Sprite>)> = uniq
        .par_iter()
        .map(|(t, ci)| -> Result<((String, Option<usize>), (Arc<Sprite>, Arc<Sprite>))> {
            let color = color_of(*ci);
            let hi_color = if pill.is_some() && o.highlight == o.style.color { color } else { o.highlight };
            let n = Arc::new(logical(render_word(&o.style, t, color, None)?, o.q));
            let h = if o.kind == CapKind::Line { n.clone() } else { Arc::new(logical(render_word(&o.style, t, hi_color, pill)?, o.q)) };
            Ok(((t.clone(), *ci), (n, h)))
        })
        .collect::<Result<_>>()?;
    // 3. layout each chunk centred in the layer box
    let mut groups = Vec::with_capacity(chunks.len());
    for (ci, c) in chunks.iter().enumerate() {
        let toks: Vec<String> = c.iter().map(|(w, _)| w.text.clone()).collect();
        let lay = layout(&o.style, &toks);
        let gx = (o.box_w - lay.width) / 2.0;
        let gy = (o.box_h - lay.height) / 2.0;
        let lh = o.style.line_px();
        let next_start = chunks.get(ci + 1).map(|n| n[0].0.start).unwrap_or(f64::INFINITY);
        let start = c[0].0.start;
        // keep the chunk up until the next one starts if the gap is short
        let mut end = c.last().unwrap().0.end + 0.25;
        if next_start - end < 0.6 {
            end = next_start;
        }
        let end = end.min(next_start).min(dur);
        let words = c
            .iter()
            .zip(&lay.words)
            .enumerate()
            .map(|(i, ((w, si), pw))| {
                let (n, h) = sprites[&(w.text.clone(), ckey(*si))].clone();
                let wend = c.get(i + 1).map(|x| x.0.start).unwrap_or(end);
                CapWord {
                    start: w.start,
                    end: wend.max(w.end.min(end)),
                    cx: (gx + pw.x + pw.w / 2.0) / o.q,
                    cy: (gy + pw.y + lh / 2.0) / o.q,
                    normal: n,
                    hi: h,
                }
            })
            .collect();
        groups.push(CapGroup { start, end, cx: o.box_w / 2.0 / o.q, cy: o.box_h / 2.0 / o.q, words });
    }
    Ok(Captions { groups, kind: o.kind })
}

fn draw_sprite_centered<'a>(
    p: &mut dyn Painter<'a>,
    s: &'a Sprite,
    ts: Transform,
    cx: f32,
    cy: f32,
    scale: f32,
    opacity: f32,
    clip: Option<Clip>,
) {
    let t = ts.pre_translate(cx, cy).pre_scale(scale, scale).pre_translate(-s.w / 2.0, -s.h / 2.0);
    p.draw(sprite_op(s, t, opacity, clip));
}

impl Captions {
    pub fn draw<'a>(&'a self, canvas: &mut dyn Painter<'a>, t: f64, ts: Transform, opacity: f32, mask: Option<Clip>) {
        let i = self.groups.partition_point(|g| g.start <= t);
        if i == 0 {
            return;
        }
        let g = &self.groups[i - 1];
        if t >= g.end {
            return;
        }
        let since = t - g.start;
        let appear = (since / 0.12).clamp(0.0, 1.0);
        let gop = opacity * appear as f32;
        let gscale = (0.92 + 0.08 * Ease::BackOut.apply(appear)) as f32;
        let gts = ts.pre_translate(g.cx, g.cy).pre_scale(gscale, gscale).pre_translate(-g.cx, -g.cy);
        for w in &g.words {
            let active = t >= w.start && t < w.end;
            let spoken = t >= w.start;
            match self.kind {
                CapKind::Line => draw_sprite_centered(canvas, &w.normal, gts, w.cx, w.cy, 1.0, gop, mask),
                CapKind::Karaoke => {
                    let s = if active { &w.hi } else { &w.normal };
                    draw_sprite_centered(canvas, s, gts, w.cx, w.cy, 1.0, gop, mask);
                }
                CapKind::Pop => {
                    if !spoken {
                        continue;
                    }
                    let p = ((t - w.start) / 0.16).clamp(0.0, 1.0);
                    let sc = if active { 0.7 + 0.42 * Ease::BackOut.apply(p) } else { 1.0 };
                    let s = if active { &w.hi } else { &w.normal };
                    draw_sprite_centered(canvas, s, gts, w.cx, w.cy, sc.min(1.12) as f32, gop * (p * 3.0).min(1.0) as f32, mask);
                }
                CapKind::Word => {
                    if !active {
                        continue;
                    }
                    let p = ((t - w.start) / 0.14).clamp(0.0, 1.0);
                    let sc = 0.6 + 0.4 * Ease::BackOut.apply(p);
                    draw_sprite_centered(canvas, &w.hi, ts, g.cx, g.cy, sc as f32, opacity, mask);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreads_srt_cues_over_words() {
        let mut v = vec![];
        spread("hallo daar wereld", 1.0, 4.0, &mut v);
        assert_eq!(v.len(), 3);
        assert!((v[0].start - 1.0).abs() < 1e-9 && (v[2].end - 4.0).abs() < 1e-9);
        assert!(v.windows(2).all(|w| w[0].end <= w[1].start + 1e-9));
    }

    #[test]
    fn loads_speakers_language_and_segments() {
        let dir = std::env::temp_dir().join(format!("flits-captions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.json");
        let v = serde_json::json!({"language": "nl", "segments": [
            {"start": 0.0, "end": 1.0, "text": "hoi daar", "speaker": "SPEAKER_00",
             "words": [{"word": "hoi", "start": 0.0, "end": 0.4}, {"word": "daar", "start": 0.5, "end": 1.0}]},
            {"start": 1.5, "end": 2.5, "text": "dag allemaal", "speaker": "SPEAKER_01"}
        ]});
        std::fs::write(&p, v.to_string()).unwrap();
        let tr = load_transcript_full(&p).unwrap();
        assert_eq!(tr.language.as_deref(), Some("nl"));
        assert_eq!(tr.segments.len(), 2);
        let words = tr.words();
        assert_eq!(words.len(), 4);
        // word speakers inherit the segment's; a segment without words is spread over its text
        assert_eq!(words[1].speaker.as_deref(), Some("SPEAKER_00"));
        assert_eq!(words[3].speaker.as_deref(), Some("SPEAKER_01"));
        assert_eq!(speaker_indices(&words), vec![Some(0), Some(0), Some(1), Some(1)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_timestamps() {
        assert_eq!(parse_ts("00:01:02,500"), Some(62.5));
        assert_eq!(parse_ts("01:02.25"), Some(62.25));
        assert_eq!(parse_ts("x"), None);
    }
}
