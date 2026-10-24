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
    let (s, warnings, p) = match loaded {
        Ok(v) => v,
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({ "ok": false, "error": format!("{e:#}") }));
            }
            return Err(e);
        }
    };
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

pub fn info(file: &Path) -> Result<()> {
    let i = media::probe(file)?;
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

pub fn doctor() -> Result<()> {
    println!("flits {}", env!("CARGO_PKG_VERSION"));
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
            font: fonts.get(r.font.as_deref(), r.weight)?,
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
