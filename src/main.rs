
use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use flits::{compose as layers, paint::gpu, render, templates, transcribe, Prepared, Scene};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "flits", version, about = "Bliksemsnelle video- en podcastrenderer (JSON-scene -> MP4) met ingebouwde transcriptie")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct RenderArgs {
    /// parallel render workers (default: logical cores / 2, max 8 for NVENC)
    #[arg(long, short = 'j')]
    workers: Option<usize>,
    /// auto | nvenc | qsv | vaapi | x264
    #[arg(long, default_value = "auto")]
    encoder: String,
    /// quality: lower = better (CRF/CQ), 18..28
    #[arg(long, default_value_t = 21)]
    quality: u32,
    /// render scale, e.g. 0.5 for a fast half-resolution preview
    #[arg(long, default_value_t = 1.0)]
    scale: f64,
    /// keep the temporary segment files
    #[arg(long)]
    keep_temp: bool,
    /// compositor: auto | gpu | cpu
    #[arg(long, default_value = "auto")]
    gpu: String,
}


#[derive(Args, Clone)]
struct WhisperArgs {
    /// whisper model: default (= large-v3-turbo on GPU, small on CPU), tiny, base, small, medium, large-v3, large-v3-turbo
    #[arg(long, default_value = "default")]
    model: String,
    /// language code (nl, en, ...) or auto
    #[arg(long, default_value = "auto")]
    lang: String,
    /// auto | cuda | cpu
    #[arg(long, default_value = "auto")]
    device: String,
    /// python with faster-whisper installed (default: run through uv)
    #[arg(long)]
    python: Option<PathBuf>,
    /// GPU server URL (default: the one saved with `flits remote`), or "off" to force local
    #[arg(long)]
    remote: Option<String>,
}

impl WhisperArgs {
    fn opts(&self) -> transcribe::TranscribeOpts {
        transcribe::TranscribeOpts { model: self.model.clone(), language: self.lang.clone(), device: self.device.clone(), python: self.python.clone(), remote: self.remote.clone() }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Render a scene JSON to an MP4
    Render {
        scene: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        r: RenderArgs,
        /// render on the configured GPU server (`flits remote`)
        #[arg(long)]
        remote: bool,
    },
    /// Render single frames to PNG (fast visual check)
    Still {
        scene: PathBuf,
        /// time(s) in seconds or mm:ss; repeat for several
        #[arg(long = "at", required = true, value_parser = parse_time)]
        at: Vec<f64>,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1.0)]
        scale: f64,
    },
    /// Contact sheet of evenly spaced frames in one PNG (check a whole video at a glance)
    Sheet {
        scene: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 12)]
        count: usize,
        #[arg(long, default_value_t = 4)]
        cols: usize,
        /// width of each thumbnail in px
        #[arg(long, default_value_t = 480)]
        thumb: u32,
    },
    /// Check a scene for errors and print its timeline
    Check { scene: PathBuf },
    /// Transcribe audio/video to word-timed JSON + SRT + VTT + TXT
    Transcribe {
        input: PathBuf,
        /// output JSON (default: <input>.words.json)
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        w: WhisperArgs,
    },
    /// Burn animated word-by-word captions into a video (transcribes automatically)
    Captions {
        video: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// karaoke | pop | word | line
        #[arg(long, default_value = "pop")]
        style: String,
        #[arg(long, default_value = "#ffd60a")]
        highlight: String,
        #[arg(long)]
        uppercase: bool,
        /// vertical position as fraction of the height
        #[arg(long, default_value_t = 0.72)]
        position: f64,
        /// existing transcript (.json/.srt/.vtt) instead of transcribing
        #[arg(long)]
        transcript: Option<PathBuf>,
        #[command(flatten)]
        w: WhisperArgs,
        #[command(flatten)]
        r: RenderArgs,
    },
    /// Turn a podcast (audio) into a video: cover, live waveform, animated captions, progress bar
    Podcast {
        audio: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long)]
        cover: Option<PathBuf>,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long)]
        subtitle: Option<String>,
        /// 16:9 | 9:16 | 1:1
        #[arg(long, default_value = "16:9")]
        format: String,
        /// clip start (seconds or mm:ss)
        #[arg(long, value_parser = parse_time, default_value = "0")]
        from: f64,
        /// clip end (seconds or mm:ss)
        #[arg(long, value_parser = parse_time)]
        to: Option<f64>,
        #[arg(long, default_value = "#f26328")]
        accent: String,
        /// captions style: karaoke | pop | word | line
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        transcript: Option<PathBuf>,
        #[command(flatten)]
        w: WhisperArgs,
        #[command(flatten)]
        r: RenderArgs,
    },
    /// Print the scene format reference
    Spec,
    /// Run a GPU server other machines can offload transcription and rendering to
    Serve {
        #[arg(long, default_value_t = 7777)]
        port: u16,
        /// shared secret (default: $FLITS_TOKEN or a new random one)
        #[arg(long)]
        token: Option<String>,
    },
    /// Set (or show) the GPU server this machine offloads to
    Remote {
        /// e.g. http://laptop:7777 ; "off" to disable
        url: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    /// Time single-threaded frame rendering (developer tool)
    #[command(hide = true)]
    Bench {
        scene: PathBuf,
        #[arg(long, default_value_t = 0.0)]
        at: f64,
        #[arg(long, default_value_t = 60)]
        frames: i64,
    },
}

