//! Word-timed animated captions from a transcript.

use super::cues;
use crate::gfx::Sprite;
use crate::paint::{sprite_op, Clip, Op, Painter, Src};
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

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CapKind {
    Karaoke,
    Pop,
    Word,
    Line,
    Bounce,
}

impl CapKind {
    pub fn parse(s: Option<&str>) -> Result<CapKind> {
        Ok(match s.unwrap_or("karaoke") {
            "karaoke" | "highlight" => CapKind::Karaoke,
            "pop" | "tiktok" => CapKind::Pop,
            "word" | "single" => CapKind::Word,
            "line" | "plain" | "subtitle" => CapKind::Line,
            "bounce" | "jump" => CapKind::Bounce,
            o => bail!("unknown captions style '{o}' (karaoke, pop, word, line, bounce)"),
        })
    }
}

/// Shape of the box behind the active word (`highlightBg`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HiBox {
    /// rounded pill that glides from word to word
    Pill,
    /// highlighter-pen stroke that wipes in behind the word
    Marker,
}

impl HiBox {
    pub fn parse(s: Option<&str>) -> Result<HiBox> {
        Ok(match s.unwrap_or("pill") {
            "pill" | "box" => HiBox::Pill,
            "marker" | "highlighter" => HiBox::Marker,
            o => bail!("unknown highlightBox '{o}' (pill, marker)"),
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
    /// the words as shown (for subtitle export)
    text: String,
    cx: f32,
    cy: f32,
    words: Vec<CapWord>,
    /// background boxes (x0, y0, x1, y1) behind the whole chunk or each of its lines
    boxes: Vec<[f32; 4]>,
}

pub struct Captions {
    groups: Vec<CapGroup>,
    kind: CapKind,
    /// box behind the active word
    hi_bg: Option<(Rgba, HiBox)>,
    /// chunk background: colour + corner radius
    bg: Option<(Rgba, f32)>,
    active_scale: f32,
    /// logical font size and line height
    size: f32,
    line: f32,
    /// box width: single words wider than this are scaled down
    box_w: f32,
    /// seconds over which a chunk fades and scales in
    appear: f64,
}

pub struct CapOptions {
    pub kind: CapKind,
    /// text style; `style.bg` becomes the chunk background (box or per line) instead of a per-word box
    pub style: TextStyle,
    pub highlight: Rgba,
    pub highlight_bg: Option<Rgba>,
    pub highlight_box: HiBox,
    pub max_words: usize,
    /// lines per chunk (a chunk ends before it would wrap onto more lines)
    pub max_lines: usize,
    /// characters per chunk (default: 9 per word of `max_words`, at least 12)
    pub max_chars: Option<usize>,
    /// phrase-aware breaks (see `cues::Rules::phrase`)
    pub phrase: bool,
    /// lowercase words a chunk doesn't end on (with `phrase`)
    pub no_break_after: Vec<String>,
    /// chunks spoken faster than this (s) merge with a neighbour; also the shortest time on screen
    pub min_duration: Option<f64>,
    /// lead, hold and gap closing on screen
    pub timing: cues::Timing,
    /// seconds over which a chunk fades and scales in (0 = at once)
    pub appear: f64,
    pub uppercase: bool,
    /// active-word colours, cycled word by word (overrides `highlight`)
    pub colors: Vec<Rgba>,
    /// scale of the active word (default per style)
    pub active_scale: Option<f32>,
    /// base word colour per speaker label
    pub speaker_colors: HashMap<String, Rgba>,
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

fn color_key(c: Rgba) -> [u8; 4] {
    [c.r, c.g, c.b, c.a].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// `words` times are already layer-local (seconds since the layer starts).
pub fn build(words: &[Word], dur: f64, o: &CapOptions) -> Result<Captions> {
    let maxw = if o.kind == CapKind::Word { 1 } else { o.max_words.max(1) };
    let max_chars = o.max_chars.unwrap_or((maxw * 9).max(12)).max(1);
    let mut style = o.style.clone();
    let chunk_bg = style.bg.take();
    // outlines grow into the gap between words: give that space back
    style.word_spacing += style.stroke.map(|s| s.0).unwrap_or(0.0);
    // 1. group words into on-screen chunks, and time them
    let shown: Vec<Word> = words
        .iter()
        .filter(|w| w.end > 0.0 && w.start < dur)
        .map(|w| Word { text: if o.uppercase { w.text.to_uppercase() } else { w.text.clone() }, ..w.clone() })
        .collect();
    let rules = cues::Rules {
        max_words: maxw,
        max_chars,
        phrase: o.phrase,
        no_break_after: o.no_break_after.clone(),
        min_duration: o.min_duration,
        ..cues::Rules::default()
    };
    let max_lines = o.max_lines.max(1);
    let mut lines_ok = |ws: &[Word]| {
        let toks: Vec<String> = ws.iter().map(|x| x.text.clone()).collect();
        layout(&style, &toks).lines.len() <= max_lines
    };
    let chunks = rules.group(&shown, &mut lines_ok);
    let times = cues::Timing { min_duration: o.min_duration, ..o.timing }.apply(&chunks, dur);
    // 2. colours per word, then unique (text, colour) sprites rendered in parallel
    let mut gi = 0usize;
    let colored: Vec<Vec<(Rgba, Rgba)>> = chunks
        .iter()
        .map(|c| {
            c.iter()
                .map(|w| {
                    let normal = w.speaker.as_ref().and_then(|s| o.speaker_colors.get(s)).copied().unwrap_or(style.color);
                    let hi = if o.colors.is_empty() { o.highlight } else { o.colors[gi % o.colors.len()] };
                    gi += 1;
                    (normal, if o.kind == CapKind::Line { normal } else { hi })
                })
                .collect()
        })
        .collect();
    let mut uniq: Vec<(String, [u8; 4], Rgba)> = vec![];
    for (c, cols) in chunks.iter().zip(&colored) {
        for (w, (n, h)) in c.iter().zip(cols) {
            if o.kind != CapKind::Word {
                uniq.push((w.text.clone(), color_key(*n), *n));
            }
            uniq.push((w.text.clone(), color_key(*h), *h));
        }
    }
    uniq.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    uniq.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    let sprites: HashMap<(String, [u8; 4]), Arc<Sprite>> = uniq
        .par_iter()
        .map(|(t, k, c)| Ok(((t.clone(), *k), Arc::new(logical(render_word(&style, t, *c, None)?, o.q)))))
        .collect::<Result<_>>()?;
    let sprite = |t: &str, c: Rgba| sprites.get(&(t.to_string(), color_key(c))).cloned();
    let default_scale = match o.kind {
        CapKind::Pop => 1.12,
        _ => 1.0,
    };
    let active_scale = o.active_scale.unwrap_or(default_scale);
    // 3. layout each chunk centred in the layer box
    let mut groups = Vec::with_capacity(chunks.len());
    for (ci, (c, cols)) in chunks.iter().zip(&colored).enumerate() {
        let toks: Vec<String> = c.iter().map(|w| w.text.clone()).collect();
        let lay = layout(&style, &toks);
        let gx = (o.box_w - lay.width) / 2.0;
        let gy = (o.box_h - lay.height) / 2.0;
        let lh = style.line_px();
        let spread = spread_for_scale(&lay, active_scale, o.box_w);
        let centres = &spread.centres;
        let (start, end) = times[ci];
        let words = c
            .iter()
            .zip(&lay.words)
            .zip(cols)
            .enumerate()
            .map(|(i, ((w, pw), (nc, hc)))| {
                let hi = sprite(&w.text, *hc).unwrap();
                let wend = c.get(i + 1).map(|x| x.start).unwrap_or(end);
                CapWord {
                    start: w.start,
                    end: wend.max(w.end.min(end)),
                    cx: (gx + centres[i]) / o.q,
                    cy: (gy + pw.y + lh / 2.0) / o.q,
                    normal: sprite(&w.text, *nc).unwrap_or_else(|| hi.clone()),
                    hi,
                }
            })
            .collect();
        let boxes = match chunk_bg {
            Some((_, px, py, _, true)) => {
                // one box per line, ~1.2 em tall: with a line height above that, lines get a small gap
                let half = (style.size * 0.6 + py).min(lh / 2.0 + py);
                lay.lines
                    .iter()
                    .enumerate()
                    .map(|(li, &(lx, ly, lw))| {
                        // around the words where they are drawn: spread for the growing word
                        let (lx, lw) = spread.lines.get(li).copied().filter(|l| l.1 > 0.0).unwrap_or((lx, lw));
                        let cy = gy + ly + lh / 2.0;
                        [gx + lx - px, cy - half, gx + lx + lw + px, cy + half].map(|v| v / o.q)
                    })
                    .collect()
            }
            Some((_, px, py, _, false)) => {
                let x0 = spread.lines.iter().filter(|l| l.1 > 0.0).map(|l| l.0).fold(0.0, f32::min);
                let x1 = spread.lines.iter().map(|l| l.0 + l.1).fold(lay.width, f32::max);
                vec![[gx + x0 - px, gy - py, gx + x1 + px, gy + lay.height + py].map(|v| v / o.q)]
            }
            None => vec![],
        };
        groups.push(CapGroup { start, end, text: cues::text(c), cx: o.box_w / 2.0 / o.q, cy: o.box_h / 2.0 / o.q, words, boxes });
    }
    Ok(Captions {
        groups,
        kind: o.kind,
        hi_bg: o.highlight_bg.map(|c| (c, o.highlight_box)),
        bg: chunk_bg.map(|(c, _, _, r, _)| (c, r / o.q)),
        active_scale,
        size: style.size / o.q,
        line: style.line_px() / o.q,
        box_w: o.box_w / o.q,
        appear: o.appear.max(0.0),
    })
}

#[allow(clippy::too_many_arguments)]
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
    let mut t = ts.pre_translate(cx, cy).pre_scale(scale, scale).pre_translate(-s.w / 2.0, -s.h / 2.0);
    // drawn 1:1 (no scale or rotation): put the bitmap on whole pixels, so it stays crisp and
    // every backend samples it the same way
    if (t.sx - s.ppu).abs() < 1e-3 && (t.sy - s.ppu).abs() < 1e-3 && t.kx.abs() < 1e-6 && t.ky.abs() < 1e-6 {
        let (ox, oy) = (t.tx - s.ox, t.ty - s.oy);
        t.tx += ox.round() - ox;
        t.ty += oy.round() - oy;
    }
    p.draw(sprite_op(s, t, opacity, clip));
}

fn solid<'a>(color: Rgba, ts: Transform, shape: [f32; 4], radius: f32, opacity: f32, clip: Option<Clip>) -> Op<'a> {
    Op { src: Src::Solid(color), ts, shape, radius, uv: Transform::identity(), opacity, clip }
}

fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

/// Where a chunk's words go when the active word grows (x in the chunk's layout).
#[derive(Debug, PartialEq)]
struct Spread {
    /// centre of each word
    centres: Vec<f32>,
    /// (left, width) of each line, words included (their background boxes follow these)
    lines: Vec<(f32, f32)>,
}

/// Word centres with room for the active word to grow: an active word is scaled about its centre,
/// so neighbours get `(scale - 1) / 2` of the wider word's width extra space; each line is centred
/// again and kept within `max_w` (a line that would get wider gets less room). Without growth, the
/// layout's own positions.
fn spread_for_scale(lay: &crate::text::TextLayout, scale: f32, max_w: f32) -> Spread {
    let grow = (scale - 1.0).max(0.0);
    let mut out = Spread { centres: lay.words.iter().map(|w| w.x + w.w / 2.0).collect(), lines: vec![] };
    let mut i = 0;
    while i < lay.words.len() {
        let line = lay.words[i].line;
        let mut j = i;
        while j < lay.words.len() && lay.words[j].line == line {
            j += 1;
        }
        let ws = &lay.words[i..j];
        let (left, right) = (ws[0].x, ws[ws.len() - 1].x + ws[ws.len() - 1].w);
        // the room each gap gets, all of it scaled down as far as the line must stay within max_w
        let extra: Vec<f32> = ws.windows(2).map(|p| grow * p[0].w.max(p[1].w) / 2.0).collect();
        let total: f32 = extra.iter().sum();
        let k = if total > 0.0 { ((max_w - (right - left)) / total).clamp(0.0, 1.0) } else { 0.0 };
        // place the line's words left to right with the extra room, then centre it where it was
        let mut x = left;
        let mut xs = vec![];
        for (n, w) in ws.iter().enumerate() {
            if n > 0 {
                x += w.x - (ws[n - 1].x + ws[n - 1].w) + extra[n - 1] * k;
            }
            xs.push(x + w.w / 2.0);
            x += w.w;
        }
        let shift = (left + x) / 2.0 - (left + right) / 2.0;
        for (n, c) in xs.into_iter().enumerate() {
            out.centres[i + n] = c - shift;
        }
        while out.lines.len() < line {
            out.lines.push((0.0, 0.0)); // a line without words
        }
        out.lines.push((left - shift, x - left));
        i = j;
    }
    out
}

impl Captions {
    /// The chunks as subtitle cues: (start, end, text) in layer time, in order, never overlapping.
    pub fn cues(&self) -> Vec<(f64, f64, String)> {
        self.groups.iter().filter(|g| g.end > g.start).map(|g| (g.start, g.end, g.text.clone())).collect()
    }

    /// Pill rectangle (x0, y0, x1, y1) around a word.
    fn pill(&self, w: &CapWord) -> [f32; 4] {
        let (hw, hh) = (w.normal.w / 2.0 + self.size * 0.18, self.line / 2.0 + self.size * 0.02);
        [w.cx - hw, w.cy - hh, w.cx + hw, w.cy + hh]
    }

    /// Scale and vertical offset of a word at time `t` (per style).
    fn word_motion(&self, w: &CapWord, t: f64) -> (f32, f32, f32) {
        let active = t >= w.start && t < w.end;
        let a = self.active_scale as f64;
        match self.kind {
            CapKind::Pop => {
                let p = ((t - w.start) / 0.16).clamp(0.0, 1.0);
                let sc = if active { (0.7 + (a - 0.7) * Ease::BackOut.apply(p)).min(a) } else { 1.0 };
                (sc as f32, 0.0, (p * 3.0).min(1.0) as f32)
            }
            CapKind::Bounce => {
                let p = ((t - w.start) / 0.45).clamp(0.0, 1.0);
                let dy = -(1.0 - Ease::Bounce.apply(p)) * self.line as f64 * 0.8;
                let sc = if active { 1.0 + (a - 1.0) * Ease::Out.apply(((t - w.start) / 0.12).clamp(0.0, 1.0)) } else { 1.0 };
                (sc as f32, dy as f32, (p * 5.0).min(1.0) as f32)
            }
            CapKind::Karaoke if active => {
                (1.0 + (a - 1.0) as f32 * Ease::Out.apply(((t - w.start) / 0.1).clamp(0.0, 1.0)) as f32, 0.0, 1.0)
            }
            _ => (1.0, 0.0, 1.0),
        }
    }

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
        let appear = if self.appear > 0.0 { (since / self.appear).clamp(0.0, 1.0) } else { 1.0 };
        let gop = opacity * appear as f32;
        let gscale = (0.92 + 0.08 * Ease::BackOut.apply(appear)) as f32;
        let gts = ts.pre_translate(g.cx, g.cy).pre_scale(gscale, gscale).pre_translate(-g.cx, -g.cy);
        if let Some((c, r)) = self.bg {
            for b in &g.boxes {
                canvas.draw(solid(c, gts, *b, r, gop, mask));
            }
        }
        if self.kind == CapKind::Word {
            if let Some(w) = g.words.iter().find(|w| t >= w.start && t < w.end) {
                let p = ((t - w.start) / 0.14).clamp(0.0, 1.0);
                let fit = (self.box_w / w.hi.w.max(1.0)).min(1.0);
                let sc = (0.6 + 0.4 * Ease::BackOut.apply(p)) as f32 * fit;
                if let Some((c, _)) = self.hi_bg {
                    let [x0, y0, x1, y1] = self.pill(w);
                    let (hw, hh) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
                    let t2 = ts.pre_translate(g.cx, g.cy).pre_scale(sc, sc);
                    canvas.draw(solid(c, t2, [-hw, -hh, hw, hh], self.size * 0.22, opacity, mask));
                }
                draw_sprite_centered(canvas, &w.hi, ts, g.cx, g.cy, sc, opacity, mask);
            }
            return;
        }
        // box behind the active word
        if let (Some((c, shape)), true) = (self.hi_bg, self.kind != CapKind::Line) {
            if let Some(k) = g.words.iter().position(|w| t >= w.start && t < w.end) {
                let w = &g.words[k];
                let (sc, dy, wop) = self.word_motion(w, t);
                let into = (t - w.start) as f32;
                match shape {
                    HiBox::Pill => {
                        let mut r = self.pill(w);
                        // glide over from the previous word
                        if k > 0 {
                            let e = Ease::Out.apply((into / 0.12).clamp(0.0, 1.0) as f64) as f32;
                            let prev = self.pill(&g.words[k - 1]);
                            r = [0, 1, 2, 3].map(|j| lerp(prev[j], r[j], e));
                        }
                        let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
                        let t2 = gts.pre_translate(cx, cy + dy).pre_scale(sc, sc);
                        let (hw, hh) = ((r[2] - r[0]) / 2.0, (r[3] - r[1]) / 2.0);
                        canvas.draw(solid(c, t2, [-hw, -hh, hw, hh], self.size * 0.22, gop * wop, mask));
                    }
                    HiBox::Marker => {
                        let e = Ease::Out.apply((into / 0.18).clamp(0.0, 1.0) as f64) as f32;
                        let hw = w.normal.w / 2.0 + self.size * 0.12;
                        let (y0, y1) = (-self.size * 0.36, self.size * 0.48);
                        let t2 = gts.pre_translate(w.cx, w.cy + dy).pre_scale(sc, sc).pre_concat(Transform::from_rotate(-1.5));
                        canvas.draw(solid(c, t2, [-hw, y0, -hw + 2.0 * hw * e, y1], self.size * 0.08, gop * wop, mask));
                    }
                }
            }
        }
        for w in &g.words {
            let active = t >= w.start && t < w.end;
            let spoken = t >= w.start;
            let s = if active { &w.hi } else { &w.normal };
            match self.kind {
                CapKind::Line => draw_sprite_centered(canvas, &w.normal, gts, w.cx, w.cy, 1.0, gop, mask),
                CapKind::Karaoke => {
                    let (sc, _, _) = self.word_motion(w, t);
                    draw_sprite_centered(canvas, s, gts, w.cx, w.cy, sc, gop, mask);
                }
                CapKind::Pop | CapKind::Bounce => {
                    if !spoken {
                        continue;
                    }
                    let (sc, dy, wop) = self.word_motion(w, t);
                    draw_sprite_centered(canvas, s, gts, w.cx, w.cy + dy, sc, gop * wop, mask);
                }
                CapKind::Word => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Align, FontDb};

    #[test]
    fn spreads_srt_cues_over_words() {
        let mut v = vec![];
        spread("hallo daar wereld", 1.0, 4.0, &mut v);
        assert_eq!(v.len(), 3);
        assert!((v[0].start - 1.0).abs() < 1e-9 && (v[2].end - 4.0).abs() < 1e-9);
        assert!(v.windows(2).all(|w| w[0].end <= w[1].start + 1e-9));
    }

    #[test]
    fn parses_timestamps() {
        assert_eq!(parse_ts("00:01:02,500"), Some(62.5));
        assert_eq!(parse_ts("01:02.25"), Some(62.25));
        assert_eq!(parse_ts("x"), None);
    }

    #[test]
    fn reads_speakers_from_words_and_segments() {
        let dir = std::env::temp_dir().join("flits-unit-speakers");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.json");
        let v = serde_json::json!({"segments": [
            {"start": 0.0, "end": 1.0, "speaker": "SPEAKER_00", "words": [
                {"word": "hi", "start": 0.0, "end": 0.4},
                {"word": "there", "start": 0.5, "end": 0.9, "speaker": "SPEAKER_01"}]},
            {"start": 1.0, "end": 2.0, "speaker": "SPEAKER_01", "text": "no words"}
        ]});
        std::fs::write(&p, v.to_string()).unwrap();
        let w = load_transcript(&p).unwrap();
        let sp: Vec<_> = w.iter().map(|w| w.speaker.as_deref().unwrap_or("-")).collect();
        assert_eq!(sp, ["SPEAKER_00", "SPEAKER_01", "SPEAKER_01", "SPEAKER_01"]);
    }

    fn opts(kind: CapKind) -> CapOptions {
        CapOptions {
            kind,
            style: TextStyle {
                font: FontDb::new(Default::default()).get(None, None).unwrap(),
                size: 40.0,
                color: Rgba::WHITE,
                line_height: 1.2,
                letter_spacing: 0.0,
                word_spacing: 0.0,
                align: Align::Center,
                max_width: Some(300.0),
                stroke: None,
                balance: false,
                shadow: None,
                bg: Some((Rgba::BLACK, 10.0, 4.0, 6.0, true)),
            },
            highlight: Rgba::rgba8(255, 214, 10, 1.0),
            highlight_bg: None,
            highlight_box: HiBox::Pill,
            max_words: 20,
            max_lines: 2,
            max_chars: None,
            phrase: false,
            no_break_after: vec![],
            min_duration: None,
            timing: cues::Timing::default(),
            appear: 0.12,
            uppercase: false,
            colors: vec![Rgba::rgba8(255, 0, 0, 1.0), Rgba::rgba8(0, 255, 0, 1.0)],
            active_scale: None,
            speaker_colors: HashMap::from([("B".to_string(), Rgba::rgba8(0, 0, 255, 1.0))]),
            box_w: 300.0,
            box_h: 200.0,
            q: 1.0,
        }
    }

    fn words(n: usize, speaker_from: usize) -> Vec<Word> {
        (0..n)
            .map(|i| Word {
                text: format!("word{i}"),
                start: i as f64 * 0.3,
                end: i as f64 * 0.3 + 0.25,
                speaker: Some(if i >= speaker_from { "B" } else { "A" }.into()),
            })
            .collect()
    }

    #[test]
    fn growing_words_keep_their_space() {
        use crate::text::{PlacedWord, TextLayout};
        let w = |text: &str, line: usize, x: f32, w: f32| PlacedWord { text: text.into(), line, x, y: 0.0, w };
        // "het vertrouwen" on one line, a third word on the next; 20 px spaces
        let lay = TextLayout {
            words: vec![w("het", 0, 0.0, 60.0), w("vertrouwen", 0, 80.0, 200.0), w("x", 1, 100.0, 40.0)],
            lines: vec![],
            width: 280.0,
            height: 0.0,
        };
        // no growth: the layout's own centres
        let s = spread_for_scale(&lay, 1.0, 1000.0);
        assert_eq!(s.centres, vec![30.0, 180.0, 120.0]);
        assert_eq!(s.lines, vec![(0.0, 280.0), (100.0, 40.0)]);
        let s = spread_for_scale(&lay, 1.12, 1000.0);
        let c = &s.centres;
        // the gap grows by 12% of half the wider word (12 px), and the line stays centred
        let (left, right) = (c[0] + 30.0, c[1] - 100.0);
        assert!((right - left - 32.0).abs() < 1e-4, "gap {}", right - left);
        assert!(((c[0] - 30.0 + c[1] + 100.0) / 2.0 - 140.0).abs() < 1e-4);
        // a scaled "vertrouwen" (1.12 x 200 about its centre) stays clear of "het"
        assert!(c[1] - 112.0 > c[0] + 30.0);
        assert_eq!(c[2], 120.0);
        // the line (its background) spans the spread words: 12 px wider, centred
        assert!((s.lines[0].0 - -6.0).abs() < 1e-4 && (s.lines[0].1 - 292.0).abs() < 1e-4, "{:?}", s.lines);
        assert!((s.lines[0].0 - (c[0] - 30.0)).abs() < 1e-4 && (s.lines[0].0 + s.lines[0].1 - (c[1] + 100.0)).abs() < 1e-4);
        // a line already as wide as the box gets no room beyond it
        let s = spread_for_scale(&lay, 1.12, 286.0);
        assert!((s.lines[0].1 - 286.0).abs() < 1e-4, "{:?}", s.lines);
    }

    #[test]
    fn line_backgrounds_cover_the_spread_words_within_the_box() {
        // pop: the active word grows, so the words of a line are spread apart
        let caps = build(&words(30, 100), 20.0, &opts(CapKind::Pop)).unwrap();
        let pad = 10.0; // the background's horizontal padding (see `opts`)
        for g in &caps.groups {
            for w in &g.words {
                let (x0, x1) = (w.cx - w.normal.w / 2.0, w.cx + w.normal.w / 2.0);
                let b = g.boxes.iter().find(|b| w.cy >= b[1] && w.cy <= b[3]).expect("a box for each line");
                assert!(x0 >= b[0] + pad - 1e-3 && x1 <= b[2] - pad + 1e-3, "word {x0:.1}..{x1:.1} outside its line's box {b:?}");
                assert!(x0 >= -1e-3 && x1 <= caps.box_w + 1e-3, "word {x0:.1}..{x1:.1} past the layer (0..{})", caps.box_w);
            }
        }
    }

    #[test]
    fn chunks_respect_lines_and_speakers() {
        let caps = build(&words(30, 100), 20.0, &opts(CapKind::Karaoke)).unwrap();
        assert!(caps.groups.len() > 1, "30 words don't fit on two lines of 300 px");
        for g in &caps.groups {
            assert!(g.boxes.len() <= 2, "at most two lines per chunk");
        }
        let caps = build(&words(6, 3), 20.0, &opts(CapKind::Karaoke)).unwrap();
        assert_eq!(caps.groups.len(), 2, "a new speaker starts a new chunk");
        assert_eq!(caps.groups[0].words.len(), 3);
    }

    #[test]
    fn timing_options_lead_and_never_overlap() {
        let mut o = opts(CapKind::Line);
        o.max_words = 2;
        o.timing = cues::Timing { lead: 0.05, ..cues::Timing::default() };
        o.min_duration = Some(0.9);
        o.appear = 0.0;
        // wide enough for four words on two lines with any system's default font (Linux CI's is wider)
        o.style.max_width = Some(800.0);
        let caps = build(&words(12, 100), 20.0, &o).unwrap();
        let c = caps.cues();
        assert_eq!(c[0].0, 0.0, "the lead stops at the layer start: {c:?}");
        assert!((c[1].0 - (caps.groups[1].words[0].start - 0.05)).abs() < 1e-9, "{c:?}");
        for p in c.windows(2) {
            assert!(p[0].1 <= p[1].0, "overlap: {p:?}");
        }
        // 0.55 s per two words is under minDuration: chunks merged up to the two lines
        assert!(caps.groups.iter().all(|g| g.words.len() > 2 || g.words.last().unwrap().end - g.words[0].start >= 0.9));
        assert!(caps.groups.iter().any(|g| g.words.len() > 2), "{c:?}");
    }

    #[test]
    fn a_scene_exports_its_captions_with_static_boxes() {
        let dir = std::env::temp_dir().join(format!("flits-scene-cues-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let words: Vec<serde_json::Value> = "Dit is de eerste zin. En dan komt er nog een tweede zin met de rest."
            .split_whitespace()
            .enumerate()
            .map(|(i, w)| serde_json::json!({"word": w, "start": i as f64 * 0.3, "end": i as f64 * 0.3 + 0.32}))
            .collect();
        std::fs::write(dir.join("t.words.json"), serde_json::json!({"language": "nl", "words": words}).to_string()).unwrap();
        // one caption at a time, white on a coloured rounded box per line, balanced, no pop-in
        let scene = serde_json::json!({"width": 1080, "height": 1920, "duration": 6, "layers": [{
            "type": "captions", "src": "t.words.json", "start": 0.5, "style": "line", "size": 56, "lineHeight": 1.38,
            "width": 880, "wrap": "balance", "maxChars": 32, "language": "auto", "minDuration": 0.9,
            "lead": 0.05, "appear": 0, "shadow": {"color": "#00000000"},
            "background": {"color": "#f07d00", "padding": [22, 6], "radius": 14, "mode": "lines"}
        }]});
        let (s, warnings) = crate::scene::Scene::from_value(scene, dir.clone()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let p = crate::compose::Prepared::build(&s, 0.25).unwrap();
        let c = crate::compose::scene_cues(&p.nodes);
        assert!(c.len() >= 2, "{c:?}");
        assert!((c[0].0 - 0.5).abs() < 1e-9, "starts with the layer (the lead can't go before it): {c:?}");
        assert_eq!(c[0].2, "Dit is de eerste zin.");
        for p in c.windows(2) {
            assert!(p[0].1 <= p[1].0, "{p:?}");
        }
        for (_, _, t) in &c {
            let last = t.split_whitespace().last().unwrap();
            assert!(!cues::WEAK_NL.contains(&last), "{c:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn colours_cycle_and_follow_speakers() {
        let caps = build(&words(4, 2), 20.0, &opts(CapKind::Karaoke)).unwrap();
        let ws: Vec<&CapWord> = caps.groups.iter().flat_map(|g| &g.words).collect();
        let px = |s: &Sprite| {
            // the most saturated pixel tells the colour
            *s.pm.pixels().iter().max_by_key(|p| p.alpha() as i32 * 3 - p.red().min(p.green()).min(p.blue()) as i32).unwrap()
        };
        assert!(px(&ws[0].hi).red() > 200 && px(&ws[1].hi).green() > 200, "active colours cycle");
        assert!(px(&ws[3].normal).blue() > 200 && px(&ws[3].normal).red() < 50, "speaker B words are blue");
    }
}

#[cfg(test)]
mod transcript_tests {
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
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_timestamps() {
        assert_eq!(parse_ts("00:01:02,500"), Some(62.5));
        assert_eq!(parse_ts("01:02.25"), Some(62.25));
        assert_eq!(parse_ts("x"), None);
    }
}
