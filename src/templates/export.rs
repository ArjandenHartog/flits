//! Timeline exports of a cut, so the edit can be finished in an editor: FCPXML 1.9 (Final Cut
//! Pro, DaVinci Resolve), FCP7 XML / xmeml (Premiere Pro), CMX3600 EDL (Resolve, Avid, ...) and
//! a JSON cut list. Every kept range becomes one clip that refers to the original file.
//!
//! Cut points are snapped to whole frames of the source (editors cut on frames); NTSC rates
//! (23.976, 29.97, 59.94, ...) use the exact 1001-based rational timebases.

use super::abs;
use crate::media;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// FCPXML 1.9: Final Cut Pro and DaVinci Resolve (File > Import > Timeline)
    Fcpxml,
    /// FCP7 XML (xmeml v4): Premiere Pro (File > Import), also Resolve
    Premiere,
    /// CMX3600 EDL
    Edl,
    /// JSON cut list (for scripts)
    Json,
}

impl Format {
    pub fn parse(s: &str) -> Result<Format> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "fcpxml" | "fcp" | "fcpx" | "finalcut" | "final-cut" | "resolve" | "davinci" => Format::Fcpxml,
            "premiere" | "xmeml" | "xml" | "fcp7" => Format::Premiere,
            "edl" | "cmx" | "cmx3600" => Format::Edl,
            "json" => Format::Json,
            o => bail!("unknown export format '{o}' (fcpxml, premiere, resolve, edl, json)"),
        })
    }

    /// File name suffix for `<base><suffix>`.
    pub fn suffix(self) -> &'static str {
        match self {
            Format::Fcpxml => ".fcpxml",
            Format::Premiere => ".xml",
            Format::Edl => ".edl",
            Format::Json => ".cuts.json",
        }
    }
}

/// A frame rate as a fraction: `num / den` frames per second (29.97 = 30000/1001).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    pub num: u64,
    pub den: u64,
}

const STANDARD_RATES: &[(u64, u64)] = &[
    (24000, 1001),
    (24, 1),
    (25, 1),
    (30000, 1001),
    (30, 1),
    (48000, 1001),
    (48, 1),
    (50, 1),
    (60000, 1001),
    (60, 1),
    (100, 1),
    (120000, 1001),
    (120, 1),
];

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

impl Rate {
    /// Snap a measured frame rate to the nearest standard one (within 0.5 %), else a /1000 fraction.
    /// Zero or nonsense (audio-only files) becomes 30.
    pub fn from_fps(fps: f64) -> Rate {
        if !(fps.is_finite() && fps > 1.0 && fps <= 1000.0) {
            return Rate { num: 30, den: 1 };
        }
        let best = STANDARD_RATES
            .iter()
            .map(|&(n, d)| (Rate { num: n, den: d }, ((n as f64 / d as f64) - fps).abs() / fps))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        if best.1 < 0.005 {
            return best.0;
        }
        let num = (fps * 1000.0).round() as u64;
        let g = gcd(num, 1000);
        Rate { num: num / g, den: 1000 / g }
    }

