//! Text layout and rasterization (glyph outlines -> tiny-skia paths).

pub mod color;
pub mod fonts;

pub use fonts::{parse_variations, Axis, Face, FaceRef, FontDb, FontRef, DEFAULT_FONT};

use crate::gfx::{rounded_rect, with_shadow, Sprite};
use crate::scene::color::Rgba;
use ab_glyph::{Font, GlyphId, OutlineCurve};
use anyhow::{anyhow, bail, Result};
use std::sync::Arc;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    pub fn parse(s: Option<&str>) -> Result<Align> {
        Ok(match s.unwrap_or("center") {
            "left" | "start" => Align::Left,
            "center" | "middle" => Align::Center,
            "right" | "end" => Align::Right,
            o => bail!("unknown align '{o}' (left, center, right)"),
        })
    }
}

#[derive(Clone)]
pub struct TextStyle {
    pub font: FontRef,
    pub size: f32,
    pub color: Rgba,
    pub line_height: f32,
    pub letter_spacing: f32,
    /// extra space between words (captions widen it so outlines don't touch)
    pub word_spacing: f32,
    pub align: Align,
    pub max_width: Option<f32>,
    pub stroke: Option<(f32, Rgba)>,
    /// even out line lengths when wrapping
    pub balance: bool,
    pub shadow: Option<(Rgba, f32, f32, f32)>,
    /// (color, pad x, pad y, radius, per-line)
    pub bg: Option<(Rgba, f32, f32, f32, bool)>,
}

impl TextStyle {
    fn k(&self) -> f32 {
        self.size / self.font.units_per_em().unwrap_or(1000.0)
    }

    pub fn measure(&self, word: &str) -> f32 {
        if !self.needs_fallback(word) {
            // the common case, without allocating: every character is in the layer's font
            let k = self.k();
            let mut x = 0.0;
            let mut prev = None;
            let mut n = 0;
            for ch in word.chars().filter(|c| *c != SHY) {
                let id = self.font.glyph_id(ch);
                if let Some(p) = prev {
                    x += self.font.kern_unscaled(p, id) * k;
                }
                x += self.font.h_advance_unscaled(id) * k;
                prev = Some(id);
                n += 1;
            }
            return x + self.letter_spacing * (n.max(1) - 1) as f32;
        }
        let (glyphs, width) = self.glyphs(word);
        let _ = glyphs;
        width
    }

    /// Whether `word` has characters the layer's font lacks, or emoji.
    fn needs_fallback(&self, word: &str) -> bool {
        let mut chars = word.chars().filter(|c| *c != SHY).peekable();
        while let Some(c) = chars.next() {
            if is_emoji(c, chars.peek().copied()) || needs_shaping(c) || (!self.font.face.covers(c) && !is_invisible(c)) {
                return true;
            }
        }
        false
    }

