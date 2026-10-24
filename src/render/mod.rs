//! The render pipeline: the timeline is split into chunks, workers pull chunks,
//! composite every frame (GPU or CPU), feed an encoder each, and the segments are
//! joined without re-encoding while the soundtrack is mixed in parallel.

pub mod frame;
pub mod preview;
pub mod profile;
pub mod yuv;

pub use frame::FrameRenderer;
pub use preview::{sheet, sheet_frames, stills};
pub use yuv::rgba_to_yuv420;

use crate::compose::Prepared;
use crate::media::encode::{self, Encoder, EncoderProc, Settings};
use crate::media::{self, audio};
use crate::paint::gpu::Gpu;
use anyhow::{anyhow, bail, Result};
use std::io::Write;
use std::path::Path;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub struct RenderOpts {
    pub workers: usize,
    pub encode: Settings,
    pub keep_temp: bool,
    pub gpu: Option<Arc<Gpu>>,
    /// no progress output
    pub quiet: bool,
}

pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.0);
    if s >= 60.0 {
        format!("{}m{:02}s", (s / 60.0) as u64, (s % 60.0) as u64)
    } else {
        format!("{s:.1}s")
    }
}

/// Contiguous frame ranges; more chunks than workers so slow parts (video clips)
/// don't leave the other workers idle at the end.
fn chunks(total: i64, workers: usize) -> Vec<(i64, i64)> {
    let n = (workers * 4).min((total / 60).max(1) as usize).max(workers.min(total as usize)).max(1);
    (0..n).map(|i| (total * i as i64 / n as i64, total * (i as i64 + 1) / n as i64)).filter(|(a, b)| b > a).collect()
}

/// Counts encoder sessions that are open: hardware encoders refuse to open more than a few
/// (NVENC on GeForce cards: 8), and a session stays open until its process has exited.
struct Sessions {
    free: Mutex<usize>,
    cv: Condvar,
}

impl Sessions {
    fn new(n: usize) -> Arc<Sessions> {
        Arc::new(Sessions { free: Mutex::new(n), cv: Condvar::new() })
    }

    fn acquire(&self) {
        let mut f = self.free.lock().unwrap();
        while *f == 0 {
            f = self.cv.wait(f).unwrap();
        }
        *f -= 1;
    }

    fn release(&self) {
        *self.free.lock().unwrap() += 1;
        self.cv.notify_one();
    }
}

/// Start the encoder for a chunk.
fn start_encoder(p: &Prepared, opts: &RenderOpts, in_w: u32, seg: &Path) -> Result<EncoderProc> {
    profile::time(profile::Stage::EncoderSetup, || encode::spawn_encoder(&opts.encode, in_w, p.width, p.height, p.fps, seg))
}

/// Render one chunk into an encoder that is already running. Returns the encoder's shutdown
/// (flushing the last frames, writing the file) as a background job, so the worker can go on
/// with its next chunk meanwhile.
fn render_chunk(
    p: &Prepared,
    fr: &mut FrameRenderer,
    opts: &RenderOpts,
    enc: EncoderProc,
    (a, b): (i64, i64),
    done: &AtomicI64,
    sessions: &Arc<Sessions>,
) -> Result<std::thread::JoinHandle<Result<()>>> {
    let EncoderProc { child, mut input } = enc;
    profile::time(profile::Stage::EncoderSetup, || sessions.acquire());
    fr.reset_videos();
    let mut yuv = Vec::new();
    let mut res = Ok(());
    for f in a..b {
        if let Err(e) = fr.yuv(p, f, &mut yuv) {
            res = Err(e.context(format!("frame {f}")));
            break;
        }
        if let Err(e) = profile::time(profile::Stage::Encode, || input.write_all(&yuv)) {
            res = Err(anyhow!("encoder stopped: {e}"));
            break;
        }
        done.fetch_add(1, Ordering::Relaxed);
    }
    drop(input);
    let settings = opts.encode;
    if res.is_err() {
        let e = finish_encoder(child, res, settings).unwrap_err();
        sessions.release();
        return Err(e);
    }
    let sessions = sessions.clone();
    Ok(std::thread::spawn(move || {
        let r = finish_encoder(child, Ok(()), settings);
        sessions.release();
        r
    }))
}

/// Wait for an encoder whose input is closed; its own error message wins if it failed.
fn finish_encoder(child: Child, mut res: Result<()>, settings: Settings) -> Result<()> {
    let out = child.wait_with_output()?;
    if !out.status.success() && (res.is_ok() || stdin_closed(&res)) {
        res = Err(anyhow!("encoder {} failed: {}{}", settings.codec_name(), ffmpeg_error(&out.stderr), encoder_hint(settings.encoder)));
    }
    res
}

/// The encoder closed its input early: its own error message says why.
fn stdin_closed(res: &Result<()>) -> bool {
    res.as_ref().is_err_and(|e| e.to_string().starts_with("encoder stopped"))
}

