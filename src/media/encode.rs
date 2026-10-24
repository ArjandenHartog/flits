//! Video encoders (NVENC, QSV, VAAPI, x264/x265/SVT-AV1) fed with raw YUV frames.

use super::ffmpeg;
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Where the encoding happens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Encoder {
    Nvenc,
    Qsv,
    Vaapi,
    /// software (libx264 / libx265 / libsvtav1)
    Cpu,
    /// discard frames (benchmarking the renderer alone)
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

/// Encoder speed/quality trade-off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Speed {
    Fast,
    Balanced,
    Quality,
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub encoder: Encoder,
    pub codec: Codec,
    pub speed: Speed,
    /// CRF/CQ-like quality, lower = better (18..30)
    pub quality: u32,
}

impl Codec {
    pub fn parse(s: &str) -> Result<Codec> {
        Ok(match s {
            "h264" | "avc" => Codec::H264,
            "hevc" | "h265" => Codec::Hevc,
            "av1" => Codec::Av1,
            o => bail!("unknown codec '{o}' (h264, hevc, av1)"),
        })
    }
}

impl Speed {
    pub fn parse(s: &str) -> Result<Speed> {
        Ok(match s {
            "fast" => Speed::Fast,
            "balanced" | "default" => Speed::Balanced,
            "quality" | "slow" => Speed::Quality,
            o => bail!("unknown speed '{o}' (fast, balanced, quality)"),
        })
    }
}

impl Settings {
    /// ffmpeg codec name.
    pub fn codec_name(&self) -> &'static str {
        use Codec::*;
        use Encoder::*;
        match (self.encoder, self.codec) {
            (Nvenc, H264) => "h264_nvenc",
            (Nvenc, Hevc) => "hevc_nvenc",
            (Nvenc, Av1) => "av1_nvenc",
            (Qsv, H264) => "h264_qsv",
            (Qsv, Hevc) => "hevc_qsv",
            (Qsv, Av1) => "av1_qsv",
            (Vaapi, H264) => "h264_vaapi",
            (Vaapi, Hevc) => "hevc_vaapi",
            (Vaapi, Av1) => "av1_vaapi",
            (Cpu, H264) => "libx264",
            (Cpu, Hevc) => "libx265",
            (Cpu, Av1) => "libsvtav1",
            (Null, _) => "null",
        }
    }

    pub fn describe(&self) -> String {
        let hw = match self.encoder {
            Encoder::Nvenc => "NVIDIA GPU",
            Encoder::Qsv => "Intel GPU",
            Encoder::Vaapi => "VAAPI GPU",
            Encoder::Cpu => "CPU",
            Encoder::Null => "discarded",
        };
        format!("{} ({hw})", self.codec_name())
    }

    /// Encoders that share the GPU with rendering, or are single engines, gain nothing from many sessions.
    pub fn max_sessions(&self) -> usize {
        match self.encoder {
            Encoder::Nvenc => 8,
            Encoder::Qsv | Encoder::Vaapi => 4,
            Encoder::Cpu | Encoder::Null => usize::MAX,
        }
    }
}

/// The VAAPI render node: `FLITS_VAAPI_DEVICE`, else the first /dev/dri/renderD* this user can open.
pub fn vaapi_device() -> String {
    if let Ok(d) = std::env::var("FLITS_VAAPI_DEVICE") {
        return d;
    }
    let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri")
        .map(|rd| {
            rd.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("renderD"))).collect()
        })
        .unwrap_or_default();
    nodes.sort();
    nodes
        .iter()
        .find(|p| std::fs::OpenOptions::new().read(true).write(true).open(p).is_ok())
        .or(nodes.first())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/dev/dri/renderD128".into())
}

pub fn encoder_works(codec: &str) -> bool {
    let mut c = ffmpeg();
    let vaapi = codec.ends_with("_vaapi");
    if vaapi {
        c.args(["-vaapi_device", &vaapi_device()]);
    }
    c.args(["-f", "lavfi", "-i", "color=c=black:s=256x256:d=0.2"]);
    if vaapi {
        c.args(["-vf", "format=nv12,hwupload"]);
    }
    c.args(["-c:v", codec, "-f", "null", "-"]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// Resolve "auto" | "nvenc" | "qsv" | "vaapi" | "cpu"/"x264" | "null" for the given codec.
pub fn pick(choice: &str, codec: Codec, speed: Speed, quality: u32) -> Result<Settings> {
    let mk = |encoder| Settings { encoder, codec, speed, quality };
    let explicit = match choice {
        "nvenc" | "gpu" => Some(Encoder::Nvenc),
        "qsv" => Some(Encoder::Qsv),
        "vaapi" => Some(Encoder::Vaapi),
        "cpu" | "x264" | "x265" | "software" => Some(Encoder::Cpu),
        "null" => Some(Encoder::Null),
        "auto" => None,
        o => bail!("unknown encoder '{o}' (auto, nvenc, qsv, vaapi, cpu, null)"),
    };
    if let Some(e) = explicit {
        return Ok(mk(e));
    }
    let mut order = vec![Encoder::Nvenc];
    if cfg!(target_os = "linux") {
        order.push(Encoder::Vaapi);
    }
    order.push(Encoder::Qsv);
    for e in order {
        if encoder_works(mk(e).codec_name()) {
            return Ok(mk(e));
        }
    }
    Ok(mk(Encoder::Cpu))
}

/// Colour tags of the frames flits produces.
const BT709: [&str; 8] = ["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709", "-color_range", "tv"];

/// A running encoder: write raw frames into `input`, drop it, then wait for `child`.
pub struct EncoderProc {
    pub child: Child,
    pub input: Box<dyn Write + Send>,
}

/// Bytes the frame pipe buffers. Windows pipes default to a few KB, so every 3 MB frame took
/// hundreds of round trips through the kernel and both processes kept waking each other up.
#[cfg(windows)]
const PIPE_BYTES: u32 = 16 << 20;

/// Start `cmd` with a pipe on stdin (a large one on Windows).
fn spawn_piped(mut cmd: Command) -> std::io::Result<EncoderProc> {
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::System::Pipes::CreatePipe;
        let (mut r, mut w) = (std::ptr::null_mut(), std::ptr::null_mut());
        // not inheritable: Command duplicates the read end into the child as an inheritable handle
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 0,
        };
        if unsafe { CreatePipe(&mut r, &mut w, &sa, PIPE_BYTES) } != 0 {
            let (r, w) = unsafe { (OwnedHandle::from_raw_handle(r), OwnedHandle::from_raw_handle(w)) };
            cmd.stdin(Stdio::from(r));
            let child = cmd.spawn()?;
            // the child has its own copy now; ours must close or ffmpeg never sees the end of input
            drop(cmd);
            return Ok(EncoderProc { child, input: Box::new(std::fs::File::from(w)) });
        }
    }
    cmd.stdin(Stdio::piped());
    let mut child = cmd.spawn()?;
    let input = Box::new(child.stdin.take().expect("piped stdin"));
    Ok(EncoderProc { child, input })
}

