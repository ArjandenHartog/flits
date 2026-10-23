//! Finding and loading fonts: paths, scene aliases, installed system fonts, and the fallback
//! chain for characters a font lacks (emoji, other scripts).

use ab_glyph::{Font as _, FontVec, VariableFont};
use anyhow::{anyhow, bail, Context, Result};
use rayon::prelude::*;
use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// One loaded font face.
pub struct Face {
    pub ab: FontVec,
    /// index within a font collection (.ttc)
    pub index: u32,
    /// has colour glyphs (COLR, CBDT or sbix: emoji fonts)
    pub color: bool,
}

pub type FaceRef = Arc<Face>;

impl Face {
    fn load(path: &Path, index: u32, weight: Option<f64>) -> Result<Face> {
        let data = std::fs::read(path).with_context(|| format!("reading font {}", path.display()))?;
        let color = ttf_parser::Face::parse(&data, index)
            .map(|f| {
                let t = f.tables();
                t.colr.is_some() || t.cbdt.is_some() || t.sbix.is_some()
            })
            .unwrap_or(false);
        let mut ab = FontVec::try_from_vec_and_index(data, index).map_err(|_| anyhow!("invalid font file {}", path.display()))?;
        if let Some(w) = weight {
            ab.set_variation(b"wght", w as f32);
        }
        Ok(Face { ab, index, color })
    }

    pub fn covers(&self, ch: char) -> bool {
        self.ab.glyph_id(ch).0 != 0
    }

    /// The face parsed by ttf-parser (for colour glyphs).
    pub fn parsed(&self) -> Option<ttf_parser::Face<'_>> {
        ttf_parser::Face::parse(self.ab.font_data(), self.index).ok()
    }
}

/// A font as used by a text layer: its own face plus the fallback chain for characters it lacks
/// (the scene's `fontFallback`, then the system's fallback fonts, then any installed font that has
/// the character). Derefs to the primary face.
pub struct Font {
    pub face: FaceRef,
    fallback: Arc<Vec<FaceRef>>,
}

pub type FontRef = Arc<Font>;

impl Deref for Font {
    type Target = FontVec;
    fn deref(&self) -> &FontVec {
        &self.face.ab
    }
}

impl Font {
    /// The face to draw `ch` with. `emoji`: prefer colour faces (emoji presentation).
    pub fn face_for(&self, ch: char, emoji: bool) -> FaceRef {
        let primary = || std::iter::once(&self.face);
        if emoji {
            let color = primary().chain(self.fallback.iter()).chain(system_fallbacks().iter()).find(|f| f.color && f.covers(ch));
            if let Some(f) = color {
                return f.clone();
            }
        }
        if let Some(f) = primary().chain(self.fallback.iter()).chain(system_fallbacks().iter()).find(|f| f.covers(ch)) {
            return f.clone();
        }
        covering_face(ch).unwrap_or_else(|| self.face.clone())
    }
}

/// Resolves font names ("Segoe UI Black", "Arial", a path, or a scene alias) to loaded fonts.
pub struct FontDb {
    aliases: HashMap<String, PathBuf>,
    fallback: Arc<Vec<FaceRef>>,
    cache: Mutex<HashMap<(PathBuf, u32, i32), FontRef>>,
}

/// A font file (and face index for collections) on disk.
type FontFile = (PathBuf, u32);

static SYSTEM_INDEX: OnceLock<Vec<(String, FontFile)>> = OnceLock::new();

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
            dirs.push(PathBuf::from(&h).join(".local/share/fonts"));
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
            } else if matches!(
                p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).as_deref(),
                Some("ttf" | "otf" | "ttc" | "otc")
            ) {
                out.push(p);
            }
        }
    }
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// (normalized full name / family+style) -> file + face index, built lazily by scanning system font dirs.
fn system_index() -> &'static Vec<(String, FontFile)> {
    SYSTEM_INDEX.get_or_init(|| {
        let mut files = Vec::new();
        for d in font_dirs() {
            collect_font_files(&d, &mut files);
        }
        files
            .par_iter()
            .flat_map_iter(|p| {
                let mut out = Vec::new();
                let Ok(data) = std::fs::read(p) else { return out };
                let faces = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
                for index in 0..faces {
                    let mut names = Vec::new();
                    if let Ok(face) = ttf_parser::Face::parse(&data, index) {
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
                    if index == 0 {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            names.push(norm(stem));
                        }
                    }
                    names.sort();
                    names.dedup();
                    out.extend(names.into_iter().map(|n| (n, (p.clone(), index))));
                }
                out
            })
            .collect()
    })
}

/// Fonts tried, in order, for characters the layer's font (and the scene's `fontFallback`) lack.
/// Text fonts with wide coverage first; emoji fonts are picked first for emoji anyway.
const SYSTEM_FALLBACKS: &[&str] = if cfg!(windows) {
    &[
        "Segoe UI",
        "Segoe UI Emoji",
        "Segoe UI Symbol",
        "Segoe UI Historic",
        "Nirmala UI",
        "Microsoft YaHei",
        "Yu Gothic",
        "Malgun Gothic",
        "Microsoft JhengHei",
        "Leelawadee UI",
        "Ebrima",
        "Gadugi",
        "Myanmar Text",
        "Javanese Text",
        "Mongolian Baiti",
        "Arial",
    ]
} else if cfg!(target_os = "macos") {
    &["Helvetica Neue", "Apple Color Emoji", "PingFang SC", "Hiragino Sans", "Apple SD Gothic Neo", "Apple Symbols", "Arial Unicode MS"]
} else {
    &[
        "DejaVu Sans",
        "Noto Sans",
        "Noto Color Emoji",
        "Twemoji",
        "Noto Sans CJK SC",
        "Noto Sans Arabic",
        "Noto Sans Hebrew",
        "Noto Sans Devanagari",
        "Noto Sans Thai",
        "Noto Sans Symbols",
        "Noto Sans Symbols2",
        "Symbola",
    ]
};