/// The lines of ffmpeg's stderr that explain a failure (its error lines, else the last few lines).
fn ffmpeg_error(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let errors: Vec<&str> = lines.iter().copied().filter(|l| l.contains("rror") || l.contains("failed")).take(6).collect();
    let pick = if errors.is_empty() { lines[lines.len().saturating_sub(5)..].to_vec() } else { errors };
    pick.join("\n  ")
}

fn encoder_hint(e: Encoder) -> &'static str {
    match e {
        Encoder::Qsv => {
            "\n(QSV needs Intel's oneVPL GPU runtime (Tiger Lake and newer) or the legacy Media SDK runtime; on Linux --encoder vaapi works on any Intel GPU since Broadwell)"
        }
        Encoder::Vaapi => "\n(VAAPI needs read access to the render node: FLITS_VAAPI_DEVICE, default /dev/dri/renderD128, and the `render` group)",
        Encoder::Nvenc => "\n(NVENC needs an NVIDIA GPU and driver; `flits doctor` shows what works)",
        Encoder::Cpu | Encoder::Null => "",
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
    let chunks = chunks(total, opts.workers.max(1));
    let workers = opts.workers.clamp(1, chunks.len());
    if !opts.quiet {
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
            opts.encode.describe()
        );
    }
    profile::reset();

    let done = AtomicI64::new(0);
    let finished = AtomicBool::new(false);
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let sessions = Sessions::new(opts.encode.max_sessions().min(usize::MAX / 2));
    let finishing: Mutex<Vec<std::thread::JoinHandle<Result<()>>>> = Mutex::new(Vec::new());
    let audio_path = tmp.join("audio.m4a");
    let null = opts.encode.encoder == Encoder::Null;

    let result: Result<bool> = std::thread::scope(|s| {
        let audio_job = s.spawn(|| if null { Ok(false) } else { audio::mix(p, &audio_path) });
        let progress = s.spawn(|| {
            let mut last = Instant::now();
            while !finished.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                if opts.quiet || last.elapsed() < Duration::from_millis(500) {
                    continue;
                }
                last = Instant::now();
                let d = done.load(Ordering::Relaxed);
                let fps = d as f64 / t0.elapsed().as_secs_f64().max(1e-3);
                let eta = (total - d) as f64 / fps.max(1e-3);
                eprint!("\r  {d}/{total} frames  {fps:.0} fps  eta {}      ", fmt_time(eta));
                let _ = std::io::stderr().flush();
            }
        });
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                s.spawn(|| -> Result<()> {
                    let mut fr = FrameRenderer::new(p, opts.gpu.clone())?;
                    let in_w = fr.yuv_width();
                    let claim = || {
                        let ci = next.fetch_add(1, Ordering::Relaxed);
                        (ci < chunks.len() && !failed.load(Ordering::Relaxed))
                            .then(|| (ci, start_encoder(p, opts, in_w, &tmp.join(format!("seg{ci:04}.mp4")))))
                    };
                    let mut cur = claim();
                    while let Some((ci, enc)) = cur.take() {
                        let enc = enc.inspect_err(|_| failed.store(true, Ordering::Relaxed))?;
                        // start the next chunk's encoder now: its process start-up overlaps this chunk
                        cur = claim();
                        match render_chunk(p, &mut fr, opts, enc, chunks[ci], &done, &sessions) {
                            Ok(job) => finishing.lock().unwrap().push(job),
                            Err(e) => {
                                failed.store(true, Ordering::Relaxed);
                                if let Some((_, Ok(mut next))) = cur {
                                    let _ = next.child.kill();
                                }
                                return Err(e);
                            }
                        }
                    }
                    Ok(())
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
        for job in std::mem::take(&mut *finishing.lock().unwrap()) {
            match job.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    first_err.get_or_insert(e);
                }
                Err(_) => {
                    first_err.get_or_insert(anyhow!("encoder thread panicked"));
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
    if !opts.quiet {
        eprintln!();
    }
    let cleanup = || {
        if !opts.keep_temp {
            let _ = std::fs::remove_dir_all(&tmp);
        }
    };
    let has_audio = match result {
        Ok(a) => a,
        Err(e) => {
            cleanup();
            return Err(e);
        }
    };
    let render_time = t0.elapsed().as_secs_f64();

    if !null {
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
        let joined = media::run(&mut cmd, "join");
        cleanup();
        joined?;
    } else {
        cleanup();
    }
    if !opts.quiet {
        let el = t0.elapsed().as_secs_f64();
        let target = if null { "(discarded)".to_string() } else { out.display().to_string() };
        eprintln!("flits: done in {} ({:.0} fps, render {}) -> {target}", fmt_time(el), total as f64 / render_time, fmt_time(render_time));
    }
    if profile::enabled() {
        eprint!("{}", profile::report(workers, render_time));
    }
    Ok(())
}