    pub fn fps(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// NTSC (1001-based) rate
    pub fn ntsc(self) -> bool {
        self.den == 1001
    }

    /// Integer rate used for timecode labels (30 for 29.97).
    pub fn timebase(self) -> u64 {
        (self.fps().round() as u64).max(1)
    }

    /// Nearest frame at time `t` (seconds).
    pub fn frames(self, t: f64) -> i64 {
        (t * self.num as f64 / self.den as f64).round() as i64
    }

    /// Exact rational time of `frames` for FCPXML ("0s", "3s", "1001/30000s").
    pub fn fcp_time(self, frames: i64) -> String {
        if frames == 0 {
            return "0s".into();
        }
        let (neg, f) = (frames < 0, frames.unsigned_abs());
        let a = f * self.den;
        let g = gcd(a, self.num);
        let sign = if neg { "-" } else { "" };
        if self.num / g == 1 {
            format!("{sign}{}s", a / g)
        } else {
            format!("{sign}{}/{}s", a / g, self.num / g)
        }
    }

    /// Frames dropped from the timecode count per minute in drop-frame mode (2 at 29.97, 4 at 59.94).
    fn drop_frames(self) -> u64 {
        if self.ntsc() && self.timebase().is_multiple_of(30) {
            self.timebase() / 15
        } else {
            0
        }
    }

    /// Timecode label of a frame count: "01:00:00:00", or drop-frame "01:00:00;00".
    pub fn timecode(self, frames: i64, drop_frame: bool) -> String {
        let tb = self.timebase();
        let mut f = frames.max(0) as u64;
        let drop = if drop_frame { self.drop_frames() } else { 0 };
        if drop > 0 {
            let per10 = tb * 600 - drop * 9;
            let per1 = tb * 60 - drop;
            let (d, m) = (f / per10, f % per10);
            f += drop * 9 * d + if m > drop { drop * ((m - drop) / per1) } else { 0 };
        }
        let sep = if drop > 0 { ';' } else { ':' };
        format!("{:02}:{:02}:{:02}{sep}{:02}", f / (tb * 3600), f / (tb * 60) % 60, f / tb % 60, f % tb)
    }

    /// Frame count of a timecode label ("01:00:00:00" or drop-frame "01:00:00;00").
    pub fn parse_timecode(self, tc: &str) -> Option<(i64, bool)> {
        let df = tc.contains(';');
        let parts: Vec<u64> = tc.trim().split([':', ';', '.']).map(|p| p.parse().ok()).collect::<Option<_>>()?;
        let [h, m, s, f] = parts[..] else { return None };
        let tb = self.timebase();
        let mut frames = (h * 3600 + m * 60 + s) * tb + f;
        let drop = if df { self.drop_frames() } else { 0 };
        if drop > 0 {
            let minutes = h * 60 + m;
            frames -= drop * (minutes - minutes / 10);
        }
        Some((frames as i64, df && drop > 0))
    }
}

/// The source file of a timeline.
#[derive(Clone, Debug)]
pub struct Source {
    /// absolute path
    pub path: PathBuf,
    pub name: String,
    pub duration: f64,
    pub rate: Rate,
    pub width: u32,
    pub height: u32,
    pub has_video: bool,
    pub has_audio: bool,
    pub audio_rate: u32,
    pub audio_channels: u32,
    /// first frame's timecode in frames (cameras often start at 01:00:00:00), and whether it is drop-frame
    pub tc_start: i64,
    pub drop_frame: bool,
}

impl Source {
    pub fn probe(path: &Path) -> Result<Source> {
        let info = media::probe(path)?;
        // the nominal rate (r_frame_rate) is what editors use; phones report a VFR average like 29.98
        let out = std::process::Command::new(std::env::var("FLITS_FFPROBE").unwrap_or_else(|_| "ffprobe".into()))
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,r_frame_rate:stream_tags=timecode:format_tags=timecode",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .context("running ffprobe")?;
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        let streams = v["streams"].as_array().cloned().unwrap_or_default();
        let video = streams.iter().find(|s| s["codec_type"] == "video");
        let ratio = |s: &str| -> Option<f64> {
            let (n, d) = s.split_once('/')?;
            let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
            (d > 0.0).then_some(n / d)
        };
        let nominal = video.and_then(|s| s["r_frame_rate"].as_str()).and_then(ratio);
        let fps = match nominal {
            Some(r) if info.fps > 0.0 && (r - info.fps).abs() / r < 0.02 => r,
            _ => info.fps,
        };
        let rate = Rate::from_fps(fps);
        let tc = v["format"]["tags"]["timecode"].as_str().or_else(|| streams.iter().find_map(|s| s["tags"]["timecode"].as_str()));
        let (tc_start, drop_frame) = tc.and_then(|t| rate.parse_timecode(t)).unwrap_or((0, false));
        Ok(Source {
            path: PathBuf::from(abs(path)),
            name: path.file_name().and_then(|n| n.to_str()).unwrap_or("media").to_string(),
            duration: info.duration,
            rate,
            width: if info.has_video { info.width } else { 1920 },
            height: if info.has_video { info.height } else { 1080 },
            has_video: info.has_video,
            has_audio: info.has_audio,
            audio_rate: if info.has_audio { info.audio_rate } else { 48000 },
            audio_channels: info.audio_channels.max(1),
            tc_start,
            drop_frame,
        })
    }

