//! Finding and loading fonts: paths, scene aliases, installed system fonts.

use ab_glyph::{FontVec, VariableFont};
use anyhow::{anyhow, bail, Context, Result};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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