/// Faces are shared process-wide: a fallback font is loaded once, however many layers use it.
fn load_shared(file: &FontFile) -> Option<FaceRef> {
    static FACES: OnceLock<Mutex<HashMap<FontFile, Option<FaceRef>>>> = OnceLock::new();
    let faces = FACES.get_or_init(Default::default);
    if let Some(f) = faces.lock().unwrap().get(file) {
        return f.clone();
    }
    let face = Face::load(&file.0, file.1, None).ok().map(Arc::new);
    faces.lock().unwrap().insert(file.clone(), face.clone());
    face
}

/// The installed faces of [`SYSTEM_FALLBACKS`] (loaded on first use).
fn system_fallbacks() -> &'static [FaceRef] {
    static FALLBACKS: OnceLock<Vec<FaceRef>> = OnceLock::new();
    FALLBACKS.get_or_init(|| {
        let mut seen = Vec::new();
        SYSTEM_FALLBACKS
            .iter()
            .filter_map(|n| find_system(n))
            .filter(|f| {
                let new = !seen.contains(f);
                seen.push(f.clone());
                new
            })
            .filter_map(|f| load_shared(&f))
            .collect()
    })
}

/// Last resort: any installed font that has `ch` (remembered per character, also when none has it).
fn covering_face(ch: char) -> Option<FaceRef> {
    static FOUND: OnceLock<Mutex<HashMap<char, Option<FontFile>>>> = OnceLock::new();
    let found = FOUND.get_or_init(Default::default);
    if let Some(f) = found.lock().unwrap().get(&ch) {
        return f.as_ref().and_then(load_shared);
    }
    let mut files: Vec<&FontFile> = system_index().iter().map(|(_, f)| f).collect();
    files.sort();
    files.dedup();
    let hit = files
        .par_iter()
        .find_first(|(path, index)| {
            std::fs::read(path)
                .ok()
                .and_then(|d| ttf_parser::Face::parse(&d, *index).ok().map(|f| f.glyph_index(ch).is_some()))
                .unwrap_or(false)
        })
        .map(|f| (*f).clone());
    found.lock().unwrap().insert(ch, hit.clone());
    hit.as_ref().and_then(load_shared)
}

/// An installed font by name ("Segoe UI Black", "Noto Color Emoji").
fn find_system(name: &str) -> Option<FontFile> {
    if let Some(file) = quick_alias(name) {
        for d in font_dirs() {
            let f = d.join(file);
            if f.is_file() {
                return Some((f, 0));
            }
        }
    }
    let n = norm(name);
    let idx = system_index();
    idx.iter().find(|(k, _)| *k == n).or_else(|| idx.iter().find(|(k, _)| k.starts_with(&n))).map(|(_, f)| f.clone())
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
        FontDb { aliases, fallback: Arc::new(Vec::new()), cache: Mutex::new(HashMap::new()) }
    }

    /// Fonts to try before the system fallbacks for characters a layer's font lacks
    /// (the scene's `fontFallback`: names, paths or aliases).
    pub fn with_fallback(mut self, names: &[String]) -> Result<FontDb> {
        let faces = names
            .iter()
            .map(|n| {
                let file = self.find(n, None).with_context(|| format!("fontFallback '{n}'"))?;
                load_shared(&file).ok_or_else(|| anyhow!("fontFallback '{n}': invalid font file {}", file.0.display()))
            })
            .collect::<Result<_>>()?;
        self.fallback = Arc::new(faces);
        Ok(self)
    }

    fn find(&self, name: &str, weight: Option<f64>) -> Result<FontFile> {
        let p = PathBuf::from(name);
        if p.is_file() {
            return Ok((p, 0));
        }
        if let Some(p) = self.aliases.get(name) {
            return Ok((p.clone(), 0));
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
                        return Ok((f, 0));
                    }
                }
            }
        }
        let idx = system_index();
        for c in &candidates {
            let n = norm(c);
            if let Some((_, f)) = idx.iter().find(|(k, _)| *k == n) {
                return Ok(f.clone());
            }
        }
        for c in &candidates {
            let n = norm(c);
            if let Some((_, f)) = idx.iter().find(|(k, _)| k.starts_with(&n)) {
                return Ok(f.clone());
            }
        }
        bail!("font '{name}' not found (give a .ttf/.otf path, a scene \"fonts\" alias, or an installed font name)")
    }

    pub fn get(&self, name: Option<&str>, weight: Option<f64>) -> Result<FontRef> {
        let name = name.unwrap_or(DEFAULT_FONT);
        let (path, index) = self.find(name, weight).or_else(|e| {
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
        let key = (path.clone(), index, weight.map(|w| w as i32).unwrap_or(-1));
        if let Some(f) = self.cache.lock().unwrap().get(&key) {
            return Ok(f.clone());
        }
        let face = Arc::new(Face::load(&path, index, weight)?);
        let f = Arc::new(Font { face, fallback: self.fallback.clone() });
        self.cache.lock().unwrap().insert(key, f.clone());
        Ok(f)
    }
}