    fn total_frames(&self) -> i64 {
        (self.duration * self.rate.fps()).ceil() as i64
    }
}

/// One clip on the timeline, in frames: source in/out (out exclusive) and its record (timeline) in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    pub src_in: i64,
    pub src_out: i64,
    pub rec_in: i64,
}

impl Clip {
    pub fn len(&self) -> i64 {
        self.src_out - self.src_in
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 0
    }
}

pub struct Timeline {
    pub source: Source,
    /// sequence/project name
    pub name: String,
    pub clips: Vec<Clip>,
    /// the exact kept ranges in seconds (for the JSON cut list)
    pub keep: Vec<(f64, f64)>,
}

impl Timeline {
    /// Kept ranges (seconds) snapped to frames; ranges that touch after snapping are merged.
    pub fn new(source: Source, keep: &[(f64, f64)], name: &str) -> Timeline {
        let r = source.rate;
        let total = source.total_frames();
        let mut clips: Vec<Clip> = vec![];
        let mut rec = 0;
        for &(s, e) in keep {
            let (a, b) = (r.frames(s).clamp(0, total), r.frames(e).clamp(0, total));
            if b <= a {
                continue;
            }
            if let Some(last) = clips.last_mut() {
                if a <= last.src_out {
                    rec += b.max(last.src_out) - last.src_out;
                    last.src_out = last.src_out.max(b);
                    continue;
                }
            }
            clips.push(Clip { src_in: a, src_out: b, rec_in: rec });
            rec += b - a;
        }
        Timeline { source, name: name.to_string(), clips, keep: keep.to_vec() }
    }

