//! # flits
//!
//! Bliksemsnelle video- en podcastrenderer. Een scene (JSON) beschrijft lagen
//! (beeld, video, tekst, ondertitels, audiogolf, vormen) met animaties; flits
//! rendert die op de GPU (wgpu), codeert parallel met hardware-encoders en
//! plakt de stukken zonder hercodering aan elkaar.
//!
//! ```no_run
//! let stats = flits::render_file("scene.json".as_ref(), "out.mp4".as_ref(), &flits::Options::default())?;
//! println!("{} frames in {:.1}s", stats.frames, stats.seconds);
//! # anyhow::Ok(())
//! ```

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub mod compose;
pub mod gfx;
pub mod media;
pub mod paint;
pub mod remote;
pub mod render;
pub mod scene;
pub mod templates;
pub mod text;
pub mod transcribe;

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

pub use compose::Prepared;
pub use scene::Scene;

/// Rendering options for the high-level API.
#[derive(Clone, Debug)]
pub struct Options {
    /// parallel workers (None = automatic)
    pub workers: Option<usize>,
    /// "auto" | "nvenc" | "qsv" | "vaapi" | "x264"
    pub encoder: String,
    /// CRF/CQ quality, lower is better (18..28)
    pub quality: u32,
    /// render scale (0.5 = half resolution preview)
    pub scale: f64,
    /// compositor: "auto" | "gpu" | "cpu"
    pub compositor: String,
    pub keep_temp: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { workers: None, encoder: "auto".into(), quality: 21, scale: 1.0, compositor: "auto".into(), keep_temp: false }
    }
}

pub struct Stats {
    pub frames: i64,
    pub seconds: f64,
}

/// Open the GPU compositor if possible ("auto" silently falls back to CPU).
pub fn pick_gpu(choice: &str) -> Result<Option<Arc<paint::gpu::Gpu>>> {
    if choice == "cpu" || choice == "off" {
        return Ok(None);
    }
    match paint::gpu::Gpu::new() {
        Ok(g) => Ok(Some(Arc::new(g))),
        Err(e) if choice == "auto" => {
            eprintln!("flits: GPU not available ({e:#}), using CPU");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

pub fn default_workers(enc: media::Encoder, gpu: bool) -> usize {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let w = if gpu { (n * 3 / 4).max(2) } else { n.max(1) };
    match enc {
        media::Encoder::Nvenc => w.min(8),
        media::Encoder::Vaapi | media::Encoder::Qsv => w.min(4),
        _ => w,
    }
}

/// Render an already prepared scene.
pub fn render_prepared(p: &Prepared, out: &Path, o: &Options) -> Result<Stats> {
    let t = std::time::Instant::now();
    let encoder = media::encode::pick_encoder(&o.encoder)?;
    let gpu = pick_gpu(&o.compositor)?;
    let workers = o.workers.unwrap_or_else(|| default_workers(encoder, gpu.is_some()));
    render::render(p, out, &render::RenderOpts { workers, encoder, quality: o.quality, keep_temp: o.keep_temp, gpu })?;
    Ok(Stats { frames: p.frames, seconds: t.elapsed().as_secs_f64() })
}

/// Load a scene JSON file and render it to `out`.
pub fn render_file(scene: &Path, out: &Path, o: &Options) -> Result<Stats> {
    let (s, warnings) = Scene::load(scene)?;
    for w in warnings {
        eprintln!("warning: {w}");
    }
    let p = Prepared::build(&s, o.scale)?;
    render_prepared(&p, out, o)
}

/// Render a scene given as a JSON value; relative paths resolve against `base_dir`.
pub fn render_json(scene: serde_json::Value, base_dir: &Path, out: &Path, o: &Options) -> Result<Stats> {
    let (s, warnings) = Scene::from_value(scene, base_dir.to_path_buf())?;
    for w in warnings {
        eprintln!("warning: {w}");
    }
    let p = Prepared::build(&s, o.scale)?;
    render_prepared(&p, out, o)
}
