//! Parallel rendering: the timeline is split into chunks, every chunk gets its own
//! compositor thread + hardware encoder, and the pieces are joined without re-encoding.

use crate::color::Rgba;
use crate::ffmpeg::{self, Encoder};
use crate::gpu::{Gpu, GpuPainter};
use crate::layers::{advance, draw_frame, render_frame, Prepared, VideoState};
use crate::text::{self, Align, FontDb, TextStyle};
use anyhow::{anyhow, bail, Context, Result};
use rayon::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_skia::{Pixmap, PixmapPaint, Transform};

pub struct RenderOpts {
    pub workers: usize,
    pub encoder: Encoder,
    pub quality: u32,
    pub keep_temp: bool,
    pub gpu: Option<Arc<Gpu>>,
}

/// RGBA (opaque) -> planar YUV 4:2:0, BT.709 limited range.
pub fn rgba_to_yuv420(src: &[u8], w: usize, h: usize, out: &mut Vec<u8>) {
    let (cw, ch) = (w / 2, h / 2);
    out.resize(w * h + 2 * cw * ch, 0);
    let (yp, uv) = out.split_at_mut(w * h);
    let (up, vp) = uv.split_at_mut(cw * ch);
    for y in 0..h {
        let row = &src[y * w * 4..(y + 1) * w * 4];
        let yrow = &mut yp[y * w..(y + 1) * w];
        for x in 0..w {
            let (r, g, b) = (row[x * 4] as i32, row[x * 4 + 1] as i32, row[x * 4 + 2] as i32);
            yrow[x] = (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8;
        }
    }
    for cy in 0..ch {
        let r0 = &src[(cy * 2) * w * 4..(cy * 2 + 1) * w * 4];
        let r1 = &src[(cy * 2 + 1) * w * 4..(cy * 2 + 2) * w * 4];
        for cx in 0..cw {
            let i = cx * 8;
            let r = r0[i] as i32 + r0[i + 4] as i32 + r1[i] as i32 + r1[i + 4] as i32;
            let g = r0[i + 1] as i32 + r0[i + 5] as i32 + r1[i + 1] as i32 + r1[i + 5] as i32;
            let b = r0[i + 2] as i32 + r0[i + 6] as i32 + r1[i + 2] as i32 + r1[i + 6] as i32;
            up[cy * cw + cx] = (((-26 * r - 87 * g + 113 * b + 512) >> 10) + 128).clamp(0, 255) as u8;
            vp[cy * cw + cx] = (((112 * r - 102 * g - 10 * b + 512) >> 10) + 128).clamp(0, 255) as u8;
        }
    }
}

fn fmt_time(s: f64) -> String {
    let s = s.max(0.0);
    if s >= 60.0 {
        format!("{}m{:02}s", (s / 60.0) as u64, (s % 60.0) as u64)
    } else {
        format!("{s:.1}s")
    }
}

fn mix_audio(p: &Prepared, out: &Path) -> Result<bool> {
    if p.audio.is_empty() {
        return Ok(false);
    }
    let mut cmd = ffmpeg::ffmpeg();
    cmd.arg("-y");
    let mut filters = Vec::new();
    for (i, a) in p.audio.iter().enumerate() {
        if a.looped {
            cmd.args(["-stream_loop", "-1"]);
        }
        cmd.args(["-ss", &format!("{:.3}", a.trim), "-t", &format!("{:.3}", a.dur)]).arg("-i").arg(&a.path);
        let mut f = format!("[{i}:a]aresample=48000,aformat=channel_layouts=stereo,volume={:.3}", a.volume);
        if a.fade_in > 0.0 {
            f += &format!(",afade=t=in:st=0:d={:.3}", a.fade_in);
        }
        if a.fade_out > 0.0 {
            f += &format!(",afade=t=out:st={:.3}:d={:.3}", (a.dur - a.fade_out).max(0.0), a.fade_out);
        }
        let ms = (a.start * 1000.0).round() as i64;
        if ms > 0 {
            f += &format!(",adelay={ms}:all=1");
        }
        f += &format!("[a{i}]");
        filters.push(f);
    }
    let ins: String = (0..p.audio.len()).map(|i| format!("[a{i}]")).collect();
    filters.push(format!(
        "{ins}amix=inputs={}:normalize=0:duration=longest,apad,atrim=0:{:.3}[out]",
        p.audio.len(),
        p.duration
    ));
    cmd.args(["-filter_complex", &filters.join(";"), "-map", "[out]", "-c:a", "aac", "-b:a", "192k"]).arg(out);
    ffmpeg::run(&mut cmd, "audio mix")?;
    Ok(true)
}

pub fn render(p: &Prepared, out: &Path, opts: &RenderOpts) -> Result<()> {
    let t0 = Instant::now();
    let total = p.frames;
    if total <= 0 {
        bail!("nothing to render (duration is 0)");
    }
    let tmp = ffmpeg::tmp_dir_for(out);
    std::fs::create_dir_all(&tmp)?;
    let workers = opts.workers.clamp(1, total.max(1) as usize);
    // split frames into contiguous chunks
    let chunks: Vec<(i64, i64)> = (0..workers)
        .map(|i| (total * i as i64 / workers as i64, total * (i as i64 + 1) / workers as i64))
        .filter(|(a, b)| b > a)
        .collect();
    eprintln!(
        "flits: {}x{} @ {} fps, {} frames ({}), {} workers, compositor {}, encoder {}",
        p.width,
        p.height,
        p.fps,
        total,
        fmt_time(p.duration),
        chunks.len(),
        opts.gpu.as_ref().map(|g| format!("GPU {}", g.name)).unwrap_or_else(|| "CPU".into()),
        opts.encoder.name()
    );

    let done = Arc::new(AtomicI64::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let audio_path = tmp.join("audio.m4a");

    let result: Result<bool> = std::thread::scope(|s| {
        let audio_job = s.spawn(|| mix_audio(p, &audio_path));
        let progress = {
            let done = done.clone();
            let finished = finished.clone();
            s.spawn(move || {
                let mut last = Instant::now();
                while !finished.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(100));
                    if last.elapsed() < Duration::from_millis(500) {
                        continue;
                    }
                    last = Instant::now();
                    let d = done.load(Ordering::Relaxed);
                    let el = t0.elapsed().as_secs_f64();
                    let fps = d as f64 / el.max(1e-3);
                    let eta = (total - d) as f64 / fps.max(1e-3);
                    eprint!("\r  {d}/{total} frames  {fps:.0} fps  eta {}      ", fmt_time(eta));
                    let _ = std::io::stderr().flush();
                }
            })
        };
        let handles: Vec<_> = chunks
            .iter()
            .enumerate()
            .map(|(ci, &(a, b))| {
                let seg = tmp.join(format!("seg{ci:03}.mp4"));
                let done = done.clone();
                s.spawn(move || -> Result<()> {
                    let mut enc = ffmpeg::spawn_encoder(opts.encoder, p.width, p.height, p.fps, opts.quality, &seg)?;
                    let mut stdin = enc.stdin.take().unwrap();
                    let mut canvas = Pixmap::new(p.width, p.height).ok_or_else(|| anyhow!("bad output size"))?;
                    let mut vs = VideoState::default();
                    let mut yuv = Vec::new();
                    let mut target = opts.gpu.as_ref().map(|g| g.target(p.width, p.height));
                    let mut res = Ok(());
                    for f in a..b {
                        let step = match (&opts.gpu, &mut target) {
                            (Some(g), Some(tg)) => advance(p, f, &mut vs).and_then(|_| {
                                let mut gp = GpuPainter::default();
                                draw_frame(p, f, &mut gp, &vs);
                                g.render(tg, p.background, gp, &mut yuv)
                            }),
                            _ => render_frame(p, f, &mut canvas, &mut vs).map(|_| rgba_to_yuv420(canvas.data(), p.width as usize, p.height as usize, &mut yuv)),
                        };
                        if let Err(e) = step {
                            res = Err(e.context(format!("frame {f}")));
                            break;
                        }
                        if let Err(e) = stdin.write_all(&yuv) {
                            res = Err(anyhow!("encoder stopped: {e}"));
                            break;
                        }
                        done.fetch_add(1, Ordering::Relaxed);
                    }
                    drop(stdin);
                    let out = enc.wait_with_output()?;
                    if !out.status.success() {
                        bail!("encoder failed: {}", String::from_utf8_lossy(&out.stderr));
                    }
                    res
                })
            })
            .collect();
        let mut first_err = None;
        for h in handles {
            match h.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    first_err.get_or_insert(e);
                }
                Err(_) => {
                    first_err.get_or_insert(anyhow!("render thread panicked"));
                }
            }
        }
        finished.store(true, Ordering::Relaxed);
        let _ = progress.join();
        let has_audio = audio_job.join().map_err(|_| anyhow!("audio thread panicked"))??;
        match first_err {
            Some(e) => Err(e),
            None => Ok(has_audio),
        }
    });
    eprintln!();
    let has_audio = match result {
        Ok(a) => a,
        Err(e) => {
            if !opts.keep_temp {
                let _ = std::fs::remove_dir_all(&tmp);
            }
            return Err(e);
        }
    };
    let render_time = t0.elapsed().as_secs_f64();

    // join segments (stream copy) + audio
    let list = tmp.join("segments.txt");
    let body: String = (0..chunks.len()).map(|i| format!("file 'seg{i:03}.mp4'\n")).collect();
    std::fs::write(&list, body)?;
    let mut cmd = ffmpeg::ffmpeg();
    cmd.args(["-y", "-f", "concat", "-safe", "0"]).arg("-i").arg(&list);
    if has_audio {
        cmd.arg("-i").arg(&audio_path).args(["-map", "0:v", "-map", "1:a"]);
    }
    cmd.args(["-c", "copy", "-movflags", "+faststart"]).arg(out);
    ffmpeg::run(&mut cmd, "join")?;
    if !opts.keep_temp {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    let el = t0.elapsed().as_secs_f64();
    eprintln!(
        "flits: done in {} ({:.0} fps, render {}) -> {}",
        fmt_time(el),
        total as f64 / render_time,
        fmt_time(render_time),
        out.display()
    );
    Ok(())
}

