//! Fonts, text layout and text rasterization (glyph outlines -> tiny-skia paths).

use crate::color::Rgba;
use crate::effects::{rounded_rect, with_shadow, Sprite};
use ab_glyph::{Font, FontVec, OutlineCurve, VariableFont};
use anyhow::{anyhow, bail, Context, Result};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

pub type FontRef = Arc<FontVec>;

/// Resolves font names ("Segoe UI Black", "Arial", a path, or a scene alias) to loaded fonts.
pub struct FontDb {
    aliases: HashMap<String, PathBuf>,
    cache: Mutex<HashMap<(PathBuf, i32), FontRef>>,
}

static SYSTEM_INDEX: OnceLock<Vec<(String, PathBuf)>> = OnceLock::new();

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        dirs.push(PathBuf::from(windir).join("Fonts"));
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        dirs.push("/System/Library/Fonts".into());
        dirs.push("/Library/Fonts".into());
        if let Ok(h) = std::env::var("HOME") {
            dirs.push(PathBuf::from(h).join("Library/Fonts"));
        }
    } else {
        dirs.push("/usr/share/fonts".into());
        dirs.push("/usr/local/share/fonts".into());
        if let Ok(h) = std::env::var("HOME") {
            dirs.push(PathBuf::from(h).join(".local/share/fonts"));
            dirs.push(PathBuf::from(h).join(".fonts"));
        }
    }
    dirs
}

fn collect_font_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_font_files(&p, out);
            } else if matches!(p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).as_deref(), Some("ttf" | "otf")) {
                out.push(p);
            }
        }
    }
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// (normalized full name / family+style) -> file, built lazily by scanning system font dirs.
fn system_index() -> &'static Vec<(String, PathBuf)> {
    SYSTEM_INDEX.get_or_init(|| {
        let mut files = Vec::new();
        for d in font_dirs() {
            collect_font_files(&d, &mut files);
        }
        files
            .par_iter()
            .flat_map_iter(|p| {
                let mut names = Vec::new();
                if let Ok(data) = std::fs::read(p) {
                    if let Ok(face) = ttf_parser::Face::parse(&data, 0) {
                        let mut family = None;
                        let mut sub = None;
                        for n in face.names() {
                            let Some(s) = n.to_string() else { continue };
                            match n.name_id {
                                ttf_parser::name_id::FULL_NAME => names.push(norm(&s)),
                                ttf_parser::name_id::FAMILY if family.is_none() => family = Some(s),
                                ttf_parser::name_id::TYPOGRAPHIC_FAMILY => family = Some(s),
                                ttf_parser::name_id::SUBFAMILY if sub.is_none() => sub = Some(s),
                                ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY => sub = Some(s),
                                _ => {}
                            }
                        }
                        if let Some(f) = &family {
                            names.push(norm(f));
                            if let Some(s) = &sub {
                                names.push(norm(&format!("{f}{s}")));
                            }
                        }
                    }
                }
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    names.push(norm(stem));
                }
                names.sort();
                names.dedup();
                names.into_iter().map(|n| (n, p.clone())).collect::<Vec<_>>()
            })
            .collect()
    })
}

/// Well-known Windows font files, so common names resolve without scanning.
fn quick_alias(name: &str) -> Option<&'static str> {
    Some(match norm(name).as_str() {
        "segoeui" | "segoeuiregular" => "segoeui.ttf",
        "segoeuisemibold" => "seguisb.ttf",
        "segoeuibold" => "segoeuib.ttf",
        "segoeuiblack" => "seguibl.ttf",
        "arial" => "arial.ttf",
        "arialbold" => "arialbd.ttf",
        "arialblack" => "ariblk.ttf",
        "impact" => "impact.ttf",
        "georgia" => "georgia.ttf",
        "georgiabold" => "georgiab.ttf",
        "verdana" => "verdana.ttf",
        "verdanabold" => "verdanab.ttf",
        "bahnschrift" => "bahnschrf.ttf",
        "consolas" => "consola.ttf",
        "consolasbold" => "consolab.ttf",
        "tahomabold" => "tahomabd.ttf",
        "trebuchetms" => "trebuc.ttf",
        "trebuchetmsbold" => "trebucbd.ttf",
        _ => return None,
    })
}

