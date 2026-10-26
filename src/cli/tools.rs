//! Inspection and infrastructure commands.

use super::load;
use anyhow::Result;
use flits::compose::{Kind, Node};
use flits::media::encode::{self, Codec, Speed};
use flits::{media, render, Prepared, Scene};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

fn kind_name(k: &Kind) -> &'static str {
    match k {
        Kind::Group => "group",
        Kind::Rect(_) => "rect",
        Kind::Image { .. } => "image",
        Kind::Video { .. } => "video",
        Kind::Text { .. } => "text",
        Kind::Counter(_) => "counter",
        Kind::Captions(_) => "captions",
        Kind::Waveform { .. } => "waveform",
        Kind::Shape(_) => "shape",
    }
}

/// Seconds and pixels in JSON: 3 decimals.
fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// `flits check`: validate, lint the layout and print the timeline (`json`: one JSON object on stdout).
pub fn check(scene: &Path, json: bool) -> Result<()> {
    let loaded = Scene::load(scene).and_then(|(s, w)| Prepared::build(&s, 0.25).map(|p| (s, w, p)));
    let (s, mut warnings, p) = match loaded {
        Ok(v) => v,
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({ "ok": false, "error": format!("{e:#}") }));
            }
            return Err(e);
        }
    };
    warnings.extend(p.warnings.iter().cloned());
    let issues = flits::compose::lint::lint(&p);
    if json {
        fn tree(n: &[Node]) -> Vec<serde_json::Value> {
            n.iter()
                .map(|n| {
                    let mut v = serde_json::json!({
                        "type": kind_name(&n.kind), "label": n.label, "start": r3(n.start), "end": r3(n.end),
                        "width": r3(n.w as f64), "height": r3(n.h as f64),
                    });
                    if !n.children.is_empty() {
                        v["children"] = tree(&n.children).into();
                    }
                    v
                })
                .collect()
        }
        let audio: Vec<_> =
            p.audio.iter().map(|a| serde_json::json!({ "src": a.path, "start": r3(a.start), "duration": r3(a.dur) })).collect();
        let out = serde_json::json!({
            "ok": true, "width": s.width, "height": s.height, "fps": s.fps, "duration": r3(p.duration), "frames": p.frames,
            "warnings": warnings, "issues": issues, "audio": audio, "layers": tree(&p.nodes),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    for w in &warnings {
        println!("warning: {w}");
    }
    for i in &issues {
        println!("warning: {:.2}-{:.2}s: {}", i.from, i.to, i.message);
    }
    println!("ok: {}x{} @ {} fps, {:.2}s = {} frames, {} audio input(s)", s.width, s.height, s.fps, p.duration, p.frames, p.audio.len());
    fn walk(n: &[Node], depth: usize) {
        for n in n {
            let label: String = n.label.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(48).collect();
            let label = if label.is_empty() { String::new() } else { format!("  \"{label}\"") };
            println!(
                "{:indent$}{:<9} {:>8.2}s -> {:>8.2}s  box {:.0}x{:.0}{label}",
                "",
                kind_name(&n.kind),
                n.start,
                n.end,
                n.w,
                n.h,
                indent = depth * 2
            );
            walk(&n.children, depth + 1);
        }
    }
    walk(&p.nodes, 1);
    Ok(())
}

/// Loudness over time (`flits info --peaks`): RMS levels in dBFS of the mono mix.
pub struct Levels {
    /// centre of the loudest `window` (s) and its RMS level
    pub loudest: (f64, f64),
    /// loudest single sample: time and level
    pub peak: (f64, f64),
    /// length of each entry in `levels` (s)
    pub step: f64,
    /// RMS level per `step`
    pub levels: Vec<f64>,
}

/// Sample rate the levels are measured at: enough for a meaningful RMS of speech, engines and music.
const LEVEL_RATE: u32 = 8000;

/// Level in dBFS (0.1 dB; silence floors at -120).
fn db(amplitude: f64) -> f64 {
    (20.0 * amplitude.max(1e-6).log10() * 10.0).round() / 10.0
}

/// RMS levels of mono samples at `rate`: the loudest `window` (10 ms hops) and one level per step,
/// the step growing (1, 2, 5, 10, ... s) so long files stay at most 120 entries.
pub fn levels(x: &[f32], rate: u32, window: f64) -> Levels {
    let r = rate as f64;
    let n = x.len();
    let mut sq = Vec::with_capacity(n + 1);
    let mut acc = 0.0f64;
    sq.push(0.0);
    for &v in x {
        acc += v as f64 * v as f64;
        sq.push(acc);
    }
    let rms = |a: usize, b: usize| if b > a { ((sq[b] - sq[a]).max(0.0) / (b - a) as f64).sqrt() } else { 0.0 };
    let win = ((window * r).round() as usize).clamp(1, n.max(1));
    let hop = (rate as usize / 100).max(1);
    let mut loudest = (win as f64 / 2.0 / r, 0.0);
    let mut start = 0;
    while start + win <= n {
        let v = rms(start, start + win);
        if v > loudest.1 {
            loudest = ((start as f64 + win as f64 / 2.0) / r, v);
        }
        start += hop;
    }
    let (pi, pv) = x.iter().enumerate().fold((0, 0.0f32), |m, (i, v)| if v.abs() > m.1 { (i, v.abs()) } else { m });
    let dur = n as f64 / r;
    let step = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0]
        .into_iter()
        .find(|s| dur / s <= 120.0)
        .unwrap_or_else(|| (dur / 120.0).ceil());
    let per = ((step * r) as usize).max(1);
    let levels = (0..n.div_ceil(per)).map(|i| db(rms(i * per, ((i + 1) * per).min(n)))).collect();
    Levels { loudest: (r3(loudest.0), db(loudest.1)), peak: (r3(pi as f64 / r), db(pv as f64)), step, levels }
}

/// `12s` below a minute, `1:05` for longer files.
fn clock(t: f64, long: bool) -> String {
    if long {
        format!("{}:{:02}", (t / 60.0).floor(), (t % 60.0).floor())
    } else {
        format!("{t:.0}s")
    }
}

/// One bar per level, the top 48 dB below the loudest.
fn sparkline(levels: &[f64]) -> String {
    let bars: Vec<char> = "▁▂▃▄▅▆▇█".chars().collect();
    let top = levels.iter().cloned().fold(-120.0, f64::max);
    let floor = top - 48.0;
    levels.iter().map(|&l| bars[((l - floor) / (top - floor) * 7.0).round().clamp(0.0, 7.0) as usize]).collect()
}

pub fn info(file: &Path, peaks: bool, window: f64, json: bool) -> Result<()> {
    let i = media::probe(file)?;
    let levels = if peaks {
        if !i.has_audio {
            anyhow::bail!("{} has no audio stream", file.display());
        }
        if window.is_nan() || window <= 0.0 {
            anyhow::bail!("--window must be a positive number of seconds");
        }
        let x = media::audio::decode_audio(file, 0.0, i.duration.max(0.0) + 1.0, LEVEL_RATE)?;
        Some(levels(&x, LEVEL_RATE, window))
    } else {
        None
    };
    if let (Some(l), true) = (&levels, json) {
        let out = serde_json::json!({
            "file": file, "duration": r3(i.duration), "window": window,
            "loudest": {"time": l.loudest.0, "db": l.loudest.1},
            "peak": {"time": l.peak.0, "db": l.peak.1},
            "step": l.step, "levels": l.levels,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    println!("{}", file.display());
    println!("  duration   {:.2}s", i.duration);
    if i.has_video {
        println!("  video      {}x{} @ {:.3} fps", i.width, i.height, i.fps);
        let c = i.color;
        println!(
            "  colour     {:?} / {:?} / {} range / {}",
            c.transfer,
            c.matrix,
            if c.full_range { "full" } else { "limited" },
            if c.high_bit { "10-bit+" } else { "8-bit" }
        );
    } else if i.width > 0 {
        println!("  image      {}x{}", i.width, i.height);
    }
    println!("  audio      {}", if i.has_audio { "yes" } else { "no" });
    if let Some(l) = levels {
        println!("  loudest    {:.2}s  {:.1} dBFS RMS ({window} s window)", l.loudest.0, l.loudest.1);
        println!("  peak       {:.2}s  {:.1} dBFS (sample)", l.peak.0, l.peak.1);
        println!("  levels     dBFS RMS per {} s  {}", l.step, sparkline(&l.levels));
        let long = i.duration >= 60.0;
        let cells: Vec<String> =
            l.levels.iter().enumerate().map(|(k, v)| format!("{:>5} {v:>6.1}", clock(k as f64 * l.step, long))).collect();
        for row in cells.chunks(8) {
            println!("            {}", row.join("  "));
        }
    }
    Ok(())
}

fn tick(ok: bool) -> &'static str {
    if ok {
        "ok "
    } else {
        "-- "
    }
}

fn works(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).output().map(|o| o.status.success()).unwrap_or(false)
}

/// Version of a flits checkout: the `version` of its Cargo.toml `[package]`, when it is flits.
fn checkout_version(dir: &Path) -> Option<String> {
    let toml = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    let (mut name, mut version) = (None, None);
    for line in toml.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        let Some((k, v)) = line.split_once('=').filter(|_| in_package) else { continue };
        let v = v.trim().trim_matches('"').to_string();
        match k.trim() {
            "name" => name = Some(v),
            "version" => version = Some(v),
            _ => {}
        }
    }
    version.filter(|_| name.as_deref() == Some("flits"))
}

/// Why the running binary is out of date compared with the flits checkout in `dir` (its version or
/// HEAD commit differs from what this binary was built from), if it is.
fn stale_against(dir: &Path) -> Option<String> {
    let version = checkout_version(dir)?;
    if version != env!("CARGO_PKG_VERSION") {
        return Some(format!("the checkout is version {version}, this binary {}", env!("CARGO_PKG_VERSION")));
    }
    let built = env!("FLITS_GIT_COMMIT");
    if built.is_empty() || !dir.join(".git").exists() {
        return None;
    }
    let head = Command::new("git").arg("-C").arg(dir).args(["rev-parse", "HEAD"]).output().ok().filter(|o| o.status.success())?;
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    (!head.is_empty() && head != built).then(|| format!("the checkout is at commit {:.9}, this binary was built from {built:.9}", head))
}

/// `flits doctor`: which binary runs, and whether a flits checkout (the current folder or the one it
/// was built from) has moved on since.
fn doctor_binary() {
    let exe = std::env::current_exe().ok();
    let shown = exe.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "?".into());
    let mut dirs: Vec<std::path::PathBuf> = vec![];
    for d in [std::env::current_dir().ok(), Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")))].into_iter().flatten() {
        let d = std::fs::canonicalize(&d).unwrap_or(d);
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    let stale: Vec<_> = dirs.iter().filter_map(|d| stale_against(d).map(|why| (d, why))).collect();
    println!("{}binary    flits {} at {shown}", tick(stale.is_empty()), env!("FLITS_LONG_VERSION"));
    let exe = exe.and_then(|p| std::fs::canonicalize(p).ok());
    for (d, why) in stale {
        // a development build inside the checkout just needs rebuilding; an installed one reinstalling
        let fix = if exe.as_ref().is_some_and(|e| e.starts_with(d.join("target"))) {
            "cargo build --release".to_string()
        } else {
            format!("cargo install --path {}", d.display().to_string().trim_start_matches(r"\\?\"))
        };
        println!("   warning: this binary may be out of date: {why}; run `{fix}`");
    }
}

pub fn doctor() -> Result<()> {
    doctor_binary();
    // ffmpeg
    let ff = Command::new("ffmpeg").arg("-version").output().ok().filter(|o| o.status.success());
    match &ff {
        Some(o) => println!("{}ffmpeg    {}", tick(true), String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim()),
        None => println!(
            "{}ffmpeg    not found: install it (Windows: `winget install ffmpeg` or scoop, Linux: `apt install ffmpeg`)",
            tick(false)
        ),
    }
    if ff.is_none() {
        return Ok(());
    }
    // compositor
    match flits::pick_gpu("gpu") {
        Ok(Some(g)) => println!("{}GPU       {} ({}10-bit video planes)", tick(true), g.name, if g.high_bit_video { "" } else { "no " }),
        _ => println!("{}GPU       no usable Vulkan/D3D12/Metal adapter: rendering falls back to the CPU (much slower)", tick(false)),
    }
    // encoders
    let mut found = vec![];
    for (name, encoder) in [("NVENC", encode::Encoder::Nvenc), ("Intel QSV", encode::Encoder::Qsv), ("VAAPI", encode::Encoder::Vaapi)] {
        let s = encode::Settings { encoder, codec: Codec::H264, speed: Speed::Balanced, quality: 21 };
        if encode::encoder_works(s.codec_name()) {
            found.push(name);
        }
    }
    let auto = encode::pick("auto", Codec::H264, Speed::Balanced, 21)?;
    println!(
        "{}encoders  hardware: {}; default: {}",
        tick(!found.is_empty()),
        if found.is_empty() { "none (CPU x264)".into() } else { found.join(", ") },
        auto.describe()
    );
    // fonts
    let fonts = flits::text::FontDb::new(Default::default());
    match fonts.get(None, None) {
        Ok(_) => println!("{}fonts     default font found", tick(true)),
        Err(e) => println!("{}fonts     {e}", tick(false)),
    }
    // TypeScript / JavaScript scenes
    let js = flits::scene::script::describe();
    println!("{}scripts   {js}", tick(!js.starts_with("no ")));
    // transcription
    let uv = works("uv", &["--version"]);
    let nvidia = works("nvidia-smi", &["-L"]);
    println!(
        "{}transcribe uv {}; NVIDIA GPU {} (whisper runs {})",
        tick(uv),
        if uv { "found" } else { "missing (install: https://docs.astral.sh/uv/)" },
        if nvidia { "found" } else { "not found" },
        if nvidia { "on CUDA: ~25x realtime" } else { "on the CPU unless a GPU server is configured" }
    );
    match flits::remote::configured() {
        None => println!("{}remote    no GPU server configured (`flits remote http://host:7777 --token ...`)", tick(nvidia)),
        Some(r) => match r.health() {
            Ok(h) => println!("{}remote    {} reachable, GPU: {}", tick(true), r.url, h["gpu"].as_str().unwrap_or("none")),
            Err(e) => println!("{}remote    {} not reachable ({e})", tick(false), r.url),
        },
    }
    Ok(())
}

pub fn serve(port: u16, token: Option<String>) -> Result<()> {
    let token = token.or_else(|| std::env::var("FLITS_TOKEN").ok()).unwrap_or_else(flits::remote::new_token);
    eprintln!("flits: token {token}");
    eprintln!("flits: on other machines run: flits remote http://<this-machine>:{port} --token {token}");
    flits::remote::serve(port, token)
}

pub fn preview(scene: &Path, port: u16, scale: f64, gpu: &str, open: bool) -> Result<()> {
    if !scene.exists() {
        anyhow::bail!("{} not found", scene.display());
    }
    let pv = flits::preview::Preview::start(scene, port, scale, flits::pick_gpu(gpu)?)?;
    let url = pv.url();
    eprintln!("flits: preview of {} at {url}  (watching for changes, Ctrl+C to stop)", scene.display());
    if open && !flits::preview::open_browser(&url) {
        eprintln!("flits: open {url} in your browser");
    }
    pv.wait();
    Ok(())
}

pub fn remote(url: Option<String>, token: Option<String>) -> Result<()> {
    if let Some(url) = url {
        let p = flits::remote::save(&url, &token.unwrap_or_default())?;
        println!("saved {}", p.display());
    }
    match flits::remote::configured() {
        None => println!("no GPU server configured (work runs locally)"),
        Some(r) => match r.health() {
            Ok(h) => println!("{} reachable: {h}", r.url),
            Err(e) => println!("{} NOT reachable: {e}", r.url),
        },
    }
    Ok(())
}

pub fn bench(scene: &Path, at: f64, frames: i64) -> Result<()> {
    let p = load(scene, 1.0)?;
    let f0 = (at * p.fps) as i64;
    let mut backends = vec![("CPU".to_string(), None)];
    if let Some(g) = flits::pick_gpu("auto")? {
        backends.insert(0, (format!("GPU {}", g.name), Some(g)));
    }
    for (name, g) in backends {
        let mut fr = render::FrameRenderer::new(&p, g)?;
        let mut yuv = Vec::new();
        fr.yuv(&p, f0, &mut yuv)?; // warm up (uploads, decoders)
        let t = Instant::now();
        for f in f0..f0 + frames {
            fr.yuv(&p, f, &mut yuv)?;
        }
        println!("{name}: {:.2} ms/frame (compose + video decode + yuv)", t.elapsed().as_secs_f64() * 1000.0 / frames as f64);
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeasureReq {
    text: String,
    font: Option<String>,
    #[serde(default = "default_size")]
    size: f32,
    weight: Option<f64>,
    #[serde(default)]
    variations: std::collections::BTreeMap<String, f64>,
    max_width: Option<f32>,
    #[serde(default = "default_lh")]
    line_height: f32,
    #[serde(default)]
    letter_spacing: f32,
    #[serde(default)]
    balance: bool,
    #[serde(default)]
    uppercase: bool,
}

fn default_size() -> f32 {
    64.0
}
fn default_lh() -> f32 {
    1.15
}

pub fn measure() -> Result<()> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let reqs: Vec<MeasureReq> = serde_json::from_str(&input)?;
    let fonts = flits::text::FontDb::new(Default::default());
    let mut out = vec![];
    for r in reqs {
        let style = flits::text::TextStyle {
            font: fonts.get_with(r.font.as_deref(), r.weight, &flits::text::parse_variations(&r.variations)?)?,
            size: r.size,
            color: flits::scene::color::Rgba::WHITE,
            line_height: r.line_height,
            letter_spacing: r.letter_spacing,
            word_spacing: 0.0,
            align: flits::text::Align::Left,
            max_width: r.max_width,
            stroke: None,
            balance: r.balance,
            shadow: None,
            bg: None,
        };
        let text = if r.uppercase { r.text.to_uppercase() } else { r.text.clone() };
        let lay = flits::text::layout(&style, &flits::text::tokenize(&text.replace("&shy;", "\u{ad}")));
        let lines: Vec<String> = (0..lay.lines.len())
            .map(|li| lay.words.iter().filter(|w| w.line == li).map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "))
            .collect();
        let widths: Vec<f32> = lay.lines.iter().map(|l| l.2).collect();
        out.push(serde_json::json!({"width": lay.width, "height": lay.height, "lines": lines, "lineWidths": widths}));
    }
    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_of_a_flits_checkout_only() {
        let dir = std::env::temp_dir().join(format!("flits-doctor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let toml = "[package]\nname = \"flits\"\nversion = \"9.9.9\"\n\n[dependencies]\nversion = \"1\"\n";
        std::fs::write(dir.join("Cargo.toml"), toml).unwrap();
        assert_eq!(checkout_version(&dir).as_deref(), Some("9.9.9"));
        assert!(stale_against(&dir).unwrap().contains("9.9.9"));
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"other\"\nversion = \"9.9.9\"\n").unwrap();
        assert_eq!(checkout_version(&dir), None);
        assert_eq!(stale_against(&dir), None);
        // the same version without git: nothing to compare
        let same = format!("[package]\nname = \"flits\"\nversion = \"{}\"\n", env!("CARGO_PKG_VERSION"));
        std::fs::write(dir.join("Cargo.toml"), same).unwrap();
        assert_eq!(stale_against(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn levels_of_a_burst() {
        let rate = 8000;
        let x: Vec<f32> = (0..rate * 4)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let a = if (2.0..2.5).contains(&t) { 0.5 } else { 0.01 };
                a * (t * 440.0 * std::f32::consts::TAU).sin()
            })
            .collect();
        let l = levels(&x, rate, 0.25);
        assert!(l.loudest.0 > 2.1 && l.loudest.0 < 2.4, "{}", l.loudest.0);
        assert!((l.loudest.1 + 9.0).abs() < 0.5, "{}", l.loudest.1); // 0.5 amplitude: -9 dBFS RMS
        assert_eq!(l.step, 1.0);
        assert_eq!(l.levels.len(), 4);
        assert!((l.levels[0] + 43.0).abs() < 0.5);
        assert_eq!(sparkline(&l.levels).chars().count(), 4);
    }
}
