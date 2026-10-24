//! Streaming decoded video frames out of ffmpeg as planar YUV, decoded ahead on a
//! background thread. Colour conversion and HDR tone mapping happen on the GPU.
//!
//! Decoding uses the GPU's video engine (NVDEC via CUDA, or VAAPI on Linux) when the
//! codec allows it — 4K HEVC phone footage decodes ~5x faster that way — and falls back
//! to software decoding automatically. `FLITS_HWDEC=off|cuda|vaapi` overrides the choice.

use super::colorspace::FrameFormat;
use super::{ffmpeg, MediaInfo};
use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Frames decoded ahead of the renderer.
const AHEAD: usize = 6;
/// Longest wait for one frame before a decoder counts as stalled.
const STALL_HW: Duration = Duration::from_secs(8);
const STALL_SW: Duration = Duration::from_secs(60);
/// A hardware decoder that has no first frame after this long is raced by a software one. Its
/// ffmpeg first has to create a CUDA context, which takes seconds while the GPU is busy
/// compositing and encoding; a software decoder usually starts well within a second.
/// `FLITS_HWDEC_START` (ms) overrides it.
fn start_hw() -> Duration {
    static D: OnceLock<Duration> = OnceLock::new();
    *D.get_or_init(|| {
        let ms = std::env::var("FLITS_HWDEC_START").ok().and_then(|v| v.parse().ok()).unwrap_or(1500);
        Duration::from_millis(ms)
    })
}

/// Part of the picture to decode: a `w`x`h` rect of the displayed (rotated) source, scaled to
/// `dw`x`dh`. Its position may change at given source times: layers that show the same footage
/// with different framings (a reel's shots) share one plan, so one decoder carries on through all
/// of them instead of being restarted at every change of framing.
#[derive(Debug, PartialEq)]
pub struct CropPlan {
    pub w: u32,
    pub h: u32,
    pub dw: u32,
    pub dh: u32,
    /// (source time from which it applies, x, y), by time; the first applies before its time too
    pub steps: Vec<(f64, u32, u32)>,
}

impl CropPlan {
    /// ffmpeg expression for a coordinate of output frame `k` of a decoder whose frame `k` shows
    /// source time `seek + k * speed / fps` (a constant when the position doesn't change).
    ///
    /// Frames are told apart by their time stamp `t`, which the frame rate conversion sets to
    /// `k / fps`: the frame counter `n` doesn't count the copy of the first frame ffmpeg adds when
    /// the first decoded frame lands on the second output slot.
    fn expr(&self, seek: f64, speed: f64, fps: f64, coord: impl Fn(u32, u32) -> u32) -> String {
        // output frame from which each step applies; earlier steps are folded into the first
        let mut parts: Vec<(i64, u32)> = Vec::new();
        for &(t, x, y) in &self.steps {
            let k = ((t - seek) * fps / speed).round() as i64;
            let v = coord(x, y);
            match parts.last_mut() {
                Some(last) if k <= 0 => *last = (0, v),
                Some(last) if last.1 == v => {}
                _ => parts.push((k.max(0), v)),
            }
        }
        let mut e = parts.last().map(|p| p.1).unwrap_or(0).to_string();
        for k in (1..parts.len()).rev() {
            e = format!("if(lt(t,{:.6}),{},{e})", (parts[k].0 as f64 - 0.5) / fps, parts[k - 1].1);
        }
        e
    }
}

