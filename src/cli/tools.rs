//! Inspection and infrastructure commands.

use super::load;
use anyhow::Result;
use flits::compose::{Kind, Node};
use flits::media::encode::{self, Codec, Speed};
use flits::{media, render, Prepared, Scene};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

pub fn check(scene: &Path) -> Result<()> {
    let (s, warnings) = Scene::load(scene)?;
    for w in &warnings {
        println!("warning: {w}");
    }
    let p = Prepared::build(&s, 0.25)?;
    println!("ok: {}x{} @ {} fps, {:.2}s = {} frames, {} audio input(s)", s.width, s.height, s.fps, p.duration, p.frames, p.audio.len());
    fn walk(n: &[Node], depth: usize) {
        for n in n {
            let kind = match &n.kind {
                Kind::Group => "group",
                Kind::Rect(_) => "rect",
                Kind::Image { .. } => "image",
                Kind::Video { .. } => "video",
                Kind::Text { .. } => "text",
                Kind::Counter(_) => "counter",
                Kind::Captions(_) => "captions",
                Kind::Waveform { .. } => "waveform",
            };
            println!("{:indent$}{kind:<9} {:>8.2}s -> {:>8.2}s  box {:.0}x{:.0}", "", n.start, n.end, n.w, n.h, indent = depth * 2);
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
