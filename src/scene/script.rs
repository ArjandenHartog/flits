//! Scenes written in TypeScript or JavaScript (`flits render video.ts`).
//!
//! The module is run with Bun, Deno or Node (whichever is installed, or `FLITS_JS`); its default
//! export is a scene object or a function of the `--props` returning one. The SDK in
//! bindings/typescript provides components (Text, Sequence, Captions, ...) and motion helpers;
//! function-valued animatable properties are sampled per frame (compile.js, embedded here). Only
//! script scenes need a JavaScript runtime: rendering itself still needs nothing but ffmpeg.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// File extensions that are run as scripts instead of read as JSON.
pub const EXTENSIONS: &[&str] = &["ts", "mts", "cts", "tsx", "js", "mjs", "cjs", "jsx"];

const LOADER: &str = include_str!("loader.mjs");
const COMPILE: &str = include_str!("../../bindings/typescript/compile.js");

static PROPS: Mutex<Option<Value>> = Mutex::new(None);

/// Props passed to a scene module that exports a function (`--props`).
pub fn set_props(v: Value) {
    *PROPS.lock().unwrap() = Some(v);
}

pub fn is_script(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// `--props`: inline JSON or the path of a JSON file.
pub fn parse_props(s: &str) -> Result<Value> {
    let t = s.trim();
    let text = if t.starts_with('{') || t.starts_with('[') {
        t.to_string()
    } else {
        std::fs::read_to_string(t).with_context(|| format!("reading props {t}"))?
    };
    serde_json::from_str(&text).context("--props is not valid JSON")
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Bun,
    Deno,
    /// with `--experimental-strip-types` (Node 22.6 .. 22.17 / 23.0 .. 23.5)
    Node {
        strip_flag: bool,
        strips: bool,
    },
}

#[derive(Clone, Debug)]
struct Runtime {
    bin: String,
    kind: Kind,
    version: String,
}

fn version_of(bin: &str) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string())
}

fn node_kind(version: &str) -> Kind {
    let v: Vec<u32> = version.trim_start_matches('v').split('.').filter_map(|p| p.parse().ok()).collect();
    let (major, minor) = (v.first().copied().unwrap_or(0), v.get(1).copied().unwrap_or(0));
    let default_on = major >= 24 || (major == 23 && minor >= 6) || (major == 22 && minor >= 18);
    let flag = !default_on && ((major == 22 && minor >= 6) || major == 23);
    Kind::Node { strip_flag: flag, strips: default_on || flag }
}

fn probe(bin: &str) -> Option<Runtime> {
    let version = version_of(bin)?;
    let name = Path::new(bin).file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let kind = if name.contains("bun") {
        Kind::Bun
    } else if name.contains("deno") {
        Kind::Deno
    } else {
        node_kind(&version)
    };
    Some(Runtime { bin: bin.to_string(), kind, version })
}

/// The installed runtimes, in order of preference (FLITS_JS first).
fn runtimes() -> &'static [Runtime] {
    static ALL: OnceLock<Vec<Runtime>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut bins: Vec<String> = std::env::var("FLITS_JS").ok().filter(|s| !s.is_empty()).into_iter().collect();
        bins.extend(["bun", "deno", "node"].map(String::from));
        bins.iter().filter_map(|b| probe(b)).collect()
    })
}

fn pick(path: &Path) -> Result<&'static Runtime> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let jsx = ext == "tsx" || ext == "jsx";
    let ts = matches!(ext.as_str(), "ts" | "mts" | "cts" | "tsx");
    let all = runtimes();
    if all.is_empty() {
        bail!("{} is a script scene: install Bun, Deno or Node 22.6+ (or set FLITS_JS) to run it", path.display());
    }
    let explicit = std::env::var("FLITS_JS").is_ok();
    let ok = |r: &Runtime| match r.kind {
        Kind::Bun | Kind::Deno => true,
        Kind::Node { strips, .. } => !jsx && (!ts || strips),
    };
    match all.iter().find(|r| ok(r)) {
        Some(r) if !explicit || std::ptr::eq(r, &all[0]) => Ok(r),
        _ if jsx => {
            bail!("{}: JSX needs Bun or Deno (Node's type stripping does not transform JSX); use .ts or install Bun", path.display())
        }
        _ => bail!("{}: TypeScript needs Node 22.6+, Bun or Deno (found {} {})", path.display(), all[0].bin, all[0].version),
    }
}

/// Whether an installed runtime can run this scene module.
pub fn can_run(path: &Path) -> bool {
    pick(path).is_ok()
}