fn parse_time(s: &str) -> Result<f64, String> {
    let mut t = 0.0;
    for p in s.trim().split(':') {
        t = t * 60.0 + p.trim().parse::<f64>().map_err(|_| format!("bad time '{s}' (use seconds or mm:ss)"))?;
    }
    Ok(t)
}


fn load(scene_path: &Path, scale: f64) -> Result<Prepared> {
    let t = Instant::now();
    let (scene, warnings) = Scene::load(scene_path)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let p = Prepared::build(&scene, scale)?;
    eprintln!("flits: prepared scene in {:.2}s", t.elapsed().as_secs_f64());
    Ok(p)
}

fn do_render(p: &Prepared, out: &Path, r: &RenderArgs) -> Result<()> {
    let o = flits::Options { workers: r.workers, encoder: r.encoder.clone(), quality: r.quality, scale: r.scale, compositor: r.gpu.clone(), keep_temp: r.keep_temp };
    flits::render_prepared(p, out, &o).map(|_| ())
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    p.with_file_name(format!("{stem}{suffix}"))
}

fn write_scene_and_render(scene: serde_json::Value, out: &Path, r: &RenderArgs) -> Result<()> {
    let scene_path = with_suffix(out, ".scene.json");
    std::fs::write(&scene_path, serde_json::to_string_pretty(&scene)?)?;
    eprintln!("flits: scene written to {} (edit it and run `flits render` to tweak)", scene_path.display());
    let p = load(&scene_path, r.scale)?;
    do_render(&p, out, r)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Render { scene, out, r, remote } => {
            let out = out.unwrap_or_else(|| scene.with_extension("mp4"));
            if remote {
                let rm = flits::remote::configured().ok_or_else(|| anyhow::anyhow!("no GPU server configured: run `flits remote http://host:7777 --token ...`"))?;
                rm.health().map_err(|e| anyhow::anyhow!("GPU server {} not reachable: {e}", rm.url))?;
                return rm.render(&scene, &out, &format!("scale={}&quality={}", r.scale, r.quality));
            }
            let p = load(&scene, r.scale)?;
            do_render(&p, &out, &r)
        }
        Cmd::Still { scene, at, out, scale } => {
            let p = load(&scene, scale)?;
            for f in render::stills(&p, &at, &out)? {
                println!("{}", f.display());
            }
            Ok(())
        }
        Cmd::Sheet { scene, out, count, cols, thumb } => {
            let (s, _) = Scene::load(&scene)?;
            let p = load(&scene, thumb as f64 / s.width as f64)?;
            render::sheet(&p, count, cols, &out)?;
            println!("{}", out.display());
            Ok(())
        }
        Cmd::Check { scene } => {
            let (s, warnings) = Scene::load(&scene)?;
            for w in &warnings {
                println!("warning: {w}");
            }
            let p = Prepared::build(&s, 0.25)?;
            println!("ok: {}x{} @ {} fps, {:.2}s = {} frames, {} audio track(s)", s.width, s.height, s.fps, p.duration, p.frames, p.audio.len());
            fn walk(n: &[layers::Node], depth: usize) {
                for n in n {
                    let kind = match &n.kind {
                        layers::Kind::Group => "group",
                        layers::Kind::Rect(_) => "rect",
                        layers::Kind::Image { .. } => "image",
                        layers::Kind::Video { .. } => "video",
                        layers::Kind::Text { .. } => "text",
                        layers::Kind::Captions(_) => "captions",
                        layers::Kind::Waveform { .. } => "waveform",
                    };
                    println!("{:indent$}{kind:<9} {:>8.2}s -> {:>8.2}s  box {:.0}x{:.0}", "", n.start, n.end, n.w, n.h, indent = depth * 2);
                    walk(&n.children, depth + 1);
                }
            }
            walk(&p.nodes, 1);
            Ok(())
        }
        Cmd::Transcribe { input, out, w } => {
            let out = out.unwrap_or_else(|| transcribe::default_json(&input));
            let t = Instant::now();
            transcribe::transcribe(&input, &out, &w.opts())?;
            eprintln!("flits: transcribed in {:.1}s", t.elapsed().as_secs_f64());
            for ext in ["json", "srt", "vtt", "txt"] {
                println!("{}", out.with_extension(ext).display());
            }
            Ok(())
        }
        Cmd::Captions { video, out, style, highlight, uppercase, position, transcript, w, r } => {
            let out = out.unwrap_or_else(|| with_suffix(&video, "-captions.mp4"));
            if out == video {
                bail!("output would overwrite the input");
            }
            let tr = transcribe::ensure(&video, transcript.as_deref(), &w.opts())?;
            let scene = templates::captioned_video(&templates::CaptionOpts { video: &video, transcript: &tr, style, highlight, uppercase, position })?;
            write_scene_and_render(scene, &out, &r)
        }
        Cmd::Podcast { audio, out, cover, title, subtitle, format, from, to, accent, style, transcript, w, r } => {
            let out = out.unwrap_or_else(|| with_suffix(&audio, "-video.mp4"));
            let tr = transcribe::ensure(&audio, transcript.as_deref(), &w.opts())?;
            let title = if title.is_empty() { audio.file_stem().and_then(|s| s.to_str()).unwrap_or("Podcast").to_string() } else { title };
            let scene = templates::podcast(&templates::PodcastOpts {
                audio: &audio,
                transcript: &tr,
                cover: cover.as_deref(),
                title,
                subtitle,
                format,
                from,
                to,
                accent,
                style,
            })
            .context("building podcast scene")?;
            write_scene_and_render(scene, &out, &r)
        }
        Cmd::Bench { scene, at, frames } => {
            let p = load(&scene, 1.0)?;
            if let Some(g) = flits::pick_gpu("auto", p.width)? {
                let mut tg = g.target(p.width, p.height);
                let mut vs = layers::VideoState::default();
                let mut yuv = Vec::new();
                let f0 = (at * p.fps) as i64;
                let t = Instant::now();
                for f in f0..f0 + frames {
                    layers::advance(&p, f, &mut vs)?;
                    let mut gp = gpu::GpuPainter::default();
                    layers::draw_frame(&p, f, &mut gp, &vs);
                    g.render(&mut tg, p.background, gp, &mut yuv)?;
                }
                println!("GPU {}: {:.2} ms/frame (incl. video decode + yuv readback)", g.name, t.elapsed().as_secs_f64() * 1000.0 / frames as f64);
            }
            let mut canvas = tiny_skia::Pixmap::new(p.width, p.height).unwrap();
            let mut vs = layers::VideoState::default();
            let f0 = (at * p.fps) as i64;
            layers::render_frame(&p, f0, &mut canvas, &mut vs)?;
            let t = Instant::now();
            for f in f0..f0 + frames {
                layers::render_frame(&p, f, &mut canvas, &mut vs)?;
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / frames as f64;
            let t = Instant::now();
            let mut yuv = Vec::new();
            for _ in 0..frames {
                render::rgba_to_yuv420(canvas.data(), p.width as usize, p.height as usize, &mut yuv);
            }
            let yms = t.elapsed().as_secs_f64() * 1000.0 / frames as f64;
            println!("{ms:.2} ms/frame compose, {yms:.2} ms/frame yuv");
            Ok(())
        }
        Cmd::Serve { port, token } => {
            let token = token.or_else(|| std::env::var("FLITS_TOKEN").ok()).unwrap_or_else(flits::remote::new_token);
            eprintln!("flits: token {token}");
            eprintln!("flits: on other machines run: flits remote http://<this-machine>:{port} --token {token}");
            flits::remote::serve(port, token)
        }
        Cmd::Remote { url, token } => {
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
        Cmd::Spec => {
            print!("{}", include_str!("../SPEC.md"));
            Ok(())
        }
    }
}