pub const DEFAULT_FONT: &str = "Segoe UI Bold";

impl FontDb {
    pub fn new(aliases: HashMap<String, PathBuf>) -> FontDb {
        FontDb { aliases, cache: Mutex::new(HashMap::new()) }
    }

    fn find(&self, name: &str, weight: Option<f64>) -> Result<PathBuf> {
        let p = PathBuf::from(name);
        if p.is_file() {
            return Ok(p);
        }
        if let Some(p) = self.aliases.get(name) {
            return Ok(p.clone());
        }
        // pick a static weight variant for families like "Segoe UI" + weight 900
        let mut candidates = vec![name.to_string()];
        if let Some(w) = weight {
            let suffix = match w as i32 {
                0..=349 => "Light",
                350..=549 => "Regular",
                550..=649 => "Semibold",
                650..=849 => "Bold",
                _ => "Black",
            };
            candidates.insert(0, format!("{name} {suffix}"));
            if suffix == "Black" {
                candidates.insert(1, format!("{name} Heavy"));
                candidates.insert(2, format!("{name} ExtraBold"));
            }
        }
        for c in &candidates {
            if let Some(file) = quick_alias(c) {
                for d in font_dirs() {
                    let f = d.join(file);
                    if f.is_file() {
                        return Ok(f);
                    }
                }
            }
        }
        let idx = system_index();
        for c in &candidates {
            let n = norm(c);
            if let Some((_, p)) = idx.iter().find(|(k, _)| *k == n) {
                return Ok(p.clone());
            }
        }
        for c in &candidates {
            let n = norm(c);
            if let Some((_, p)) = idx.iter().find(|(k, _)| k.starts_with(&n)) {
                return Ok(p.clone());
            }
        }
        bail!("font '{name}' not found (give a .ttf/.otf path, a scene \"fonts\" alias, or an installed font name)")
    }

    pub fn get(&self, name: Option<&str>, weight: Option<f64>) -> Result<FontRef> {
        let name = name.unwrap_or(DEFAULT_FONT);
        let path = self.find(name, weight).or_else(|e| {
            if name == DEFAULT_FONT {
                // non-Windows fallback
                ["DejaVu Sans Bold", "Arial Bold", "Helvetica Bold", "Liberation Sans Bold", "Noto Sans Bold"]
                    .iter()
                    .find_map(|n| self.find(n, None).ok())
                    .ok_or(e)
            } else {
                Err(e)
            }
        })?;
        let key = (path.clone(), weight.map(|w| w as i32).unwrap_or(-1));
        if let Some(f) = self.cache.lock().unwrap().get(&key) {
            return Ok(f.clone());
        }
        let data = std::fs::read(&path).with_context(|| format!("reading font {}", path.display()))?;
        let mut font = FontVec::try_from_vec(data).map_err(|_| anyhow!("invalid font file {}", path.display()))?;
        if let Some(w) = weight {
            font.set_variation(b"wght", w as f32);
        }
        let f = Arc::new(font);
        self.cache.lock().unwrap().insert(key, f.clone());
        Ok(f)
    }
}

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
        for ch in word.chars() {
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
        for ch in word.chars() {
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
pub fn layout(style: &TextStyle, tokens: &[String]) -> TextLayout {
    let space = style.space();
    let lh = style.line_px();
    let mut lines: Vec<Vec<(String, f32)>> = vec![vec![]];
    let mut cur_w = 0.0f32;
    for t in tokens {
        if t == "\n" {
            lines.push(vec![]);
            cur_w = 0.0;
            continue;
        }
        let w = style.measure(t);
        let line = lines.last_mut().unwrap();
        let needed = if line.is_empty() { w } else { cur_w + space + w };
        if let Some(maxw) = style.max_width {
            if needed > maxw && !line.is_empty() {
                lines.push(vec![(t.clone(), w)]);
                cur_w = w;
                continue;
            }
        }
        line.push((t.clone(), w));
        cur_w = needed;
    }
    let widths: Vec<f32> = lines
        .iter()
        .map(|l| l.iter().map(|(_, w)| *w).sum::<f32>() + space * (l.len().max(1) - 1) as f32)
        .collect();
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