/// One line for `flits doctor`.
pub fn describe() -> String {
    match runtimes() {
        [] => "no JavaScript runtime: .ts/.js scenes need Bun, Deno or Node 22.6+ (JSON scenes don't)".into(),
        all => {
            let list: Vec<String> = all.iter().map(|r| format!("{} {}", r.bin, r.version)).collect();
            format!("{} (TypeScript scenes run with {})", list.join(", "), all[0].bin)
        }
    }
}

/// Write the loader next to its compile.js in a per-version temp folder (once).
fn loader_dir() -> Result<PathBuf> {
    let hash = LOADER.bytes().chain(COMPILE.bytes()).fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let dir = std::env::temp_dir().join(format!("flits-loader-{hash:016x}"));
    if !dir.join("loader.mjs").is_file() || !dir.join("compile.js").is_file() {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("compile.js"), COMPILE)?;
        // a package.json so Node treats compile.js as an ES module
        std::fs::write(dir.join("package.json"), "{\"type\": \"module\"}\n")?;
        std::fs::write(dir.join("loader.mjs"), LOADER)?;
    }
    Ok(dir)
}

/// Run a scene module and return the scene JSON it exports.
pub fn load(path: &Path) -> Result<Value> {
    let rt = pick(path)?;
    let file = std::fs::canonicalize(path).with_context(|| format!("reading {}", path.display()))?;
    let dir = loader_dir()?;
    let out = dir.join(format!("scene-{}-{}.json", std::process::id(), unique()));
    let props = PROPS.lock().unwrap().clone().unwrap_or(Value::Object(Default::default())).to_string();
    let mut cmd = Command::new(&rt.bin);
    match rt.kind {
        Kind::Bun => {
            cmd.arg("run");
        }
        Kind::Deno => {
            cmd.args(["run", "-A", "--quiet"]);
        }
        Kind::Node { strip_flag, .. } => {
            if strip_flag {
                cmd.arg("--experimental-strip-types");
            }
            cmd.args(["--no-warnings", "--enable-source-maps"]);
        }
    }
    cmd.arg(dir.join("loader.mjs")).arg(&file).arg(&props).arg(&out);
    // run from the project root (nearest package.json/tsconfig.json), like npm scripts: runtimes find
    // their tsconfig (JSX settings) and node_modules from there
    let root = file.ancestors().skip(1).find(|d| d.join("package.json").is_file() || d.join("tsconfig.json").is_file()).or(file.parent());
    if let Some(root) = root {
        cmd.current_dir(root);
    }
    let res = cmd.output().with_context(|| format!("running {}", rt.bin))?;
    // the script's own output (console.log) goes to stderr, so stdout stays clean for flits
    let mut log = String::from_utf8_lossy(&res.stdout).into_owned();
    log += &String::from_utf8_lossy(&res.stderr);
    let json = std::fs::read_to_string(&out);
    let _ = std::fs::remove_file(&out);
    if !res.status.success() {
        bail!("{} failed ({} {}):\n{}", path.display(), rt.bin, rt.version, tidy_error(&log));
    }
    if !log.trim().is_empty() {
        eprint!("{log}");
    }
    let json = json.map_err(|e| anyhow!("{}: the loader wrote no scene ({e})", path.display()))?;
    serde_json::from_str(&json).with_context(|| format!("{}: exported scene is not valid JSON", path.display()))
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// Drop the runtime's internal stack frames from an error.
fn tidy_error(log: &str) -> String {
    let lines: Vec<&str> = log
        .lines()
        .filter(|l| !(l.contains("node:internal") || l.contains("loader.mjs") || l.trim() == "^" || l.contains("Node.js v")))
        .collect();
    lines.join("\n").trim().to_string()
}

/// Source files of a script scene to watch for changes (its folder tree, without node_modules).
pub fn sources(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![path.to_path_buf()];
    let Some(root) = path.parent() else { return out };
    let root = if root.as_os_str().is_empty() { Path::new(".") } else { root };
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" || name == "out" || name == "target" {
                continue;
            }
            if p.is_dir() {
                if depth < 4 {
                    walk(&p, depth + 1, out);
                }
            } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e) || e == "json") && out.len() < 1000 {
                out.push(p);
            }
        }
    }
    walk(root, 0, &mut out);
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_versions() {
        assert_eq!(node_kind("v25.5.0"), Kind::Node { strip_flag: false, strips: true });
        assert_eq!(node_kind("v22.18.0"), Kind::Node { strip_flag: false, strips: true });
        assert_eq!(node_kind("v22.6.0"), Kind::Node { strip_flag: true, strips: true });
        assert_eq!(node_kind("v20.11.1"), Kind::Node { strip_flag: false, strips: false });
    }

    #[test]
    fn props_inline_or_file() {
        assert_eq!(parse_props(r#"{"a": 1}"#).unwrap()["a"], 1);
        assert!(parse_props("{nope").is_err());
    }
}