/// The rect `(x, y, w, h)` of the displayed picture (`dw`x`dh`, rotated by `rotation`) in the
/// stored (unrotated) frame, where hardware frames are cropped.
pub fn stored_rect(rotation: i64, (dw, dh): (u32, u32), (x, y, w, h): (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    match rotation {
        // shown rotated a quarter turn counter-clockwise (transpose=cclock)
        90 => (dh - y - h, x, h, w),
        180 => (dw - x - w, dh - y - h, w, h),
        // a quarter turn clockwise (transpose=clock)
        270 => (y, dw - x - w, h, w),
        _ => (x, y, w, h),
    }
}

/// set when ffmpeg can't crop CUDA frames (older versions): crop after decoding in software then
static CUDA_CROP_BAD: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hw {
    Cuda,
    Vaapi,
}

/// hardware decoders currently open (each holds a few hundred MB of video memory)
static HW_ACTIVE: AtomicUsize = AtomicUsize::new(0);
/// set after hardware decoding failed where software worked: stop trying
static HW_BROKEN: AtomicBool = AtomicBool::new(false);

fn hw_backend() -> Option<Hw> {
    static B: OnceLock<Option<Hw>> = OnceLock::new();
    *B.get_or_init(|| match std::env::var("FLITS_HWDEC").unwrap_or_default().as_str() {
        "off" | "0" | "none" => None,
        "cuda" | "nvdec" => Some(Hw::Cuda),
        "vaapi" => Some(Hw::Vaapi),
        _ => {
            let nvidia = std::process::Command::new("nvidia-smi").arg("-L").output().map(|o| o.status.success()).unwrap_or(false);
            if nvidia {
                Some(Hw::Cuda)
            } else if cfg!(target_os = "linux") && Path::new(&super::encode::vaapi_device()).exists() {
                Some(Hw::Vaapi)
            } else {
                None
            }
        }
    })
}

/// Concurrent hardware decoders. 4K HEVC decoders need ~0.5 GB of video memory each and
/// share it with compositing and NVENC; too many make them stall. Scales with VRAM.
fn hw_max() -> usize {
    static M: OnceLock<usize> = OnceLock::new();
    *M.get_or_init(|| {
        if let Some(v) = std::env::var("FLITS_HWDEC_MAX").ok().and_then(|v| v.parse().ok()) {
            return v;
        }
        let vram_mb = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next()?.trim().parse::<f64>().ok());
        match vram_mb {
            Some(mb) => (((mb / 1024.0) - 2.0).floor() as usize).clamp(1, 8),
            None => 2,
        }
    })
}

/// Can the hardware decoder take this stream? (10-bit H.264 isn't supported by NVDEC/VAAPI)
fn hw_ok(hw: Hw, info: &MediaInfo) -> bool {
    if HW_BROKEN.load(Ordering::Relaxed) {
        return false;
    }
    match info.codec.as_str() {
        "hevc" | "vp9" => true,
        "av1" => hw == Hw::Cuda,
        "h264" => !info.color.high_bit,
        _ => false,
    }
}

#[derive(Clone)]
struct SpawnArgs {
    src: std::path::PathBuf,
    /// display rotation in degrees (0, 90, 180, 270), applied after hardware scaling
    rotation: i64,
    /// displayed (rotated) size of the source
    disp: (u32, u32),
    seek: f64,
    fps: f64,
    w: u32,
    h: u32,
    looped: bool,
    speed: f64,
    fmt: FrameFormat,
    /// decode only part of the picture (see `cropped` for whether it is applied)
    plan: Option<Arc<CropPlan>>,
    cropped: bool,
}

impl SpawnArgs {
    /// Size of the frames delivered.
    fn out_size(&self) -> (u32, u32) {
        match (&self.plan, self.cropped) {
            (Some(p), true) => (p.dw, p.dh),
            _ => (self.w, self.h),
        }
    }
}

/// A running ffmpeg decoder and the thread reading its frames.
struct Proc {
    child: Child,
    frames: Receiver<Vec<u8>>,
    recycle: SyncSender<Vec<u8>>,
}

