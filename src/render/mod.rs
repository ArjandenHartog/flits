//! The render pipeline: the timeline is split into chunks, workers pull chunks,
//! composite every frame (GPU or CPU), feed a hardware encoder each, and the
//! segments are joined without re-encoding.

pub mod preview;
pub mod yuv;

pub use preview::{sheet, stills};
pub use yuv::rgba_to_yuv420;

use crate::compose::{advance, draw_frame, render_frame, Prepared, VideoState};
use crate::media::{self, audio, encode, Encoder};
use crate::paint::gpu::{Gpu, GpuPainter};
use anyhow::{anyhow, bail, Result};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_skia::Pixmap;

pub struct RenderOpts {
    pub workers: usize,
    pub encoder: Encoder,
    pub quality: u32,
    pub keep_temp: bool,
    pub gpu: Option<Arc<Gpu>>,
}

fn fmt_time(s: f64) -> String {
    let s = s.max(0.0);
    if s >= 60.0 {
        format!("{}m{:02}s", (s / 60.0) as u64, (s % 60.0) as u64)
    } else {
        format!("{s:.1}s")
    }
}

pub fn render(p: &Prepared, out: &Path, opts: &RenderOpts) -> Result<()> {
    let t0 = Instant::now();
    let total = p.frames;
    if total <= 0 {
        bail!("nothing to render (duration is 0)");
    }
    let tmp = media::process::tmp_dir_for(out);
    std::fs::create_dir_all(&tmp)?;
    let workers = opts.workers.clamp(1, total.max(1) as usize);
    // more chunks than workers: workers pull the next chunk when done, so slow parts
    // (e.g. video clips) don't leave the other workers idle
    let n_chunks = (workers * 4).min((total / 60).max(1) as usize).max(workers.min(total as usize));
    let chunks: Vec<(i64, i64)> = (0..n_chunks)
        .map(|i| (total * i as i64 / n_chunks as i64, total * (i as i64 + 1) / n_chunks as i64))
        .filter(|(a, b)| b > a)
        .collect();
    let workers = workers.min(chunks.len());
    eprintln!(
        "flits: {}x{} @ {} fps, {} frames ({}), {} workers / {} chunks, compositor {}, encoder {}",
        p.width,
        p.height,
        p.fps,
        total,
        fmt_time(p.duration),
        workers,
        chunks.len(),
        opts.gpu.as_ref().map(|g| format!("GPU {}", g.name)).unwrap_or_else(|| "CPU".into()),
        opts.encoder.name()
    );

    let done = Arc::new(AtomicI64::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let audio_path = tmp.join("audio.m4a");

    let result: Result<bool> = std::thread::scope(|s| {
        let audio_job = s.spawn(|| audio::mix(p, &audio_path));
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
        let next = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let done = done.clone();
                let next = next.clone();
                let failed = failed.clone();
                let chunks = &chunks;
                let tmp = &tmp;
                s.spawn(move || -> Result<()> {
                    let mut canvas = Pixmap::new(p.width, p.height).ok_or_else(|| anyhow!("bad output size"))?;
                    let mut yuv = Vec::new();
                    let mut target = opts.gpu.as_ref().map(|g| g.target(p.width, p.height));
                    loop {
                        let ci = next.fetch_add(1, Ordering::Relaxed);
                        if ci >= chunks.len() || failed.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        let (a, b) = chunks[ci];
                        let seg = tmp.join(format!("seg{ci:04}.mp4"));
                        let mut enc = encode::spawn_encoder(opts.encoder, p.width, p.height, p.fps, opts.quality, &seg)?;
                        let mut stdin = enc.stdin.take().unwrap();
                        let mut vs = VideoState::default();
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
                        if res.is_ok() && !out.status.success() {
                            res = Err(anyhow!("encoder failed: {}", String::from_utf8_lossy(&out.stderr)));
                        }
                        if res.is_err() {
                            failed.store(true, Ordering::Relaxed);
                            return res;
                        }
                    }
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
    let body: String = (0..chunks.len()).map(|i| format!("file 'seg{i:04}.mp4'\n")).collect();
    std::fs::write(&list, body)?;
    let mut cmd = media::ffmpeg();
    cmd.args(["-y", "-f", "concat", "-safe", "0"]).arg("-i").arg(&list);
    if has_audio {
        cmd.arg("-i").arg(&audio_path).args(["-map", "0:v", "-map", "1:a"]);
    }
    cmd.args(["-c", "copy", "-movflags", "+faststart"]).arg(out);
    media::run(&mut cmd, "join")?;
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