    /// The glyphs of one word, positioned in px from the word's origin on the baseline, and its
    /// advance width. Runs in the layer's font use its kerning; runs in fallback fonts (emoji,
    /// other scripts) are shaped, so emoji sequences (ZWJ, flags, skin tones, keycaps) join up.
    fn glyphs(&self, word: &str) -> (Vec<Glyph>, f32) {
        let chars: Vec<char> = word.chars().filter(|c| *c != SHY).collect();
        let fast = !self.needs_fallback(word);
        // face per character; marks, joiners and modifiers stay with the character before them
        let mut faces: Vec<(FaceRef, bool)> = Vec::with_capacity(chars.len());
        for (i, &c) in chars.iter().enumerate() {
            let emoji = !fast && is_emoji(c, chars.get(i + 1).copied());
            let joins = i > 0 && (is_joiner(c) || (is_regional(c) && is_regional(chars[i - 1]) && pairs_open(&chars[..i])));
            let face = if fast {
                (self.font.face.clone(), false)
            } else if joins || (i > 0 && is_invisible(c)) {
                faces[i - 1].clone()
            } else {
                (self.font.face_for(c, emoji), emoji)
            };
            faces.push(face);
        }
        let mut out = Vec::with_capacity(chars.len());
        let mut x = 0.0f32;
        let mut i = 0;
        while i < chars.len() {
            let (face, emoji) = faces[i].clone();
            let mut j = i + 1;
            while j < chars.len() && Arc::ptr_eq(&faces[j].0, &face) && faces[j].1 == emoji {
                j += 1;
            }
            let k = self.size / face.ab.units_per_em().unwrap_or(1000.0);
            if Arc::ptr_eq(&face, &self.font.face) && !emoji && !chars[i..j].iter().any(|c| needs_shaping(*c)) {
                let mut prev = None;
                for &ch in &chars[i..j] {
                    if is_invisible(ch) && !face.covers(ch) {
                        continue;
                    }
                    let id = face.ab.glyph_id(ch);
                    if let Some(p) = prev {
                        x += face.ab.kern_unscaled(p, id) * k;
                    }
                    if !out.is_empty() {
                        x += self.letter_spacing;
                    }
                    out.push(Glyph { face: face.clone(), id, x, dy: 0.0 });
                    x += face.ab.h_advance_unscaled(id) * k;
                    prev = Some(id);
                }
            } else {
                let run: String = chars[i..j].iter().collect();
                for (id, dx, dy, adv) in shape(&face, &run) {
                    if !out.is_empty() {
                        x += self.letter_spacing;
                    }
                    out.push(Glyph { face: face.clone(), id: GlyphId(id), x: x + dx * k, dy: dy * k });
                    x += adv * k;
                }
            }
            i = j;
        }
        (out, x)
    }

    pub fn space(&self) -> f32 {
        self.measure(" ").max(self.size * 0.25) + self.letter_spacing + self.word_spacing
    }

    pub fn line_px(&self) -> f32 {
        self.size * self.line_height
    }

    /// Baseline offset from the top of a line box.
    fn baseline(&self) -> f32 {
        let k = self.k();
        let asc = self.font.ascent_unscaled() * k;
        let desc = self.font.descent_unscaled() * k; // negative
        (self.line_px() - (asc - desc)) / 2.0 + asc
    }

    /// Outlines of `word` into `pb`; colour glyphs (emoji) are collected in `color` instead.
    fn append_glyphs(&self, word: &str, x0: f32, baseline: f32, pb: &mut PathBuilder, color: &mut Vec<(Glyph, f32, f32)>) {
        let (glyphs, _) = self.glyphs(word);
        for g in glyphs {
            if g.face.color && g.face.parsed().is_some_and(|f| color::has_color(&f, g.id.0)) {
                let (gx, gy) = (x0 + g.x, baseline - g.dy);
                color.push((g, gx, gy));
                continue;
            }
            let k = self.size / g.face.ab.units_per_em().unwrap_or(1000.0);
            let (x, base) = (x0 + g.x, baseline - g.dy);
            if let Some(ol) = g.face.ab.outline(g.id) {
                let tp = |p: ab_glyph::Point| (x + p.x * k, base - p.y * k);
                let mut last: Option<(f32, f32)> = None;
                let mut open = false;
                for c in &ol.curves {
                    let (p0, rest): ((f32, f32), Vec<(f32, f32)>) = match c {
                        OutlineCurve::Line(a, b) => (tp(*a), vec![tp(*b)]),
                        OutlineCurve::Quad(a, b, c2) => (tp(*a), vec![tp(*b), tp(*c2)]),
                        OutlineCurve::Cubic(a, b, c2, d) => (tp(*a), vec![tp(*b), tp(*c2), tp(*d)]),
                    };
                    let moved = match last {
                        Some(l) => (l.0 - p0.0).abs() > 1e-3 || (l.1 - p0.1).abs() > 1e-3,
                        None => true,
                    };
                    if moved {
                        if open {
                            pb.close();
                        }
                        pb.move_to(p0.0, p0.1);
                        open = true;
                    }
                    match rest.len() {
                        1 => pb.line_to(rest[0].0, rest[0].1),
                        2 => pb.quad_to(rest[0].0, rest[0].1, rest[1].0, rest[1].1),
                        _ => pb.cubic_to(rest[0].0, rest[0].1, rest[1].0, rest[1].1, rest[2].0, rest[2].1),
                    }
                    last = Some(*rest.last().unwrap());
                }
                if open {
                    pb.close();
                }
            }
        }
    }
}

