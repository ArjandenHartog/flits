//! Commands that produce media.

use super::{load, with_suffix, write_scene_and_render, RenderArgs, WhisperArgs};
use anyhow::{anyhow, bail, Context, Result};
use flits::compose::captions::load_transcript;
use flits::templates::{self, cut, slideshow, starters};
use flits::{media, render, transcribe, Scene};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub fn render(scene: &Path, out: Option<PathBuf>, r: &RenderArgs, remote: bool) -> Result<()> {
    let out = out.unwrap_or_else(|| scene.with_extension("mp4"));
    if remote {
        let rm = flits::remote::configured()
            .ok_or_else(|| anyhow!("no GPU server configured: run `flits remote http://host:7777 --token ...`"))?;
        rm.health().map_err(|e| anyhow!("GPU server {} not reachable: {e}", rm.url))?;
        return rm.render(scene, &out, &format!("scale={}&quality={}", r.scale, r.quality));
    }
    let p = load(scene, r.scale)?;
    flits::render_prepared(&p, &out, &r.options()).map(|_| ())
}

pub fn still(scene: &Path, at: &[f64], out: &Path, scale: f64) -> Result<()> {
    let p = load(scene, scale)?;
    for f in render::stills(&p, flits::pick_gpu("auto")?, at, out)? {
        println!("{}", f.display());
    }
    Ok(())
}

pub fn sheet(scene: &Path, out: &Path, count: usize, cols: usize, thumb: u32) -> Result<()> {
    let (s, _) = Scene::load(scene)?;
    let p = load(scene, thumb as f64 / s.width as f64)?;
    render::sheet(&p, flits::pick_gpu("auto")?, count, cols, out)?;
    println!("{}", out.display());
    Ok(())
}

pub fn new(starter: Option<String>, out: Option<PathBuf>) -> Result<()> {
    let Some(name) = starter else {
        println!("starters (flits new <name> -o scene.json):");
        for (n, d) in starters::STARTERS {
            println!("  {n:<12} {d}");
        }
        return Ok(());
    };
    let scene = starters::starter(&name)?;
    let out = out.unwrap_or_else(|| PathBuf::from(format!("{name}.json")));
    if out.exists() {
        bail!("{} already exists", out.display());
    }
    std::fs::write(&out, serde_json::to_string_pretty(&scene)?)?;
    println!("{}  (render with: flits render {})", out.display(), out.display());
    Ok(())
}

pub fn slideshow(o: slideshow::SlideshowOpts, out: Option<PathBuf>, r: &RenderArgs) -> Result<()> {
    let out = out.unwrap_or_else(|| {
        let base =
            o.inputs.first().map(|p| if p.is_dir() { p.join("slideshow") } else { p.with_file_name("slideshow") }).unwrap_or_default();
        base.with_extension("mp4")
    });
    let scene = slideshow::slideshow(&o)?;
    write_scene_and_render(scene, &out, r)
}

#[allow(clippy::too_many_arguments)]
pub fn captions(
    video: &Path,
    out: Option<PathBuf>,
    style: String,
    highlight: String,
    uppercase: bool,
    position: f64,
    transcript: Option<PathBuf>,
    w: &WhisperArgs,
    r: &RenderArgs,
) -> Result<()> {
    let out = out.unwrap_or_else(|| with_suffix(video, "-captions.mp4"));
    if out == video {
        bail!("output would overwrite the input");
    }
    let tr = transcribe::ensure(video, transcript.as_deref(), &w.opts())?;
    let scene = templates::captioned_video(&templates::CaptionOpts { video, transcript: &tr, style, highlight, uppercase, position })?;
    write_scene_and_render(scene, &out, r)
}

