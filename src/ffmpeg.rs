//! Everything that talks to ffmpeg/ffprobe: probing, decoding video frames and audio, encoders.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::OnceLock;

pub fn ffmpeg() -> Command {
    let mut c = Command::new(std::env::var("FLITS_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()));
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    c
}

#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub fps: f64,
    pub has_video: bool,
    pub has_audio: bool,
    pub hdr: bool,
}

pub fn probe(path: &Path) -> Result<MediaInfo> {
    if !path.exists() {
        bail!("file not found: {}", path.display());
    }
    let out = Command::new(std::env::var("FLITS_FFPROBE").unwrap_or_else(|_| "ffprobe".into()))
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .context("running ffprobe (is ffmpeg installed and on PATH?)")?;
    if !out.status.success() {
        bail!("ffprobe failed on {}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
    }
    let v: Value = serde_json::from_slice(&out.stdout)?;
    let mut info = MediaInfo::default();
    if let Some(d) = v["format"]["duration"].as_str().and_then(|s| s.parse().ok()) {
        info.duration = d;
    }
    for s in v["streams"].as_array().cloned().unwrap_or_default() {
        match s["codec_type"].as_str() {
            Some("video") if !info.has_video => {
                info.has_video = true;
                let (mut w, mut h) = (s["width"].as_u64().unwrap_or(0) as u32, s["height"].as_u64().unwrap_or(0) as u32);
                let mut rot = s["tags"]["rotate"].as_str().and_then(|r| r.parse::<i64>().ok()).unwrap_or(0);
                if let Some(sd) = s["side_data_list"].as_array() {
                    for d in sd {
                        if let Some(r) = d["rotation"].as_i64() {
                            rot = r;
                        }
                    }
                }
                if rot.rem_euclid(180) == 90 {
                    std::mem::swap(&mut w, &mut h);
                }
                info.width = w;
                info.height = h;
                let fr = s["avg_frame_rate"].as_str().or(s["r_frame_rate"].as_str()).unwrap_or("0/1");
                let mut it = fr.split('/');
                let (n, d): (f64, f64) = (it.next().and_then(|x| x.parse().ok()).unwrap_or(0.0), it.next().and_then(|x| x.parse().ok()).unwrap_or(1.0));
                info.fps = if d > 0.0 { n / d } else { 0.0 };
                let trc = s["color_transfer"].as_str().unwrap_or("");
                info.hdr = trc == "arib-std-b67" || trc == "smpte2084";
                if info.duration == 0.0 {
                    info.duration = s["duration"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                }
            }
            Some("audio") => info.has_audio = true,
            _ => {}
        }
    }
    Ok(info)
}

pub fn decode_image_rgba(path: &Path, w: u32, h: u32) -> Result<Vec<u8>> {
    let out = ffmpeg()
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()?;
    if !out.status.success() || out.stdout.len() < (w * h * 4) as usize {
        bail!("ffmpeg could not decode {}", path.display());
    }
    Ok(out.stdout[..(w * h * 4) as usize].to_vec())
}

/// Decode mono f32 PCM at `rate` for [start, start+dur).
pub fn decode_audio(path: &Path, start: f64, dur: f64, rate: u32) -> Result<Vec<f32>> {
    let out = ffmpeg()
        .args(["-ss", &format!("{start:.3}"), "-t", &format!("{dur:.3}")])
        .arg("-i")
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", &rate.to_string(), "-f", "f32le", "-"])
        .output()
        .context("running ffmpeg")?;
    if !out.status.success() {
        bail!("ffmpeg could not decode audio from {}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

static HAS_ZSCALE: OnceLock<bool> = OnceLock::new();
fn has_zscale() -> bool {
    *HAS_ZSCALE.get_or_init(|| {
        Command::new("ffmpeg")
            .args(["-hide_banner", "-filters"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(" zscale "))
            .unwrap_or(false)
    })
}

/// Streams decoded RGBA frames of a video, already scaled to the size it is drawn at.
pub struct VideoReader {
    child: Child,
    out: BufReader<ChildStdout>,
    pub frame: Vec<u8>,
    pub w: u32,
    pub h: u32,
    /// layer-local frame index of the frame currently in `frame` (-1 = none yet)
    pub cur: i64,
    first: i64,
    eof: bool,
}

impl VideoReader {
    /// Start decoding so that the first frame read corresponds to layer-local frame `first`.
    pub fn spawn(src: &Path, info: &MediaInfo, trim: f64, first: i64, fps: f64, w: u32, h: u32, looped: bool) -> Result<VideoReader> {
        let mut seek = trim + first as f64 / fps;
        if looped && info.duration > 0.0 {
            seek %= info.duration;
        }
        // resample + scale first so the (expensive) tone mapping runs at the final, smaller size
        let mut vf = format!("fps={fps},scale={w}:{h}:flags=bicubic");
        if info.hdr && has_zscale() {
            // HLG/PQ phone footage -> SDR, otherwise it looks washed out
            vf.push_str(",zscale=t=linear:npl=203,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv");
        }
        vf.push_str(",format=rgba");
        let mut cmd = ffmpeg();
        cmd.args(["-threads", "2"]);
        if looped {
            cmd.args(["-stream_loop", "-1"]);
        }
        cmd.args(["-ss", &format!("{seek:.4}")]).arg("-i").arg(src);
        cmd.args(["-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-"]);
        cmd.stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
        let mut child = cmd.spawn().context("starting ffmpeg decoder")?;
        let out = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());
        Ok(VideoReader { child, out, frame: vec![0; (w * h * 4) as usize], w, h, cur: first - 1, first, eof: false })
    }

    pub fn first(&self) -> i64 {
        self.first
    }

    /// Advance to layer-local frame `idx` (must be >= current). Holds the last frame at EOF.
    pub fn seek_forward(&mut self, idx: i64) -> Result<()> {
        while self.cur < idx && !self.eof {
            match self.out.read_exact(&mut self.frame) {
                Ok(()) => {
                    // ffmpeg's rgba is straight alpha; video is opaque so force alpha
                    self.cur += 1;
                }
                Err(_) => {
                    self.eof = true;
                    if self.cur < self.first {
                        // nothing decoded at all: leave black
                        self.frame.fill(0);
                    }
                }
            }
        }
        Ok(())
    }
}

impl Drop for VideoReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Encoder {
    Nvenc,
    Qsv,
    Vaapi,
    X264,
}

impl Encoder {
    pub fn name(self) -> &'static str {
        match self {
            Encoder::Nvenc => "h264_nvenc (GPU)",
            Encoder::Qsv => "h264_qsv (Intel GPU)",
            Encoder::Vaapi => "h264_vaapi (GPU)",
            Encoder::X264 => "libx264 (CPU)",
        }
    }
}

pub fn vaapi_device() -> String {
    std::env::var("FLITS_VAAPI_DEVICE").unwrap_or_else(|_| "/dev/dri/renderD128".into())
}

fn encoder_works(codec: &str) -> bool {
    let mut c = ffmpeg();
    if codec == "h264_vaapi" {
        c.args(["-vaapi_device", &vaapi_device()]);
    }
    c.args(["-f", "lavfi", "-i", "color=c=black:s=256x256:d=0.2"]);
    if codec == "h264_vaapi" {
        c.args(["-vf", "format=nv12,hwupload"]);
    }
    c.args(["-c:v", codec, "-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn pick_encoder(choice: &str) -> Result<Encoder> {
    Ok(match choice {
        "nvenc" | "gpu" => Encoder::Nvenc,
        "qsv" => Encoder::Qsv,
        "vaapi" => Encoder::Vaapi,
        "x264" | "cpu" => Encoder::X264,
        "auto" => {
            if encoder_works("h264_nvenc") {
                Encoder::Nvenc
            } else if cfg!(target_os = "linux") && encoder_works("h264_vaapi") {
                Encoder::Vaapi
            } else if encoder_works("h264_qsv") {
                Encoder::Qsv
            } else {
                Encoder::X264
            }
        }
        o => bail!("unknown encoder '{o}' (auto, nvenc, qsv, vaapi, x264)"),
    })
}

/// ffmpeg process that reads raw yuv420p frames on stdin and writes an H.264 file.
pub fn spawn_encoder(enc: Encoder, w: u32, h: u32, fps: f64, quality: u32, out: &Path) -> Result<Child> {
    let mut cmd = ffmpeg();
    if enc == Encoder::Vaapi {
        cmd.args(["-vaapi_device", &vaapi_device()]);
    }
    cmd.args(["-y", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-s", &format!("{w}x{h}"), "-r", &format!("{fps}"), "-i", "-"]);
    match enc {
        Encoder::Nvenc => {
            cmd.args(["-c:v", "h264_nvenc", "-preset", "p4", "-tune", "hq", "-rc", "vbr", "-cq", &quality.to_string(), "-b:v", "0", "-profile:v", "high"]);
        }
        Encoder::Qsv => {
            cmd.args(["-c:v", "h264_qsv", "-preset", "fast", "-global_quality", &quality.to_string()]);
        }
        Encoder::Vaapi => {
            cmd.args(["-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi", "-qp", &quality.to_string()]);
        }
        Encoder::X264 => {
            cmd.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", &quality.to_string(), "-threads", "4"]);
        }
    }
    if enc != Encoder::Vaapi {
        cmd.args(["-pix_fmt", "yuv420p"]);
    }
    cmd.args(["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709", "-color_range", "tv"]);
    cmd.arg(out);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    cmd.spawn().context("starting ffmpeg encoder")
}

pub fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let out = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output().with_context(|| format!("starting ffmpeg ({what})"))?;
    if !out.status.success() {
        return Err(anyhow!("ffmpeg {what} failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

pub fn tmp_dir_for(out: &Path) -> PathBuf {
    let parent = out.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    parent.join(format!(".flits-{}", std::process::id()))
}