/// Characters of `text` that no font has a glyph for (they would be drawn as .notdef boxes),
/// added to `out` once each. Characters that draw nothing on their own (whitespace, joiners,
/// variation selectors, tags, skin tones, combining marks, soft hyphens) are not reported.
pub fn missing_glyphs(font: &fonts::Font, text: &str, out: &mut Vec<char>) {
    for c in text.chars() {
        if c.is_whitespace() || c.is_control() || c == SHY || c == '\u{FEFF}' || is_joiner(c) || is_invisible(c) || out.contains(&c) {
            continue;
        }
        if !font.has_glyph(c) {
            out.push(c);
        }
    }
}

/// A positioned glyph: `x` from the word origin, `dy` up from the baseline (px).
#[derive(Clone)]
struct Glyph {
    face: FaceRef,
    id: GlyphId,
    x: f32,
    dy: f32,
}

/// Shape `text` with `face`: (glyph id, x offset, y offset, advance) in font units.
fn shape(face: &Face, text: &str) -> Vec<(u16, f32, f32, f32)> {
    let Some(rb) = face.shaper() else {
        return text.chars().map(|c| (face.ab.glyph_id(c).0, 0.0, 0.0, face.ab.h_advance_unscaled(face.ab.glyph_id(c)))).collect();
    };
    let mut buf = rustybuzz::UnicodeBuffer::new();
    buf.push_str(text);
    let shaped = rustybuzz::shape(&rb, &[], buf);
    shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .map(|(i, p)| (i.glyph_id as u16, p.x_offset as f32, p.y_offset as f32, p.x_advance as f32))
        .collect()
}