/// ffmpeg process that reads raw yuv420p frames on stdin and writes a video file.
/// `in_w` may be wider than `w` (padded GPU frames); the extra columns are cropped.
pub fn spawn_encoder(s: &Settings, in_w: u32, w: u32, h: u32, fps: f64, out: &Path) -> Result<EncoderProc> {
    let mut cmd = ffmpeg();
    if s.encoder == Encoder::Vaapi {
        cmd.args(["-vaapi_device", &vaapi_device()]);
    }
    // the frames are BT.709 limited range: say so on the input too, or ffmpeg >= 7.1 takes the
    // untagged input for BT.601 and converts it to the BT.709 tagged on the output (shifting colours)
    cmd.args(["-y", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-s", &format!("{in_w}x{h}"), "-r", &format!("{fps}")]);
    cmd.args(BT709);
    cmd.args(["-i", "-"]);
    if s.encoder == Encoder::Null {
        cmd.args(["-f", "null", "-"]);
        return spawn_piped(cmd).context("starting ffmpeg");
    }
    let mut vf = Vec::new();
    if in_w != w {
        vf.push(format!("crop={w}:{h}:0:0"));
    }
    if s.encoder == Encoder::Vaapi {
        vf.push("format=nv12,hwupload".into());
    }
    if !vf.is_empty() {
        cmd.args(["-vf", &vf.join(",")]);
    }
    let q = s.quality.to_string();
    let codec = s.codec_name();
    cmd.args(["-c:v", codec]);
    match s.encoder {
        Encoder::Nvenc => {
            let preset = match s.speed {
                Speed::Fast => "p2", // as fast as p1 here, slightly better quality
                Speed::Balanced => "p4",
                Speed::Quality => "p6",
            };
            cmd.args(["-preset", preset, "-tune", "hq", "-rc", "vbr", "-cq", &q, "-b:v", "0"]);
            if s.codec == Codec::H264 {
                cmd.args(["-profile:v", "high"]);
            }
        }
        Encoder::Qsv => {
            let preset = match s.speed {
                Speed::Fast => "veryfast",
                Speed::Balanced => "fast",
                Speed::Quality => "slow",
            };
            cmd.args(["-preset", preset, "-global_quality", &q]);
        }
        Encoder::Vaapi => {
            // constant QP (the rate control every VAAPI driver has): QP = CRF + 4 gives about the size
            // and quality of x264 at that CRF (measured on Intel iHD, HD 630)
            cmd.args(["-rc_mode", "CQP", "-qp", &(s.quality + 4).min(51).to_string()]);
        }
        Encoder::Cpu => {
            match s.codec {
                Codec::Av1 => {
                    let preset = match s.speed {
                        Speed::Fast => "10",
                        Speed::Balanced => "8",
                        Speed::Quality => "5",
                    };
                    cmd.args(["-preset", preset, "-crf", &(s.quality + 8).to_string()]);
                }
                _ => {
                    let preset = match s.speed {
                        Speed::Fast => "ultrafast",
                        Speed::Balanced => "veryfast",
                        Speed::Quality => "medium",
                    };
                    cmd.args(["-preset", preset, "-crf", &q, "-threads", "4"]);
                }
            }
            if s.codec == Codec::Hevc {
                cmd.args(["-x265-params", "log-level=error"]);
            }
        }
        Encoder::Null => unreachable!(),
    }
    if s.codec == Codec::Hevc {
        cmd.args(["-tag:v", "hvc1"]); // plays in QuickTime/iOS
    }
    if s.encoder != Encoder::Vaapi {
        cmd.args(["-pix_fmt", "yuv420p"]);
    }
    cmd.args(BT709);
    cmd.arg(out);
    spawn_piped(cmd).context("starting ffmpeg encoder")
}