    pub fn frames(&self) -> i64 {
        self.clips.iter().map(Clip::len).sum()
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// `file:///C:/My%20Videos/a.mp4` (`localhost` = `file://localhost/...`, the form Premiere writes).
pub fn file_url(path: &Path, localhost: bool) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from(if localhost { "file://localhost" } else { "file://" });
    if !p.starts_with('/') {
        out.push('/');
    }
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => out.push(b as char),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn fcp_audio_rate(hz: u32) -> &'static str {
    match hz {
        0..=38000 => "32k",
        38001..=46000 => "44.1k",
        46001..=70000 => "48k",
        70001..=92000 => "88.2k",
        92001..=140000 => "96k",
        140001..=184000 => "176.4k",
        _ => "192k",
    }
}

/// FCPXML 1.9 (Final Cut Pro 10.5+, DaVinci Resolve 17+).
pub fn fcpxml(t: &Timeline) -> String {
    let s = &t.source;
    let r = s.rate;
    let tc_format = if s.drop_frame { "DF" } else { "NDF" };
    let layout = if s.audio_channels == 1 {
        "mono"
    } else if s.audio_channels > 2 {
        "surround"
    } else {
        "stereo"
    };
    let mut x = String::new();
    x += "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE fcpxml>\n\n<fcpxml version=\"1.9\">\n    <resources>\n";
    let _ = writeln!(
        x,
        "        <format id=\"r1\" frameDuration=\"{}\" width=\"{}\" height=\"{}\" colorSpace=\"1-1-1 (Rec. 709)\"/>",
        r.fcp_time(1),
        s.width,
        s.height
    );
    let mut asset = format!(
        "        <asset id=\"r2\" name=\"{}\" start=\"{}\" duration=\"{}\" hasVideo=\"{}\"",
        esc(&s.name),
        r.fcp_time(s.tc_start),
        r.fcp_time(s.total_frames()),
        s.has_video as u8
    );
    if s.has_video {
        asset += " format=\"r1\"";
    }
    asset += &format!(" hasAudio=\"{}\"", s.has_audio as u8);
    if s.has_audio {
        asset += &format!(" audioSources=\"1\" audioChannels=\"{}\" audioRate=\"{}\"", s.audio_channels, s.audio_rate);
    }
    let _ = writeln!(
        x,
        "{asset}>\n            <media-rep kind=\"original-media\" src=\"{}\"/>\n        </asset>",
        esc(&file_url(&s.path, false))
    );
    x += "    </resources>\n    <library>\n";
    let _ = writeln!(x, "        <event name=\"{}\">\n            <project name=\"{}\">", esc(&t.name), esc(&t.name));
    let _ = writeln!(
        x,
        "                <sequence format=\"r1\" duration=\"{}\" tcStart=\"0s\" tcFormat=\"{tc_format}\" audioLayout=\"{layout}\" audioRate=\"{}\">\n                    <spine>",
        r.fcp_time(t.frames()),
        fcp_audio_rate(s.audio_rate)
    );
    for c in &t.clips {
        let fmt = if s.has_video { " format=\"r1\"" } else { "" };
        let _ = writeln!(
            x,
            "                        <asset-clip ref=\"r2\" offset=\"{}\" name=\"{}\" start=\"{}\" duration=\"{}\"{fmt} tcFormat=\"{tc_format}\"/>",
            r.fcp_time(c.rec_in),
            esc(&s.name),
            r.fcp_time(s.tc_start + c.src_in),
            r.fcp_time(c.len())
        );
    }
    x += "                    </spine>\n                </sequence>\n            </project>\n        </event>\n    </library>\n</fcpxml>\n";
    x
}

fn xml_rate(r: Rate) -> String {
    format!("<rate><timebase>{}</timebase><ntsc>{}</ntsc></rate>", r.timebase(), if r.ntsc() { "TRUE" } else { "FALSE" })
}

fn xml_timecode(r: Rate, frames: i64, df: bool) -> String {
    format!(
        "<timecode>{}<string>{}</string><frame>{frames}</frame><displayformat>{}</displayformat></timecode>",
        xml_rate(r),
        r.timecode(frames, df),
        if df { "DF" } else { "NDF" }
    )
}

/// FCP7 XML (xmeml version 4) as imported by Premiere Pro (and Resolve): one video track and one
/// audio track with linked clip items.
pub fn xmeml(t: &Timeline) -> String {
    let s = &t.source;
    let r = s.rate;
    let rate = xml_rate(r);
    let total = s.total_frames();
    let n = t.clips.len();
    // clip item ids: video 1..=n, audio n+1..=2n
    let vid = |i: usize| format!("clipitem-{}", i + 1);
    let aid = |i: usize| format!("clipitem-{}", if s.has_video { n + i + 1 } else { i + 1 });
    let file_full = {
        let mut f = format!(
            "<file id=\"file-1\"><name>{}</name><pathurl>{}</pathurl>{rate}<duration>{total}</duration>{}<media>",
            esc(&s.name),
            esc(&file_url(&s.path, true)),
            xml_timecode(r, s.tc_start, s.drop_frame)
        );
        if s.has_video {
            let _ = write!(
                f,
                "<video><samplecharacteristics>{rate}<width>{}</width><height>{}</height></samplecharacteristics></video>",
                s.width, s.height
            );
        }
        if s.has_audio {
            let _ = write!(
                f,
                "<audio><samplecharacteristics><depth>16</depth><samplerate>{}</samplerate></samplecharacteristics><channelcount>{}</channelcount></audio>",
                s.audio_rate, s.audio_channels
            );
        }
        f + "</media></file>"
    };
    let mut file_written = false;
    let mut file = || {
        if std::mem::replace(&mut file_written, true) {
            "<file id=\"file-1\"/>".to_string()
        } else {
            file_full.clone()
        }
    };
    let links = |i: usize| {
        let mut l = String::new();
        if s.has_video {
            let _ = write!(
                l,
                "<link><linkclipref>{}</linkclipref><mediatype>video</mediatype><trackindex>1</trackindex><clipindex>{}</clipindex></link>",
                vid(i),
                i + 1
            );
        }
        if s.has_audio {
            let _ = write!(
                l,
                "<link><linkclipref>{}</linkclipref><mediatype>audio</mediatype><trackindex>1</trackindex><clipindex>{}</clipindex><groupindex>1</groupindex></link>",
                aid(i),
                i + 1
            );
        }
        l
    };
    let item = |id: String, c: &Clip, kind: &str, file: String, links: String| {
        format!(
            "                        <clipitem id=\"{id}\">\n                            <masterclipid>masterclip-1</masterclipid><name>{}</name><enabled>TRUE</enabled><duration>{total}</duration>{rate}\n                            <start>{}</start><end>{}</end><in>{}</in><out>{}</out>\n                            {file}\n                            <sourcetrack><mediatype>{kind}</mediatype><trackindex>1</trackindex></sourcetrack>{links}\n                        </clipitem>\n",
            esc(&s.name),
            c.rec_in,
            c.rec_in + c.len(),
            c.src_in,
            c.src_out
        )
    };
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n<xmeml version=\"4\">\n");
    let _ = writeln!(
        x,
        "    <sequence id=\"sequence-1\">\n        <name>{}</name>\n        <duration>{}</duration>\n        {rate}\n        {}\n        <media>",
        esc(&t.name),
        t.frames(),
        xml_timecode(r, 0, s.drop_frame)
    );
    if s.has_video {
        let _ = writeln!(
            x,
            "            <video>\n                <format><samplecharacteristics>{rate}<width>{}</width><height>{}</height><pixelaspectratio>square</pixelaspectratio><fielddominance>none</fielddominance></samplecharacteristics></format>\n                <track>",
            s.width, s.height
        );
        for (i, c) in t.clips.iter().enumerate() {
            x += &item(vid(i), c, "video", file(), links(i));
        }
        x += "                </track>\n            </video>\n";
    }
    if s.has_audio {
        let _ = writeln!(
            x,
            "            <audio>\n                <numOutputChannels>{}</numOutputChannels>\n                <format><samplecharacteristics><depth>16</depth><samplerate>{}</samplerate></samplecharacteristics></format>\n                <track>",
            s.audio_channels.min(2),
            s.audio_rate
        );
        for (i, c) in t.clips.iter().enumerate() {
            x += &item(aid(i), c, "audio", file(), links(i));
        }
        x += "                </track>\n            </audio>\n";
    }
    x += "        </media>\n    </sequence>\n</xmeml>\n";
    x
}

