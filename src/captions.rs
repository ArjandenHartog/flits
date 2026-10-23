//! Word-timed animated captions from a transcript.

use crate::anim::Ease;
use crate::color::Rgba;
use crate::effects::Sprite;
use crate::text::{layout, render_word, TextStyle};
use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use crate::paint::{sprite_op, Clip, Painter};
use tiny_skia::Transform;

#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
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
    let txt = std::fs::read_to_string(path).with_context(|| format!("reading transcript {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut words = Vec::new();
    if ext == "json" {
        let v: Value = serde_json::from_str(&txt)?;
        let segs = v["segments"].as_array().cloned().unwrap_or_default();
        let direct = v["words"].as_array().cloned();
        let push = |w: &Value, words: &mut Vec<Word>| {
            let text = w["word"].as_str().or(w["text"].as_str()).unwrap_or("").trim().to_string();
            if let (Some(s), Some(e)) = (w["start"].as_f64(), w["end"].as_f64()) {
                if !text.is_empty() {
                    words.push(Word { text, start: s, end: e });
                }
            }
        };
        if let Some(ws) = direct {
            for w in &ws {
                push(w, &mut words);
            }
        }
        for s in &segs {
            match s["words"].as_array() {
                Some(ws) if !ws.is_empty() => {
                    for w in ws {
                        push(w, &mut words);
                    }
                }
                _ => {
                    if let (Some(a), Some(b), Some(t)) = (s["start"].as_f64(), s["end"].as_f64(), s["text"].as_str()) {
                        spread(t, a, b, &mut words);
                    }
                }
            }
        }
    } else if ext == "srt" || ext == "vtt" {
        let mut lines = txt.lines().peekable();
        while let Some(l) = lines.next() {
            if let Some((a, b)) = l.split_once("-->") {
                let (Some(a), Some(b)) = (parse_ts(a), parse_ts(b.split_whitespace().next().unwrap_or(""))) else { continue };
                let mut text = String::new();
                while let Some(n) = lines.peek() {
                    if n.trim().is_empty() {
                        break;
                    }
                    text.push(' ');
                    text.push_str(n);
                    lines.next();
                }
                spread(&text, a, b, &mut words);
            }
        }
    } else {
        bail!("unsupported transcript format '{}' (json, srt, vtt)", path.display());
    }
    words.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    Ok(words)
}

fn spread(text: &str, a: f64, b: f64, out: &mut Vec<Word>) {
    let toks: Vec<&str> = text.split_whitespace().collect();
    let total: usize = toks.iter().map(|t| t.chars().count() + 1).sum();
    let mut t = a;
    for tok in toks {
        let d = (b - a) * (tok.chars().count() + 1) as f64 / total.max(1) as f64;
        out.push(Word { text: tok.to_string(), start: t, end: t + d });
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
    let mut chunks: Vec<Vec<Word>> = vec![];
    for w in words.iter().filter(|w| w.end > 0.0 && w.start < dur) {
        let text = if o.uppercase { w.text.to_uppercase() } else { w.text.clone() };
        let w = Word { text, ..w.clone() };
        let new_chunk = match chunks.last() {
            None => true,
            Some(c) => {
                let last = c.last().unwrap();
                let chars: usize = c.iter().map(|x| x.text.chars().count() + 1).sum();
                c.len() >= maxw
                    || w.start - last.end > 0.6
                    || chars + w.text.chars().count() > max_chars
                    || last.text.ends_with(['.', '?', '!', ':'])
                    || (last.text.ends_with(',') && c.len() >= maxw / 2 + 1)
            }
        };
        if new_chunk {
            chunks.push(vec![w]);
        } else {
            chunks.last_mut().unwrap().push(w);
        }
    }
    // 2. unique word sprites, rendered in parallel
    let mut uniq: Vec<String> = chunks.iter().flatten().map(|w| w.text.clone()).collect();
    uniq.sort();
    uniq.dedup();
    let pill = o.highlight_bg;
    let hi_color = if pill.is_some() && o.highlight == o.style.color { o.style.color } else { o.highlight };
    let sprites: HashMap<String, (Arc<Sprite>, Arc<Sprite>)> = uniq
        .par_iter()
        .map(|t| -> Result<(String, (Arc<Sprite>, Arc<Sprite>))> {
            let n = Arc::new(logical(render_word(&o.style, t, o.style.color, None)?, o.q));
            let h = if o.kind == CapKind::Line { n.clone() } else { Arc::new(logical(render_word(&o.style, t, hi_color, pill)?, o.q)) };
            Ok((t.clone(), (n, h)))
        })
        .collect::<Result<_>>()?;
    // 3. layout each chunk centred in the layer box
    let mut groups = Vec::with_capacity(chunks.len());
    for (ci, c) in chunks.iter().enumerate() {
        let toks: Vec<String> = c.iter().map(|w| w.text.clone()).collect();
        let lay = layout(&o.style, &toks);
        let gx = (o.box_w - lay.width) / 2.0;
        let gy = (o.box_h - lay.height) / 2.0;
        let lh = o.style.line_px();
        let next_start = chunks.get(ci + 1).map(|n| n[0].start).unwrap_or(f64::INFINITY);
        let start = c[0].start;
        // keep the chunk up until the next one starts if the gap is short
        let mut end = c.last().unwrap().end + 0.25;
        if next_start - end < 0.6 {
            end = next_start;
        }
        let end = end.min(next_start).min(dur);
        let words = c
            .iter()
            .zip(&lay.words)
            .enumerate()
            .map(|(i, (w, pw))| {
                let (n, h) = sprites[&w.text].clone();
                let wend = c.get(i + 1).map(|x| x.start).unwrap_or(end);
                CapWord { start: w.start, end: wend.max(w.end.min(end)), cx: (gx + pw.x + pw.w / 2.0) / o.q, cy: (gy + pw.y + lh / 2.0) / o.q, normal: n, hi: h }
            })
            .collect();
        groups.push(CapGroup { start, end, cx: o.box_w / 2.0 / o.q, cy: o.box_h / 2.0 / o.q, words });
    }
    Ok(Captions { groups, kind: o.kind })
}

fn draw_sprite_centered<'a>(p: &mut dyn Painter<'a>, s: &'a Sprite, ts: Transform, cx: f32, cy: f32, scale: f32, opacity: f32, clip: Option<Clip>) {
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