/// Render single frames at the given times to PNG.
pub fn stills(p: &Prepared, times: &[f64], out: &Path) -> Result<Vec<PathBuf>> {
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
            let mut canvas = Pixmap::new(p.width, p.height).unwrap();
            let mut vs = VideoState::default();
            let f = ((t * p.fps).round() as i64).clamp(0, (p.frames - 1).max(0));
            render_frame(p, f, &mut canvas, &mut vs)?;
            canvas.save_png(path).with_context(|| format!("writing {}", path.display()))?;
            Ok(())
        })
        .collect::<Result<()>>()?;
    Ok(paths)
}

/// Contact sheet: `count` evenly spaced frames in a grid with timestamps.
pub fn sheet(p: &Prepared, count: usize, cols: usize, out: &Path) -> Result<()> {
    let count = count.max(1);
    let cols = cols.clamp(1, count);
    let rows = count.div_ceil(cols);
    let (tw, th) = (p.width, p.height);
    let label_h = 34u32;
    let gap = 8u32;
    let sw = cols as u32 * (tw + gap) + gap;
    let sh = rows as u32 * (th + label_h + gap) + gap;
    let frames: Vec<i64> = (0..count).map(|i| ((i as f64 + 0.5) / count as f64 * p.frames as f64) as i64).collect();
    let tiles: Vec<Pixmap> = frames
        .par_iter()
        .map(|&f| -> Result<Pixmap> {
            let mut c = Pixmap::new(tw, th).unwrap();
            let mut vs = VideoState::default();
            render_frame(p, f, &mut c, &mut vs)?;
            Ok(c)
        })
        .collect::<Result<_>>()?;
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
        sheet.draw_pixmap(0, 0, spr.pm.as_ref(), &PixmapPaint::default(), spr.place(Transform::from_translate(x as f32, y as f32 + 2.0)), None);
    }
    sheet.save_png(out).with_context(|| format!("writing {}", out.display()))?;
    Ok(())
}