#[allow(clippy::too_many_arguments)]
pub fn podcast(
    audio: &Path,
    out: Option<PathBuf>,
    cover: Option<PathBuf>,
    title: String,
    subtitle: Option<String>,
    format: String,
    from: f64,
    to: Option<f64>,
    accent: String,
    style: Option<String>,
    transcript: Option<PathBuf>,
    w: &WhisperArgs,
    r: &RenderArgs,
) -> Result<()> {
    let out = out.unwrap_or_else(|| with_suffix(audio, "-video.mp4"));
    let tr = transcribe::ensure(audio, transcript.as_deref(), &w.opts())?;
    let title = if title.is_empty() { audio.file_stem().and_then(|s| s.to_str()).unwrap_or("Podcast").to_string() } else { title };
    let scene = templates::podcast(&templates::PodcastOpts {
        audio,
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
    write_scene_and_render(scene, &out, r)
}

fn fmt_ts(t: f64) -> String {
    format!("{:02}:{:05.2}", (t / 60.0) as u64, t % 60.0)
}

#[allow(clippy::too_many_arguments)]
pub fn cut(
    input: &Path,
    out: Option<PathBuf>,
    threshold: f64,
    min: f64,
    keep: f64,
    fillers: bool,
    dry_run: bool,
    transcript: Option<PathBuf>,
    w: &WhisperArgs,
    r: &RenderArgs,
) -> Result<()> {
    let info = media::probe(input)?;
    if !info.has_audio {
        bail!("{} has no audio to detect silences in", input.display());
    }
    let words = if fillers { load_transcript(&transcribe::ensure(input, transcript.as_deref(), &w.opts())?)? } else { vec![] };
    let o = cut::CutOpts {
        threshold_db: threshold,
        min_silence: min,
        keep,
        fillers: if fillers { cut::DEFAULT_FILLERS.iter().map(|s| s.to_string()).collect() } else { vec![] },
    };
    let ranges = cut::plan(input, info.duration, &o, &words)?;
    let kept: f64 = ranges.iter().map(|(s, e)| e - s).sum();
    eprintln!(
        "flits: keeping {} of {} ({} removed, {} cuts)",
        fmt_ts(kept),
        fmt_ts(info.duration),
        fmt_ts(info.duration - kept),
        ranges.len().saturating_sub(1)
    );
    if dry_run {
        for (s, e) in &ranges {
            println!("keep {} - {}  ({:.2}s)", fmt_ts(*s), fmt_ts(*e), e - s);
        }
        return Ok(());
    }
    let default_ext = if info.has_video { "mp4" } else { input.extension().and_then(|e| e.to_str()).unwrap_or("m4a") };
    let out = out.unwrap_or_else(|| with_suffix(input, &format!("-cut.{default_ext}")));
    if out == input {
        bail!("output would overwrite the input");
    }
    let t = Instant::now();
    if !info.has_video {
        cut::cut_audio(input, &ranges, &out)?;
        eprintln!("flits: done in {:.1}s -> {}", t.elapsed().as_secs_f64(), out.display());
        return Ok(());
    }
    // the cut soundtrack stays next to the output: the written scene refers to it
    let audio = with_suffix(&out, ".audio.m4a");
    cut::cut_audio(input, &ranges, &audio)?;
    let scene = cut::cut_video_scene(input, &ranges, &audio)?;
    write_scene_and_render(scene, &out, r)
}

pub fn transcribe(inputs: &[PathBuf], out: Option<PathBuf>, outdir: Option<PathBuf>, w: &WhisperArgs) -> Result<()> {
    if out.is_some() && inputs.len() > 1 {
        bail!("-o works with one input; use --outdir for several");
    }
    let jobs: Vec<(PathBuf, PathBuf)> = inputs
        .iter()
        .map(|i| {
            let o = match (&out, &outdir) {
                (Some(o), _) => o.clone(),
                (None, Some(d)) => d.join(transcribe::default_json(i).file_name().unwrap()),
                (None, None) => transcribe::default_json(i),
            };
            (i.clone(), o)
        })
        .collect();
    if let Some(d) = &outdir {
        std::fs::create_dir_all(d)?;
    }
    let t = Instant::now();
    transcribe::transcribe_many(&jobs, &w.opts())?;
    eprintln!("flits: transcribed {} file(s) in {:.1}s", jobs.len(), t.elapsed().as_secs_f64());
    for (_, o) in &jobs {
        println!("{}", o.display());
    }
    Ok(())
}