/// CMX3600 EDL. Record timecode starts at 00:00:00:00; source timecode includes the file's own start timecode.
pub fn edl(t: &Timeline) -> String {
    let s = &t.source;
    let r = s.rate;
    let df = s.drop_frame;
    let title: String = t.name.chars().filter(|c| !c.is_control()).take(70).collect();
    let mut x = format!("TITLE: {title}\nFCM: {}\n\n", if df { "DROP FRAME" } else { "NON-DROP FRAME" });
    let track = match (s.has_video, s.has_audio, s.audio_channels) {
        (true, true, 1) => "A/V",
        (true, true, _) => "AA/V",
        (true, false, _) => "V",
        (false, _, 1) => "A",
        (false, _, _) => "AA",
    };
    for (i, c) in t.clips.iter().enumerate() {
        let _ = writeln!(
            x,
            "{:03}  AX       {track:<5} C        {} {} {} {}\n* FROM CLIP NAME: {}\n* SOURCE FILE: {}\n",
            i + 1,
            r.timecode(s.tc_start + c.src_in, df),
            r.timecode(s.tc_start + c.src_out, df),
            r.timecode(c.rec_in, df),
            r.timecode(c.rec_in + c.len(), df),
            s.name,
            s.path.display()
        );
    }
    x
}

