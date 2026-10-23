//! Stills and contact sheets for quick visual checks.

use super::FrameRenderer;
use crate::compose::Prepared;
use crate::paint::gpu::Gpu;
use crate::scene::color::Rgba;
use crate::text::{self, Align, FontDb, TextStyle};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

/// Render single frames at the given times to PNG.
pub fn stills(p: &Prepared, gpu: Option<Arc<Gpu>>, times: &[f64], out: &Path) -> Result<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = if times.len() == 1 {
        vec![out.to_path_buf()]
    } else {
        let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("still");
        let dir = out.parent().unwrap_or(Path::new("."));
        times.iter().map(|t| dir.join(format!("{stem}-{t:.2}s.png"))).collect()
    };
    times
        .par_iter()
        .zip(paths.par_iter())
        .map(|(t, path)| -> Result<()> {
            let f = ((t * p.fps).round() as i64).clamp(0, (p.frames - 1).max(0));
            let canvas = FrameRenderer::new(p, gpu.clone())?.rgba(p, f)?;
            canvas.save_png(path).with_context(|| format!("writing {}", path.display()))?;
            Ok(())
        })
        .collect::<Result<()>>()?;
    Ok(paths)
}

/// Contact sheet: `count` evenly spaced frames in a grid with timestamps.
pub fn sheet(p: &Prepared, gpu: Option<Arc<Gpu>>, count: usize, cols: usize, out: &Path) -> Result<()> {
    let count = count.max(1);
    let cols = cols.clamp(1, count);
    let rows = count.div_ceil(cols);
    let (tw, th) = (p.width, p.height);
    let label_h = 34u32;
    let gap = 8u32;
    let sw = cols as u32 * (tw + gap) + gap;
    let sh = rows as u32 * (th + label_h + gap) + gap;
    let frames: Vec<i64> = (0..count).map(|i| ((i as f64 + 0.5) / count as f64 * p.frames as f64) as i64).collect();
    let tiles: Vec<Pixmap> =
        frames.par_iter().map(|&f| -> Result<Pixmap> { FrameRenderer::new(p, gpu.clone())?.rgba(p, f) }).collect::<Result<_>>()?;
    let mut sheet = Pixmap::new(sw, sh).unwrap();
    sheet.fill(Rgba::rgba8(24, 24, 27, 1.0).skia(1.0));
    let fonts = FontDb::new(Default::default());
    let style = TextStyle {
        font: fonts.get(None, None)?,
        size: 22.0,
        color: Rgba::rgba8(230, 230, 235, 1.0),
        line_height: 1.2,
        letter_spacing: 0.0,
        align: Align::Left,
        max_width: None,
        stroke: None,
        shadow: None,
        bg: None,
    };
    for (i, tile) in tiles.iter().enumerate() {
        let (c, r) = ((i % cols) as u32, (i / cols) as u32);
        let x = gap + c * (tw + gap);
        let y = gap + r * (th + label_h + gap);
        sheet.draw_pixmap(x as i32, (y + label_h) as i32, tile.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        let t = frames[i] as f64 / p.fps;
        let label = format!("{:02}:{:05.2}  (frame {})", (t / 60.0) as u64, t % 60.0, frames[i]);
        let lay = text::layout(&style, &[label]);
        let spr = text::render(&style, &lay, None, None)?;
        sheet.draw_pixmap(
            0,
            0,
            spr.pm.as_ref(),
            &PixmapPaint::default(),
            spr.place(Transform::from_translate(x as f32, y as f32 + 2.0)),
            None,
        );
    }
    sheet.save_png(out).with_context(|| format!("writing {}", out.display()))?;
    Ok(())
}