impl Proc {
    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Streams decoded YUV 4:2:0 frames of a video, already scaled to the size it is drawn at.
pub struct VideoReader {
    proc: Proc,
    hw: Option<Hw>,
    args: SpawnArgs,
    /// the current process delivered a frame
    started: bool,
    /// the current frame: Y plane, U plane, V plane (see `fmt`)
    pub frame: Vec<u8>,
    pub fmt: FrameFormat,
    pub w: u32,
    pub h: u32,
    /// frames hold only the part of the picture the crop plan names (else the whole picture)
    pub cropped: bool,
    /// layer-local frame index of the frame currently in `frame` (first - 1 = none yet)
    pub cur: i64,
    first: i64,
    eof: bool,
}

impl VideoReader {
    /// Start decoding so that the first frame read corresponds to layer-local frame `first`.
    /// `high_bit`: deliver 10-bit samples for 10-bit/HDR sources (the consumer supports 16-bit planes).
    /// `plan`: decode only that part of the picture (the frames then are `plan.dw`x`plan.dh`,
    /// unless [`VideoReader::cropped`] says the decoder couldn't crop).
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        src: &Path,
        info: &MediaInfo,
        trim: f64,
        first: i64,
        fps: f64,
        w: u32,
        h: u32,
        looped: bool,
        speed: f64,
        high_bit: bool,
        plan: Option<Arc<CropPlan>>,
    ) -> Result<VideoReader> {
        let mut seek = trim + first as f64 / fps * speed;
        if looped && info.duration > 0.0 {
            seek %= info.duration;
        }
        let mut fmt = info.color;
        fmt.high_bit = high_bit && (fmt.high_bit || fmt.is_hdr());
        // reserve a hardware decoder slot, or decode in software when all are taken
        let hw = hw_backend().filter(|b| hw_ok(*b, info)).filter(|_| {
            let ok = HW_ACTIVE.fetch_add(1, Ordering::Relaxed) < hw_max();
            if !ok {
                HW_ACTIVE.fetch_sub(1, Ordering::Relaxed);
            }
            ok
        });
        // VAAPI frames are never cropped (untested there); CUDA ones when this ffmpeg can
        let cropped = plan.is_some()
            && match hw {
                None => true,
                Some(Hw::Cuda) => !CUDA_CROP_BAD.load(Ordering::Relaxed),
                Some(Hw::Vaapi) => false,
            };
        let args = SpawnArgs {
            src: src.to_path_buf(),
            rotation: info.rotation,
            disp: (info.width, info.height),
            seek,
            fps,
            w,
            h,
            looped,
            speed,
            fmt,
            plan,
            cropped,
        };
        let proc = match Self::start(&args, hw) {
            Ok(p) => p,
            Err(e) => {
                if hw.is_some() {
                    HW_ACTIVE.fetch_sub(1, Ordering::Relaxed);
                }
                return Err(e);
            }
        };
        let (w, h) = args.out_size();
        let size = fmt.frame_size(w, h);
        let mut frame = vec![0; size];
        // start as black (Y=16, U=V=128 in 8-bit; zeros are fine for 16-bit before the first frame)
        if !fmt.high_bit {
            frame[..(w * h) as usize].fill(16);
            frame[(w * h) as usize..].fill(128);
        }
        Ok(VideoReader { proc, hw, args, started: false, frame, fmt, w, h, cropped, cur: first - 1, first, eof: false })
    }