/// JSON cut list with exact (unsnapped) times in seconds plus the frame-snapped clips.
pub fn json(t: &Timeline) -> Value {
    let s = &t.source;
    let mut rec = 0.0;
    let ranges: Vec<Value> = t
        .keep
        .iter()
        .map(|&(a, b)| {
            let v = json!({"start": a, "end": b, "duration": b - a, "timelineStart": rec});
            rec += b - a;
            v
        })
        .collect();
    let mut removed = vec![];
    let mut prev = 0.0;
    for &(a, b) in &t.keep {
        if a > prev {
            removed.push(json!({"start": prev, "end": a}));
        }
        prev = b;
    }
    if s.duration > prev + 1e-6 {
        removed.push(json!({"start": prev, "end": s.duration}));
    }
    let clips: Vec<Value> = t.clips.iter().map(|c| json!({"in": c.src_in, "out": c.src_out, "timelineIn": c.rec_in})).collect();
    json!({
        "version": 1,
        "source": s.path,
        "duration": s.duration,
        "fps": s.rate.fps(),
        "timebase": format!("{}/{}", s.rate.num, s.rate.den),
        "keptDuration": rec,
        "removedDuration": s.duration - rec,
        "ranges": ranges,
        "removed": removed,
        "frames": clips,
    })
}