/// Scripts that need shaping (right-to-left, joining or reordering marks): Hebrew, Arabic, Syriac,
/// Thaana, N'Ko, the Indic scripts, Thai, Lao, Myanmar, Khmer.
fn needs_shaping(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x0DFF | 0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

/// Right-to-left letters (Hebrew, Arabic, Syriac, Thaana, N'Ko and their presentation forms).
fn is_rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x07FF | 0x0860..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

/// Emoji presentation: pictographs, the BMP characters that default to emoji presentation
/// (Unicode `Emoji_Presentation`: ⚡ ✨ ⭐ ⌛ ✅ ❌ ...), or any character followed by VS16 or the
/// keycap mark. Followed by VS15 (U+FE0E) it stays text.
fn is_emoji(c: char, next: Option<char>) -> bool {
    if next == Some('\u{FE0E}') {
        return false;
    }
    matches!(
        c as u32,
        0x1F000..=0x1FAFF
            | 0x231A..=0x231B
            | 0x23E9..=0x23EC
            | 0x23F0
            | 0x23F3
            | 0x25FD..=0x25FE
            | 0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267F
            | 0x2693
            | 0x26A1
            | 0x26AA..=0x26AB
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26CE
            | 0x26D4
            | 0x26EA
            | 0x26F2..=0x26F3
            | 0x26F5
            | 0x26FA
            | 0x26FD
            | 0x2705
            | 0x270A..=0x270B
            | 0x2728
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2795..=0x2797
            | 0x27B0
            | 0x27BF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
    ) || matches!(next, Some('\u{FE0F}' | '\u{20E3}'))
}

/// Characters that attach to the one before them: ZWJ, variation selectors, skin tones, tags, keycap.
fn is_joiner(c: char) -> bool {
    matches!(c as u32, 0x200D | 0xFE0E | 0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F | 0x20E3)
        || (is_invisible(c) && c != '\u{200B}')
        || matches!(c as u32, 0x0300..=0x036F)
}

/// Zero-width characters that need no glyph of their own.
fn is_invisible(c: char) -> bool {
    matches!(c as u32, 0x200B..=0x200D | 0x2060 | 0xFE00..=0xFE0F | 0xE0020..=0xE007F)
}

fn is_regional(c: char) -> bool {
    matches!(c as u32, 0x1F1E6..=0x1F1FF)
}

/// Whether the regional indicators ending `before` leave a flag half-open.
fn pairs_open(before: &[char]) -> bool {
    before.iter().rev().take_while(|c| is_regional(**c)).count() % 2 == 1
}

pub struct PlacedWord {
    pub text: String,
    pub line: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
}

pub struct TextLayout {
    pub words: Vec<PlacedWord>,
    pub lines: Vec<(f32, f32, f32)>, // (x, y, width) per line
    pub width: f32,
    pub height: f32,
}

/// Greedy word wrap. `tokens` are words; "\n" tokens force a line break.
const SHY: char = '\u{ad}';

/// Greedy line breaking at `maxw`. Words with soft hyphens (U+00AD) are split there
/// (with a visible hyphen) when they don't fit.
fn break_lines(style: &TextStyle, tokens: &[String], maxw: Option<f32>) -> Vec<Vec<(String, f32)>> {
    let space = style.space();
    let mut lines: Vec<Vec<(String, f32)>> = vec![vec![]];
    let mut cur_w = 0.0f32;
    let mut queue: std::collections::VecDeque<String> = tokens.iter().cloned().collect();
    while let Some(t) = queue.pop_front() {
        if t == "\n" {
            lines.push(vec![]);
            cur_w = 0.0;
            continue;
        }
        let w = style.measure(&t);
        let line = lines.last_mut().unwrap();
        let needed = if line.is_empty() { w } else { cur_w + space + w };
        if let Some(maxw) = maxw {
            if needed > maxw {
                // try a soft-hyphen split that fits on the current line
                let room = if line.is_empty() { maxw } else { maxw - cur_w - space };
                let split = t
                    .char_indices()
                    .filter(|(_, c)| *c == SHY)
                    .map(|(k, _)| k)
                    .rev()
                    .find(|&k| style.measure(&format!("{}-", &t[..k])) <= room);
                if let Some(k) = split {
                    let head = format!("{}-", &t[..k]);
                    let hw = style.measure(&head);
                    line.push((head, hw));
                    lines.push(vec![]);
                    cur_w = 0.0;
                    queue.push_front(t[k + SHY.len_utf8()..].to_string());
                    continue;
                }
                if !line.is_empty() {
                    lines.push(vec![(t.clone(), w)]);
                    cur_w = w;
                    continue;
                }
            }
        }
        line.push((t.clone(), w));
        cur_w = needed;
    }
    lines
}

fn widest(style: &TextStyle, lines: &[Vec<(String, f32)>]) -> f32 {
    let space = style.space();
    lines.iter().map(|l| l.iter().map(|(_, w)| *w).sum::<f32>() + space * (l.len().max(1) - 1) as f32).fold(0.0, f32::max)
}

/// Lay out tokens; "\n" tokens force a line break. With `style.balance`, lines are made
/// as even as possible (like CSS `text-wrap: balance`): no lonely word on the last line.
pub fn layout(style: &TextStyle, tokens: &[String]) -> TextLayout {
    let space = style.space();
    let lh = style.line_px();
    let mut lines = break_lines(style, tokens, style.max_width);
    if let (true, Some(maxw)) = (style.balance, style.max_width) {
        let n = lines.len();
        if n > 1 {
            // narrowest width that still needs the same number of lines
            let (mut lo, mut hi) = (widest(style, &lines) / n as f32, maxw);
            for _ in 0..16 {
                let mid = (lo + hi) / 2.0;
                if break_lines(style, tokens, Some(mid)).len() > n {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lines = break_lines(style, tokens, Some(hi));
        }
    }
    let widths: Vec<f32> = lines.iter().map(|l| l.iter().map(|(_, w)| *w).sum::<f32>() + space * (l.len().max(1) - 1) as f32).collect();
    let block_w = widths.iter().cloned().fold(0.0, f32::max);
    // a paragraph whose first letter is Hebrew/Arabic/... runs right to left: words keep their
    // (logical) order in `words`, only their positions are mirrored within the line
    let rtl = tokens.iter().flat_map(|t| t.chars()).find(|c| c.is_alphabetic()).is_some_and(is_rtl);
    let mut words = Vec::new();
    let mut line_boxes = Vec::new();
    for (li, l) in lines.iter().enumerate() {
        let lw = widths[li];
        let x0 = match style.align {
            Align::Left => 0.0,
            Align::Center => (block_w - lw) / 2.0,
            Align::Right => block_w - lw,
        };
        let y = li as f32 * lh;
        line_boxes.push((x0, y, lw));
        let mut x = x0;
        for (t, w) in l {
            let wx = if rtl { x0 + lw - (x - x0) - w } else { x };
            words.push(PlacedWord { text: t.clone(), line: li, x: wx, y, w: *w });
            x += w + space;
        }
    }
    TextLayout { words, lines: line_boxes, width: block_w, height: lines.len() as f32 * lh }
}

pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push("\n".to_string());
        }
        out.extend(line.split_whitespace().map(str::to_string));
    }
    out
}

