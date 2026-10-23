//! Text layout and rasterization (glyph outlines -> tiny-skia paths).

pub mod fonts;

pub use fonts::{FontDb, FontRef, DEFAULT_FONT};

use crate::gfx::{rounded_rect, with_shadow, Sprite};
use crate::scene::color::Rgba;
use ab_glyph::{Font, OutlineCurve};
use anyhow::{anyhow, bail, Result};
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
        x + self.letter_spacing * (n.max(1) - 1) as f32
    }

    pub fn space(&self) -> f32 {
        self.measure(" ").max(self.size * 0.25) + self.letter_spacing
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

    fn append_glyphs(&self, word: &str, x0: f32, baseline: f32, pb: &mut PathBuilder) {
        let k = self.k();
        let mut x = x0;
        let mut prev = None;
        for ch in word.chars().filter(|c| *c != SHY) {
            let id = self.font.glyph_id(ch);
            if let Some(p) = prev {
                x += self.font.kern_unscaled(p, id) * k;
            }
            if let Some(ol) = self.font.outline(id) {
                let tp = |p: ab_glyph::Point| (x + p.x * k, baseline - p.y * k);
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
            x += self.font.h_advance_unscaled(id) * k + self.letter_spacing;
            prev = Some(id);
        }
    }
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
    let mut words = Vec::new();
    let mut line_boxes = Vec::new();
    for (li, l) in lines.iter().enumerate() {
        let lw = widths[li];
        let mut x = match style.align {
            Align::Left => 0.0,
            Align::Center => (block_w - lw) / 2.0,
            Align::Right => block_w - lw,
        };
        let y = li as f32 * lh;
        line_boxes.push((x, y, lw));
        for (t, w) in l {
            words.push(PlacedWord { text: t.clone(), line: li, x, y, w: *w });
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
    for (i, w) in lay.words.iter().enumerate() {
        if let Some(sel) = only {
            if !sel.contains(&i) {
                continue;
            }
        }
        style.append_glyphs(&w.text, tx + w.x, ty + w.y + baseline, &mut pb);
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
        let toks = tokenize("dit is een zin die net niet op twee regels past");
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
        let lay = layout(&st, &tokenize("een twee drie vier vijf zes zeven acht\nnegen"));
        assert!(lay.lines.len() >= 3, "wraps into several lines");
        assert!(lay.width <= 200.0 + 1.0);
        assert_eq!(lay.words.last().unwrap().line, lay.lines.len() - 1, "explicit break starts a new line");
        assert!((lay.height - lay.lines.len() as f32 * 48.0).abs() < 0.01);
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