    /// Spawn ffmpeg (hardware or software decode) plus the read-ahead thread.
    fn start(a: &SpawnArgs, hw: Option<Hw>) -> Result<Proc> {
        let (w, h) = a.out_size();
        let fps = a.fps;
        let hb = a.fmt.high_bit;
        let pix = if hb { "yuv420p10le" } else { "yuv420p" };
        // keep the source colour encoding: only retime, resample + scale here, the GPU converts
        let retime = if (a.speed - 1.0).abs() > 1e-6 { format!("setpts=PTS/{},", a.speed) } else { String::new() };
        // Output frame k shows the source frame nearest seek + k * speed / fps. ffmpeg's accurate
        // seek drops every frame before the seek point, so the first slot got a copy of the second
        // whenever the nearest frame lay just before it: a frozen frame wherever a decoder started
        // (every chunk). Seek to the keyframe only and keep frames from half a frame early instead.
        let keep = format!("select='gte(t,{:.6})',", -0.5 / fps);
        // only the part of the picture that is shown: cut out before scaling
        let crop = |rotated: bool| match (&a.plan, a.cropped) {
            (Some(p), true) => {
                let (rot, size) = (if rotated { a.rotation } else { 0 }, a.disp);
                let (_, _, cw, ch) = stored_rect(rot, size, (0, 0, p.w, p.h));
                let x = p.expr(a.seek, a.speed, fps, |x, y| stored_rect(rot, size, (x, y, p.w, p.h)).0);
                let y = p.expr(a.seek, a.speed, fps, |x, y| stored_rect(rot, size, (x, y, p.w, p.h)).1);
                format!("crop=w={cw}:h={ch}:x='{x}':y='{y}',")
            }
            _ => String::new(),
        };
        let mut cmd = ffmpeg();
        // Hardware frames can't go through ffmpeg's automatic rotation: scale to the stored
        // (unrotated) size on the GPU, download, then rotate the small frame in software.
        let (sw, sh) = if a.rotation % 180 == 90 { (h, w) } else { (w, h) };
        let rotate = match a.rotation {
            90 => ",transpose=cclock",
            180 => ",hflip,vflip",
            270 => ",transpose=clock",
            _ => "",
        };
        let vf = match hw {
            Some(Hw::Cuda) => {
                cmd.args(["-noautorotate", "-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
                let crop = crop(true);
                if hb {
                    format!("{retime}{keep}fps={fps},{crop}scale_cuda={sw}:{sh},hwdownload,format=p010le,format={pix}{rotate}")
                } else {
                    format!("{retime}{keep}fps={fps},{crop}scale_cuda={sw}:{sh}:format=yuv420p,hwdownload,format=yuv420p{rotate}")
                }
            }
            Some(Hw::Vaapi) => {
                cmd.args([
                    "-noautorotate",
                    "-hwaccel",
                    "vaapi",
                    "-hwaccel_device",
                    &super::encode::vaapi_device(),
                    "-hwaccel_output_format",
                    "vaapi",
                ]);
                let (sf, df) = if hb { ("p010", "p010le") } else { ("nv12", "nv12") };
                format!("{retime}{keep}fps={fps},scale_vaapi=w={sw}:h={sh}:format={sf},hwdownload,format={df},format={pix}{rotate}")
            }
            None => {
                // software frames arrive rotated already
                cmd.args(["-threads", "2", "-filter_threads", "1"]);
                format!("{retime}{keep}fps={fps},{}scale={w}:{h}:flags=bicubic,format={pix}", crop(false))
            }
        };
        if a.looped {
            cmd.args(["-stream_loop", "-1"]);
        }
        cmd.args(["-ss", &format!("{:.4}", a.seek), "-noaccurate_seek"]).arg("-i").arg(&a.src);
        cmd.args(["-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-"]);
        if std::env::var_os("FLITS_DEBUG").is_some() {
            eprintln!("flits: decode {:?} {} from {:.3} -vf {vf}", hw, a.src.display(), a.seek);
            cmd.stderr(Stdio::inherit());
        } else {
            cmd.stderr(Stdio::null());
        }
        cmd.stdout(Stdio::piped()).stdin(Stdio::null());
        let mut child = cmd.spawn().context("starting ffmpeg decoder")?;
        let mut out = child.stdout.take().unwrap();

        let size = a.fmt.frame_size(w, h);
        let (tx, frames) = sync_channel::<Vec<u8>>(AHEAD);
        let (recycle, free) = sync_channel::<Vec<u8>>(AHEAD + 2);
        std::thread::Builder::new()
            .name("flits-video".into())
            .spawn(move || loop {
                let mut buf = free.try_recv().unwrap_or_else(|_| vec![0; size]);
                if out.read_exact(&mut buf).is_err() || tx.send(buf).is_err() {
                    return; // EOF, or the reader was dropped
                }
            })
            .context("starting decoder thread")?;
        Ok(Proc { child, frames, recycle })
    }

    pub fn first(&self) -> i64 {
        self.first
    }

    /// Source time (seconds) of the next frame this reader will deliver.
    pub fn next_source_time(&self) -> f64 {
        self.args.seek + (self.cur + 1 - self.first) as f64 * self.args.speed / self.args.fps
    }

    /// Same file, size, speed, bit depth and crop plan: frames are interchangeable.
    pub fn compatible(&self, src: &Path, w: u32, h: u32, speed: f64, looped: bool, plan: Option<&Arc<CropPlan>>) -> bool {
        let same_plan = match (&self.args.plan, plan) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        self.args.src == src
            && self.args.w == w
            && self.args.h == h
            && same_plan
            && (self.args.speed - speed).abs() < 1e-9
            && !looped
            && !self.args.looped
    }

    /// Re-express frame indices for a layer that starts at `trim` in the same source,
    /// so the decoder can simply continue instead of being restarted.
    pub fn rebase(&mut self, trim: f64) {
        let d = ((self.args.seek - trim) * self.args.fps / self.args.speed).round() as i64 - self.first;
        self.cur += d;
        self.first += d;
        self.args.seek = trim + self.first as f64 * self.args.speed / self.args.fps;
    }

    /// Advance to layer-local frame `idx` (must be >= current). Holds the last frame at EOF.
    pub fn seek_forward(&mut self, idx: i64) -> Result<()> {
        while self.cur < idx && !self.eof {
            // a decoder that stops producing must never hang the render
            let wait = match (self.hw, self.started) {
                (Some(_), false) => start_hw(),
                (Some(_), true) => STALL_HW,
                (None, _) => STALL_SW,
            };
            match self.proc.frames.recv_timeout(wait) {
                Ok(buf) => self.take(buf),
                Err(RecvTimeoutError::Timeout) if self.hw.is_some() && !self.started => {
                    // slow to start, not stalled: take whichever decoder delivers first
                    if let Some(buf) = self.race_software()? {
                        self.take(buf);
                    }
                }
                Err(e) if self.hw.is_some() => {
                    // the hardware path stalled or produced nothing: continue in software
                    let timeout = matches!(e, RecvTimeoutError::Timeout);
                    if !self.started && !timeout && self.cropped {
                        // most likely an ffmpeg that can't crop CUDA frames: from now on crop in software
                        CUDA_CROP_BAD.store(true, Ordering::Relaxed);
                    }
                    if !self.started || timeout {
                        let why = if timeout { "stalled" } else { "failed" };
                        eprintln!("flits: hardware decoding {why} on {}, continuing in software", self.args.src.display());
                        self.restart_in_software()?;
                    } else {
                        self.eof = true; // a normal end of the clip
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    eprintln!("flits: decoder stalled on {}; holding the last frame", self.args.src.display());
                    self.eof = true;
                }
                Err(RecvTimeoutError::Disconnected) => self.eof = true, // keep showing the last frame (black if none)
            }
        }
        Ok(())
    }

    fn take(&mut self, buf: Vec<u8>) {
        let old = std::mem::replace(&mut self.frame, buf);
        let _ = self.proc.recycle.try_send(old);
        self.cur += 1;
        self.started = true;
    }

    /// Arguments for a decoder that starts at the next frame this reader still has to deliver.
    fn resume_args(&self) -> (SpawnArgs, i64) {
        let mut a = self.args.clone();
        let mut first = self.first;
        if self.cur >= self.first {
            a.seek += (self.cur + 1 - self.first) as f64 * a.speed / a.fps;
            first = self.cur + 1;
        }
        (a, first)
    }

    /// Continue with a software decoder started by `resume_args`.
    fn adopt_software(&mut self, proc: Proc, args: SpawnArgs, first: i64) {
        let old = std::mem::replace(&mut self.proc, proc);
        old.kill();
        if self.hw.take().is_some() {
            HW_ACTIVE.fetch_sub(1, Ordering::Relaxed);
        }
        self.args = args;
        self.first = first;
        self.started = false;
    }

    /// Restart decoding in software from the next frame this reader still has to deliver.
    fn restart_in_software(&mut self) -> Result<()> {
        let (args, first) = self.resume_args();
        let proc = Self::start(&args, None)?;
        self.adopt_software(proc, args, first);
        Ok(())
    }

    /// The hardware decoder is still starting up: run a software decoder alongside and keep
    /// whichever delivers a frame first. `None` when neither does in time.
    fn race_software(&mut self) -> Result<Option<Vec<u8>>> {
        let (args, first) = self.resume_args();
        let sw = Self::start(&args, None)?;
        let deadline = Instant::now() + STALL_SW;
        let mut hw_alive = true;
        while Instant::now() < deadline {
            if hw_alive {
                match self.proc.frames.try_recv() {
                    Ok(buf) => {
                        sw.kill();
                        return Ok(Some(buf));
                    }
                    Err(TryRecvError::Disconnected) => hw_alive = false,
                    Err(TryRecvError::Empty) => {}
                }
            }
            match sw.frames.recv_timeout(Duration::from_millis(10)) {
                Ok(buf) => {
                    if std::env::var_os("FLITS_DEBUG").is_some() {
                        eprintln!("flits: hardware decoder slow to start on {}, decoding in software", self.args.src.display());
                    }
                    self.adopt_software(sw, args, first);
                    return Ok(Some(buf));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    // software can't decode it either: leave it to the hardware decoder
                    sw.kill();
                    return Ok(None);
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        sw.kill();
        Ok(None)
    }
}

impl Drop for VideoReader {
    fn drop(&mut self) {
        if self.hw.is_some() {
            HW_ACTIVE.fetch_sub(1, Ordering::Relaxed);
        }
        let _ = self.proc.child.kill();
        let _ = self.proc.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(steps: Vec<(f64, u32, u32)>) -> CropPlan {
        CropPlan { w: 608, h: 1080, dw: 608, dh: 1080, steps }
    }

    #[test]
    fn crop_position_switches_at_the_right_frame() {
        let p = plan(vec![(10.0, 100, 0), (12.0, 400, 0), (15.0, 400, 0), (20.0, 0, 0)]);
        // a decoder at 25 fps starting at 11 s: frame 25 is 12 s, frame 225 is 20 s
        assert_eq!(p.expr(11.0, 1.0, 25.0, |x, _| x), "if(lt(t,0.980000),100,if(lt(t,8.980000),400,0))");
        assert_eq!(p.expr(11.0, 1.0, 25.0, |_, y| y), "0");
        // started after the second switch: earlier steps fold into the first
        assert_eq!(p.expr(13.0, 1.0, 25.0, |x, _| x), "if(lt(t,6.980000),400,0)");
        assert_eq!(p.expr(30.0, 1.0, 25.0, |x, _| x), "0");
        // before the first step its position applies too
        assert_eq!(plan(vec![(10.0, 100, 0)]).expr(0.0, 1.0, 25.0, |x, _| x), "100");
        // at double speed frame 25 is two seconds in
        assert_eq!(p.expr(10.0, 2.0, 25.0, |x, _| x), "if(lt(t,0.980000),100,if(lt(t,4.980000),400,0))");
    }

    #[test]
    fn rects_map_to_the_stored_frame() {
        // a 1080x1920 picture shown from a 1920x1080 frame
        let disp = (1080, 1920);
        assert_eq!(stored_rect(0, disp, (10, 20, 100, 200)), (10, 20, 100, 200));
        // the stored rect must hold the same pixels: check corners through the rotation
        for rot in [90, 180, 270] {
            let (x, y, w, h) = stored_rect(rot, disp, (10, 20, 100, 200));
            let stored = if rot == 180 { disp } else { (disp.1, disp.0) };
            assert!(x + w <= stored.0 && y + h <= stored.1, "{rot}");
            assert_eq!(if rot == 180 { (w, h) } else { (h, w) }, (100, 200));
        }
        assert_eq!(stored_rect(90, disp, (0, 0, 100, 200)), (1920 - 200, 0, 200, 100));
        assert_eq!(stored_rect(270, disp, (0, 0, 100, 200)), (0, 1080 - 100, 200, 100));
        assert_eq!(stored_rect(180, disp, (0, 0, 100, 200)), (980, 1720, 100, 200));
    }
}