/// Render (a subset of) laid-out words into a sprite whose box = the whole layout (+bg padding).
pub fn render(style: &TextStyle, lay: &TextLayout, only: Option<&[usize]>, color: Option<Rgba>) -> Result<Sprite> {
    let (bgpx, bgpy) = style.bg.map(|b| (b.1, b.2)).unwrap_or((0.0, 0.0));
    let stroke_w = style.stroke.map(|s| s.0).unwrap_or(0.0);
    let pad = stroke_w + 4.0 + style.size * 0.25; // room for glyph overhang
    let box_w = lay.width + bgpx * 2.0;
    let box_h = lay.height + bgpy * 2.0;
    let pw = (box_w + pad * 2.0).ceil().max(1.0) as u32;
    let ph = (box_h + pad * 2.0).ceil().max(1.0) as u32;
    let mut pm = Pixmap::new(pw, ph).ok_or_else(|| anyhow!("text too large"))?;
    let (ox, oy) = (pad, pad);
    let tx = ox + bgpx;
    let ty = oy + bgpy;

    if let Some((c, px, py, r, per_line)) = style.bg {
        let mut paint = Paint::default();
        paint.set_color(c.skia(1.0));
        paint.anti_alias = true;
        if per_line {
            for (lx, ly, lw) in &lay.lines {
                if let Some(p) = rounded_rect(tx + lx - px, ty + ly, lw + px * 2.0, style.line_px(), r) {
                    pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
                }
            }
        } else if let Some(p) = rounded_rect(ox, oy, box_w, box_h, r) {
            let _ = py;
            pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    let baseline = style.baseline();
    let mut pb = PathBuilder::new();
    let mut color_glyphs = Vec::new();
    for (i, w) in lay.words.iter().enumerate() {
        if let Some(sel) = only {
            if !sel.contains(&i) {
                continue;
            }
        }
        style.append_glyphs(&w.text, tx + w.x, ty + w.y + baseline, &mut pb, &mut color_glyphs);
    }
    if let Some(path) = pb.finish() {
        if let Some((sw, sc)) = style.stroke {
            let mut paint = Paint::default();
            paint.set_color(sc.skia(1.0));
            paint.anti_alias = true;
            let stroke = Stroke { width: sw * 2.0, line_join: LineJoin::Round, line_cap: LineCap::Round, ..Stroke::default() };
            pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
        let mut paint = Paint::default();
        paint.set_color(color.unwrap_or(style.color).skia(1.0));
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
    for (g, x, y) in color_glyphs {
        if let Some(face) = g.face.parsed() {
            color::draw(&mut pm, &face, g.id.0, x, y, style.size, color.unwrap_or(style.color));
        }
    }
    let sprite = Sprite { pm, ox, oy, w: box_w, h: box_h, ppu: 1.0 };
    Ok(match style.shadow {
        Some((c, blur, dx, dy)) => with_shadow(sprite, c, blur, dx, dy),
        None => sprite,
    })
}

/// A single word as its own sprite (used by captions and word reveals).
pub fn render_word(style: &TextStyle, word: &str, color: Rgba, pill: Option<Rgba>) -> Result<Sprite> {
    let mut st = style.clone();
    st.max_width = None;
    st.align = Align::Left;
    if let Some(p) = pill {
        st.bg = Some((p, style.size * 0.18, style.size * 0.02, style.size * 0.22, false));
    }
    let lay = layout(&st, &[word.to_string()]);
    render(&st, &lay, None, Some(color))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(max_width: Option<f32>) -> TextStyle {
        TextStyle {
            font: FontDb::new(Default::default()).get(None, None).unwrap(),
            size: 40.0,
            color: Rgba::WHITE,
            line_height: 1.2,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            align: Align::Center,
            max_width,
            stroke: None,
            balance: false,
            shadow: None,
            bg: None,
        }
    }

    #[test]
    fn balances_lines_and_uses_soft_hyphens() {
        let mut st = style(Some(400.0));
        let toks = tokenize("this is a sentence that just does not fit on two lines");
        let greedy = layout(&st, &toks);
        st.balance = true;
        let bal = layout(&st, &toks);
        assert_eq!(greedy.lines.len(), bal.lines.len());
        let spread =
            |l: &TextLayout| l.lines.iter().map(|x| x.2).fold(0.0, f32::max) - l.lines.iter().map(|x| x.2).fold(f32::MAX, f32::min);
        assert!(spread(&bal) <= spread(&greedy) + 0.5);
        let st = style(Some(200.0));
        let lay = layout(&st, &tokenize("beleids\u{ad}coördinator"));
        assert_eq!(lay.words[0].text, "beleids-");
        assert_eq!(lay.lines.len(), 2);
    }

    #[test]
    fn wraps_and_breaks() {
        let st = style(Some(200.0));
        let lay = layout(&st, &tokenize("one two three four five six seven eight\nnine"));
        assert!(lay.lines.len() >= 3, "wraps into several lines");
        assert!(lay.width <= 200.0 + 1.0);
        assert_eq!(lay.words.last().unwrap().line, lay.lines.len() - 1, "explicit break starts a new line");
        assert!((lay.height - lay.lines.len() as f32 * 48.0).abs() < 0.01);
    }

    #[test]
    fn emoji_sequences_stay_together() {
        let flag: Vec<char> = "🇳🇱🇧🇪".chars().collect();
        assert!(!pairs_open(&flag[..0]) && pairs_open(&flag[..1]) && !pairs_open(&flag[..2]) && pairs_open(&flag[..3]));
        for c in ['\u{200D}', '\u{FE0F}', '\u{1F3FD}', '\u{20E3}', '\u{E0067}'] {
            assert!(is_joiner(c), "{c:?}");
        }
        assert!(is_emoji('😀', None) && is_emoji('1', Some('\u{FE0F}')) && !is_emoji('1', None) && !is_emoji('a', Some('b')));
        assert!(is_emoji('⚡', None) && is_emoji('✨', None) && !is_emoji('⚡', Some('\u{FE0E}')) && !is_emoji('✓', None));
        assert!(is_rtl('ש') && is_rtl('م') && !is_rtl('a') && !is_rtl('日'));
    }

    #[test]
    fn latin_text_takes_the_fast_path() {
        let st = style(None);
        assert!(!st.needs_fallback("Hallo,") && st.needs_fallback("hi😀") && st.needs_fallback("שלום"));
        // measuring through the glyph list gives the same width as the fast path
        let (glyphs, w) = st.glyphs("Kerning AV");
        assert_eq!(glyphs.len(), 10);
        assert!((w - st.measure("Kerning AV")).abs() < 1e-3);
    }

    #[test]
    fn windows_default_fonts_have_stand_ins_and_typos_still_fail() {
        let db = FontDb::new(Default::default());
        for f in ["Segoe UI Black", "Segoe UI Semibold", "Segoe UI", "Arial Black", "Consolas"] {
            assert!(db.get(Some(f), None).is_ok(), "{f}");
        }
        assert!(db.get(Some("Definitely Not A Font 42"), None).is_err());
    }

    #[test]
    fn missing_characters_fall_back_to_another_font() {
        let st = style(None);
        // some installed font has Greek, CJK or the check mark
        for ch in ['λ', '日', '✓'] {
            let face = st.font.face_for(ch, false);
            if !Arc::ptr_eq(&face, &st.font.face) {
                assert!(face.covers(ch), "{ch}");
            }
        }
    }

    #[test]
    fn emoji_are_drawn_in_colour() {
        let st = style(None);
        let face = st.font.face_for('😀', true);
        if !face.color {
            eprintln!("no colour emoji font installed: skipping");
            return;
        }
        let lay = layout(&st, &tokenize("😀"));
        let s = render(&st, &lay, None, None).unwrap();
        let colourful = s.pm.pixels().iter().filter(|p| p.alpha() == 255 && (p.red() as i32 - p.blue() as i32).abs() > 80).count();
        assert!(colourful > 50, "white text colour would give grey pixels only ({colourful})");
    }

    #[test]
    fn right_to_left_paragraphs_run_right_to_left() {
        let st = style(None);
        let lay = layout(&st, &tokenize("שלום עולם"));
        assert!(lay.words[0].x > lay.words[1].x, "first word on the right");
        let lay = layout(&st, &tokenize("hello world"));
        assert!(lay.words[0].x < lay.words[1].x);
    }

    #[test]
    fn variation_axes_change_the_width() {
        // Bahnschrift (Windows) has wdth and wght axes; skip where no such font is installed
        let db = FontDb::new(Default::default());
        let get = |vars: &[Axis]| db.get_with(Some("Bahnschrift"), None, vars).ok();
        let Some(normal) = get(&[(*b"wdth", 100.0)]).filter(|f| f.face.axes().contains(b"wdth")) else {
            eprintln!("no variable font with a wdth axis installed: skipping");
            return;
        };
        let narrow = get(&[(*b"wdth", 75.0)]).unwrap();
        assert!(!Arc::ptr_eq(&normal, &narrow), "cached per variation");
        let with = |font: FontRef| TextStyle { font, ..style(None) };
        let (wn, wc) = (with(normal).measure("Condensed headline"), with(narrow.clone()).measure("Condensed headline"));
        assert!(wc < wn * 0.95, "wdth 75 is narrower: {wc} vs {wn}");
        // shaping (other scripts, emoji runs) uses the same instance as the outlines
        let shaped: f32 = shape(&narrow.face, "Condensed").iter().map(|g| g.3).sum();
        let k = 40.0 / narrow.units_per_em().unwrap();
        assert!(
            (shaped * k - with(narrow.clone()).measure("Condensed")).abs() < 2.0,
            "{} vs {}",
            shaped * k,
            with(narrow).measure("Condensed")
        );
        // `weight` wins over a "wght" in variations
        let bold = db.get_with(Some("Bahnschrift"), Some(700.0), &[(*b"wght", 300.0)]).unwrap();
        assert!(bold.face.variations.contains(&(*b"wght", 700.0)) && !bold.face.variations.contains(&(*b"wght", 300.0)));
        assert!(parse_variations([(&"wdth".to_string(), &75.0)]).is_ok() && parse_variations([(&"width".to_string(), &75.0)]).is_err());
    }

    #[test]
    fn renders_sprite_with_padding() {
        let st = style(None);
        let lay = layout(&st, &tokenize("Hallo"));
        let s = render(&st, &lay, None, None).unwrap();
        assert!(s.pm.width() as f32 > s.w && s.ox > 0.0);
        assert!(s.pm.data().as_chunks::<4>().0.iter().any(|p| p[3] == 255), "glyphs are drawn");
    }
}