pub fn render(t: &Timeline, f: Format) -> Result<String> {
    Ok(match f {
        Format::Fcpxml => fcpxml(t),
        Format::Premiere => xmeml(t),
        Format::Edl => edl(t),
        Format::Json => serde_json::to_string_pretty(&json(t))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(fps: f64, video: bool) -> Source {
        Source {
            path: PathBuf::from("/media/my clip & co.mp4"),
            name: "my clip & co.mp4".into(),
            duration: 20.0,
            rate: Rate::from_fps(fps),
            width: 1920,
            height: 1080,
            has_video: video,
            has_audio: true,
            audio_rate: 48000,
            audio_channels: 2,
            tc_start: 0,
            drop_frame: false,
        }
    }

    const KEEP: &[(f64, f64)] = &[(0.0, 2.5), (4.0, 7.25), (10.0, 20.0)];

    /// "1001/30000s" -> seconds
    fn secs(t: &str) -> f64 {
        let t = t.trim_end_matches('s');
        match t.split_once('/') {
            Some((a, b)) => a.parse::<f64>().unwrap() / b.parse::<f64>().unwrap(),
            None => t.parse().unwrap(),
        }
    }

    fn parse(x: &str) -> roxmltree::Document<'_> {
        roxmltree::Document::parse_with_options(x, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
            .expect("well-formed XML")
    }

    #[test]
    fn rates_snap_to_standard_timebases() {
        assert_eq!(Rate::from_fps(29.97002997), Rate { num: 30000, den: 1001 });
        assert_eq!(Rate::from_fps(59.94), Rate { num: 60000, den: 1001 });
        assert_eq!(Rate::from_fps(23.976), Rate { num: 24000, den: 1001 });
        assert_eq!(Rate::from_fps(25.0), Rate { num: 25, den: 1 });
        assert_eq!(Rate::from_fps(30.01), Rate { num: 30, den: 1 });
        assert_eq!(Rate::from_fps(12.5), Rate { num: 25, den: 2 });
        assert_eq!(Rate::from_fps(0.0), Rate { num: 30, den: 1 });
        let r = Rate::from_fps(29.97);
        assert_eq!(r.fcp_time(1), "1001/30000s");
        assert_eq!(r.fcp_time(30), "1001/1000s");
        assert_eq!(Rate::from_fps(25.0).fcp_time(50), "2s");
        assert_eq!(Rate::from_fps(25.0).fcp_time(0), "0s");
    }

    #[test]
    fn timecodes_round_trip_including_drop_frame() {
        let r = Rate::from_fps(29.97);
        assert_eq!(r.timecode(108000, false), "01:00:00:00");
        // drop-frame: 00:01:00;02 follows 00:00:59;29
        assert_eq!(r.timecode(1799, true), "00:00:59;29");
        assert_eq!(r.timecode(1800, true), "00:01:00;02");
        assert_eq!(r.timecode(17982, true), "00:10:00;00");
        for f in [0, 1799, 1800, 17982, 107892, 123456] {
            assert_eq!(r.parse_timecode(&r.timecode(f, true)), Some((f, true)));
            assert_eq!(r.parse_timecode(&r.timecode(f, false)), Some((f, false)));
        }
        assert_eq!(Rate::from_fps(25.0).parse_timecode("01:00:00:00"), Some((90000, false)));
    }

    #[test]
    fn timeline_snaps_to_frames_and_merges() {
        let t = Timeline::new(source(25.0, true), &[(0.0, 1.0), (1.01, 2.0), (3.0, 3.01), (5.0, 6.0)], "x");
        // 1.01 s snaps to frame 25 = touches the first clip; 3.0..3.01 is less than a frame
        assert_eq!(t.clips, vec![Clip { src_in: 0, src_out: 50, rec_in: 0 }, Clip { src_in: 125, src_out: 150, rec_in: 50 }]);
        assert_eq!(t.frames(), 75);
    }

    #[test]
    fn fcpxml_is_well_formed_with_exact_ntsc_times() {
        let t = Timeline::new(source(29.97, true), KEEP, "my clip & co (cut)");
        let x = fcpxml(&t);
        let doc = parse(&x);
        let root = doc.root_element();
        assert_eq!(root.tag_name().name(), "fcpxml");
        assert_eq!(root.attribute("version"), Some("1.9"));
        let format = doc.descendants().find(|n| n.has_tag_name("format")).unwrap();
        assert_eq!(format.attribute("frameDuration"), Some("1001/30000s"));
        let asset = doc.descendants().find(|n| n.has_tag_name("asset")).unwrap();
        assert_eq!(asset.attribute("audioChannels"), Some("2"));
        let src = doc.descendants().find(|n| n.has_tag_name("media-rep")).unwrap().attribute("src").unwrap();
        assert_eq!(src, "file:///media/my%20clip%20%26%20co.mp4");
        let clips: Vec<_> = doc.descendants().filter(|n| n.has_tag_name("asset-clip")).collect();
        assert_eq!(clips.len(), 3);
        let fps = 30000.0 / 1001.0;
        let mut offset = 0.0;
        for (c, &(s, e)) in clips.iter().zip(KEEP) {
            let start = secs(c.attribute("start").unwrap());
            let dur = secs(c.attribute("duration").unwrap());
            assert!((start - s).abs() <= 0.5 / fps + 1e-9, "start {start} vs {s}");
            assert!((dur - (e - s)).abs() <= 1.0 / fps + 1e-9, "duration {dur} vs {}", e - s);
            assert!((secs(c.attribute("offset").unwrap()) - offset).abs() < 1e-9);
            // every time is a whole number of frames
            assert!(((start * fps) - (start * fps).round()).abs() < 1e-6);
            offset += dur;
        }
        let seq = doc.descendants().find(|n| n.has_tag_name("sequence")).unwrap();
        assert!((secs(seq.attribute("duration").unwrap()) - offset).abs() < 1e-9);
    }

    #[test]
    fn fcpxml_audio_only_and_source_timecode() {
        let mut s = source(0.0, false);
        s.tc_start = s.rate.parse_timecode("01:00:00:00").unwrap().0;
        let t = Timeline::new(s, KEEP, "pod");
        let x = fcpxml(&t);
        let doc = parse(&x);
        let asset = doc.descendants().find(|n| n.has_tag_name("asset")).unwrap();
        assert_eq!(asset.attribute("hasVideo"), Some("0"));
        assert_eq!(asset.attribute("format"), None);
        assert_eq!(secs(asset.attribute("start").unwrap()), 3600.0);
        let first = doc.descendants().find(|n| n.has_tag_name("asset-clip")).unwrap();
        assert_eq!(secs(first.attribute("start").unwrap()), 3600.0);
    }

    #[test]
    fn premiere_xml_is_well_formed_with_linked_tracks() {
        let t = Timeline::new(source(59.94, true), KEEP, "cut");
        let x = xmeml(&t);
        let doc = parse(&x);
        assert_eq!(doc.root_element().tag_name().name(), "xmeml");
        let seq = doc.descendants().find(|n| n.has_tag_name("sequence")).unwrap();
        let rate = seq.children().find(|n| n.has_tag_name("rate")).unwrap();
        let text =
            |n: roxmltree::Node, tag: &str| n.descendants().find(|c| c.has_tag_name(tag)).and_then(|c| c.text()).unwrap().to_string();
        assert_eq!(text(rate, "timebase"), "60");
        assert_eq!(text(rate, "ntsc"), "TRUE");
        let video: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("video"))
            .flat_map(|v| v.descendants().filter(|n| n.has_tag_name("clipitem")))
            .collect();
        let audio: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("audio"))
            .flat_map(|v| v.descendants().filter(|n| n.has_tag_name("clipitem")))
            .collect();
        assert_eq!(video.len(), 3);
        assert_eq!(audio.len(), 3);
        let fps = 60000.0 / 1001.0;
        let mut rec = 0;
        for (c, &(s, e)) in video.iter().zip(KEEP) {
            let num = |tag: &str| text(*c, tag).parse::<i64>().unwrap();
            assert_eq!(num("start"), rec);
            assert_eq!(num("end") - num("start"), num("out") - num("in"));
            assert!((num("in") as f64 / fps - s).abs() <= 0.5 / fps + 1e-9);
            assert!((num("out") as f64 / fps - e).abs() <= 0.5 / fps + 1e-9);
            rec = num("end");
            assert_eq!(c.descendants().filter(|n| n.has_tag_name("link")).count(), 2);
        }
        // the file is defined once and referenced afterwards
        assert_eq!(doc.descendants().filter(|n| n.has_tag_name("pathurl")).count(), 1);
        assert_eq!(text(doc.root_element(), "pathurl"), "file://localhost/media/my%20clip%20%26%20co.mp4");
    }

    #[test]
    fn edl_lists_every_clip_with_continuous_record_times() {
        let t = Timeline::new(source(25.0, true), KEEP, "cut");
        let e = edl(&t);
        assert!(e.starts_with("TITLE: cut\nFCM: NON-DROP FRAME\n"));
        let events: Vec<Vec<&str>> =
            e.lines().filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit())).map(|l| l.split_whitespace().collect()).collect();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], ["001", "AX", "AA/V", "C", "00:00:00:00", "00:00:02:13", "00:00:00:00", "00:00:02:13"]);
        assert_eq!(events[1], ["002", "AX", "AA/V", "C", "00:00:04:00", "00:00:07:06", "00:00:02:13", "00:00:05:19"]);
        // record in of each event = record out of the previous one
        for w in events.windows(2) {
            assert_eq!(w[0][7], w[1][6]);
        }
        assert_eq!(e.matches("* FROM CLIP NAME: my clip & co.mp4").count(), 3);
    }

    #[test]
    fn json_cut_list_has_ranges_and_removed_parts() {
        let t = Timeline::new(source(30.0, true), KEEP, "cut");
        let v: Value = serde_json::from_str(&render(&t, Format::Json).unwrap()).unwrap();
        assert_eq!(v["ranges"].as_array().unwrap().len(), 3);
        assert_eq!(v["ranges"][1]["timelineStart"], 2.5);
        assert_eq!(v["removed"], json!([{"start": 2.5, "end": 4.0}, {"start": 7.25, "end": 10.0}]));
        assert_eq!(v["keptDuration"], 15.75);
        assert_eq!(v["frames"][1], json!({"in": 120, "out": 218, "timelineIn": 75}));
    }

    #[test]
    fn parses_format_names() {
        assert_eq!(Format::parse("resolve").unwrap(), Format::Fcpxml);
        assert_eq!(Format::parse("Premiere").unwrap(), Format::Premiere);
        assert_eq!(Format::parse("edl").unwrap(), Format::Edl);
        assert!(Format::parse("avid").is_err());
    }
}
