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
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::OnceLock;

/// Frames decoded ahead of the renderer.
const AHEAD: usize = 6;
/// Longest wait for one frame before a decoder counts as stalled.
const STALL_HW: std::time::Duration = std::time::Duration::from_secs(8);
const STALL_SW: std::time::Duration = std::time::Duration::from_secs(60);

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
    seek: f64,
    fps: f64,
    w: u32,
    h: u32,
    looped: bool,
    speed: f64,
    fmt: FrameFormat,
}

/// Streams decoded YUV 4:2:0 frames of a video, already scaled to the size it is drawn at.
pub struct VideoReader {
    child: Child,
    hw: Option<Hw>,
    args: SpawnArgs,
    frames: Receiver<Vec<u8>>,
    recycle: SyncSender<Vec<u8>>,
    /// the current frame: Y plane, U plane, V plane (see `fmt`)
    pub frame: Vec<u8>,
    pub fmt: FrameFormat,
    pub w: u32,
    pub h: u32,
    /// layer-local frame index of the frame currently in `frame` (first - 1 = none yet)
    pub cur: i64,
    first: i64,
    eof: bool,
}

impl VideoReader {
    /// Start decoding so that the first frame read corresponds to layer-local frame `first`.
    /// `high_bit`: deliver 10-bit samples for 10-bit/HDR sources (the consumer supports 16-bit planes).
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
    ) -> Result<VideoReader> {
        let mut seek = trim + first as f64 / fps * speed;
        if looped && info.duration > 0.0 {
            seek %= info.duration;
        }
        let mut fmt = info.color;
        fmt.high_bit = high_bit && (fmt.high_bit || fmt.is_hdr());
        let args = SpawnArgs { src: src.to_path_buf(), rotation: info.rotation, seek, fps, w, h, looped, speed, fmt };
        // reserve a hardware decoder slot, or decode in software when all are taken
        let hw = hw_backend().filter(|b| hw_ok(*b, info)).filter(|_| {
            let ok = HW_ACTIVE.fetch_add(1, Ordering::Relaxed) < hw_max();
            if !ok {
                HW_ACTIVE.fetch_sub(1, Ordering::Relaxed);
            }
            ok
        });
        let (child, frames, recycle) = Self::start(&args, hw)?;
        let size = fmt.frame_size(w, h);
        let mut frame = vec![0; size];
        // start as black (Y=16, U=V=128 in 8-bit; zeros are fine for 16-bit before the first frame)
        if !fmt.high_bit {
            frame[..(w * h) as usize].fill(16);
            frame[(w * h) as usize..].fill(128);
        }
        Ok(VideoReader { child, hw, args, frames, recycle, frame, fmt, w, h, cur: first - 1, first, eof: false })
    }

    /// Spawn ffmpeg (hardware or software decode) plus the read-ahead thread.
    fn start(a: &SpawnArgs, hw: Option<Hw>) -> Result<(Child, Receiver<Vec<u8>>, SyncSender<Vec<u8>>)> {
        let (w, h, fps) = (a.w, a.h, a.fps);
        let hb = a.fmt.high_bit;
        let pix = if hb { "yuv420p10le" } else { "yuv420p" };
        // keep the source colour encoding: only retime, resample + scale here, the GPU converts
        let retime = if (a.speed - 1.0).abs() > 1e-6 { format!("setpts=PTS/{},", a.speed) } else { String::new() };
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
                if hb {
                    format!("{retime}fps={fps},scale_cuda={sw}:{sh},hwdownload,format=p010le,format={pix}{rotate}")
                } else {
                    format!("{retime}fps={fps},scale_cuda={sw}:{sh}:format=yuv420p,hwdownload,format=yuv420p{rotate}")
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
                format!("{retime}fps={fps},scale_vaapi=w={sw}:h={sh}:format={sf},hwdownload,format={df},format={pix}{rotate}")
            }
            None => {
                cmd.args(["-threads", "2", "-filter_threads", "1"]);
                format!("{retime}fps={fps},scale={w}:{h}:flags=bicubic,format={pix}")
            }
        };
        if a.looped {
            cmd.args(["-stream_loop", "-1"]);
        }
        cmd.args(["-ss", &format!("{:.4}", a.seek)]).arg("-i").arg(&a.src);
        cmd.args(["-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-"]);
        if std::env::var_os("FLITS_DEBUG").is_some() {
            eprintln!("flits: decode {:?} {}", hw, a.src.display());
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
        Ok((child, frames, recycle))
    }

    pub fn first(&self) -> i64 {
        self.first
    }

    /// Source time (seconds) of the next frame this reader will deliver.
    pub fn next_source_time(&self) -> f64 {
        self.args.seek + (self.cur + 1 - self.first) as f64 * self.args.speed / self.args.fps
    }

    /// Same file, size, speed and bit depth: frames are interchangeable.
    pub fn compatible(&self, src: &Path, w: u32, h: u32, speed: f64, looped: bool) -> bool {
        self.args.src == src && self.w == w && self.h == h && (self.args.speed - speed).abs() < 1e-9 && !looped && !self.args.looped
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
        use std::sync::mpsc::RecvTimeoutError;
        while self.cur < idx && !self.eof {
            // a decoder that stops producing must never hang the render
            let wait = if self.hw.is_some() { STALL_HW } else { STALL_SW };
            match self.frames.recv_timeout(wait) {
                Ok(buf) => {
                    let old = std::mem::replace(&mut self.frame, buf);
                    let _ = self.recycle.try_send(old);
                    self.cur += 1;
                }
                Err(e) if self.hw.is_some() => {
                    // the hardware path stalled or produced nothing: continue in software
                    let why = if matches!(e, RecvTimeoutError::Timeout) { "stalled" } else { "failed" };
                    if self.cur < self.first || matches!(e, RecvTimeoutError::Timeout) {
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

    /// Restart decoding in software from the next frame this reader still has to deliver.
    fn restart_in_software(&mut self) -> Result<()> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        HW_ACTIVE.fetch_sub(1, Ordering::Relaxed);
        self.hw = None;
        if self.cur >= self.first {
            // resume right after the last delivered frame
            self.args.seek += (self.cur + 1 - self.first) as f64 * self.args.speed / self.args.fps;
            self.first = self.cur + 1;
        }
        let (child, frames, recycle) = Self::start(&self.args, None)?;
        self.child = child;
        self.frames = frames;
        self.recycle = recycle;
        Ok(())
    }
}

impl Drop for VideoReader {
    fn drop(&mut self) {
        if self.hw.is_some() {
            HW_ACTIVE.fetch_sub(1, Ordering::Relaxed);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
