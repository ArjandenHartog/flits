//! Audio: decoding to PCM and mixing the scene's soundtrack (one ffmpeg filter graph).
//!
//! Every input gets trim / speed / volume (constant or keyframed) / fades / delay.
//! Tracks marked `duck` are compressed by everything else (voice, video sound), and
//! the master can be loudness-normalized (EBU R128).

use super::{ffmpeg, run};
use crate::compose::{AudioInput, Prepared};
use crate::scene::anim::{Ease, Prop};
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::sync::OnceLock;

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
    Ok(out.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect())
}

/// ffmpeg >= 7 reads filter scripts with `-/filter_complex file`, older ones with `-filter_complex_script`.
fn ffmpeg_major() -> u32 {
    static V: OnceLock<u32> = OnceLock::new();
    *V.get_or_init(|| {
        let o = std::process::Command::new("ffmpeg").arg("-version").output().ok();
        o.and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            let v = s.split_whitespace().nth(2)?.to_string();
            v.trim_start_matches(|c: char| !c.is_ascii_digit()).split('.').next()?.parse().ok()
        })
        .unwrap_or(7)
    })
}

/// `volume` filter argument: a constant, or a piecewise-linear expression over the keyframes.
fn volume_expr(v: &Prop, dur: f64) -> String {
    match v {
        Prop::Const(c) => format!("{c:.4}"),
        Prop::Keys(keys) => {
            // key points; eased segments get a few extra samples, then linear interpolation
            let times: Vec<f64> = keys.iter().map(|k| k.0.at(dur)).collect();
            let mut pts = vec![(times[0], keys[0].1)];
            for i in 1..keys.len() {
                let steps = if keys[i].2 == Ease::Linear { 1 } else { 8 };
                for s in 1..=steps {
                    let t = times[i - 1] + (times[i] - times[i - 1]) * s as f64 / steps as f64;
                    pts.push((t, v.eval(t, dur)));
                }
            }
            let mut expr = format!("{:.4}", pts.last().unwrap().1);
            for w in pts.windows(2).rev() {
                let ((t0, v0), (t1, v1)) = (w[0], w[1]);
                expr = format!("if(lt(t,{t1:.3}),{v0:.4}+({:.4})*(t-{t0:.3}),{expr})", (v1 - v0) / (t1 - t0).max(1e-6));
            }
            format!("'if(lt(t,{:.3}),{:.4},{expr})':eval=frame", pts[0].0, pts[0].1)
        }
    }
}

/// atempo only accepts 0.5..2 per instance on older ffmpeg: chain it.
fn atempo(speed: f64) -> String {
    let mut s = speed;
    let mut parts = vec![];
    while s > 2.0 {
        parts.push("atempo=2.0".to_string());
        s /= 2.0;
    }
    while s < 0.5 {
        parts.push("atempo=0.5".to_string());
        s /= 0.5;
    }
    if (s - 1.0).abs() > 1e-6 {
        parts.push(format!("atempo={s:.5}"));
    }
    parts.join(",")
}

fn input_chain(i: usize, a: &AudioInput) -> String {
    let mut f = format!("[{i}:a]aresample=48000,aformat=channel_layouts=stereo");
    let t = atempo(a.speed);
    if !t.is_empty() {
        f += ",";
        f += &t;
    }
    f += &format!(",volume={}", volume_expr(&a.volume, a.dur));
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
    f + &format!("[a{i}]")
}

fn mix_of(labels: &[String], out: &str) -> String {
    if labels.len() == 1 {
        format!("{}anull[{out}]", labels[0])
    } else {
        format!("{}amix=inputs={}:normalize=0:duration=longest[{out}]", labels.concat(), labels.len())
    }
}

/// Build the whole filter graph (exposed for tests).
pub fn filter_graph(p: &Prepared) -> String {
    let mut f: Vec<String> = p.audio.iter().enumerate().map(|(i, a)| input_chain(i, a)).collect();
    let label = |i: usize| format!("[a{i}]");
    let ducked: Vec<String> = p.audio.iter().enumerate().filter(|(_, a)| a.duck.is_some()).map(|(i, _)| label(i)).collect();
    let keys: Vec<String> = p.audio.iter().enumerate().filter(|(_, a)| a.duck.is_none()).map(|(i, _)| label(i)).collect();
    if !ducked.is_empty() && !keys.is_empty() {
        // strongest requested duck level -> compressor ratio (voice ~20 dB above the threshold)
        let level = p.audio.iter().filter_map(|a| a.duck).fold(1.0f64, f64::min).clamp(0.01, 1.0);
        let reduction_db = -20.0 * level.log10();
        let ratio = 1.0 / (1.0 - (reduction_db / 20.0).min(0.95));
        f.push(mix_of(&ducked, "bed"));
        f.push(mix_of(&keys, "voice"));
        f.push("[voice]asplit=2[voice1][key]".into());
        f.push(format!("[bed][key]sidechaincompress=threshold=0.01:ratio={ratio:.2}:attack=20:release=400[bedd]"));
        f.push("[bedd][voice1]amix=inputs=2:normalize=0:duration=longest[mix]".into());
    } else {
        let all: Vec<String> = (0..p.audio.len()).map(label).collect();
        f.push(mix_of(&all, "mix"));
    }
    let mut master = format!("[mix]apad,atrim=0:{:.3}", p.duration);
    if let Some(lufs) = p.loudness {
        master += &format!(",loudnorm=I={lufs:.1}:TP=-1.5:LRA=11,aresample=48000");
    }
    f.push(master + "[out]");
    f.join(";\n")
}

/// Mix the soundtrack into `out` (AAC). Returns false when the scene has no audio.
pub fn mix(p: &Prepared, out: &Path) -> Result<bool> {
    if p.audio.is_empty() {
        return Ok(false);
    }
    let mut cmd = ffmpeg();
    cmd.arg("-y");
    for a in &p.audio {
        if a.looped {
            cmd.args(["-stream_loop", "-1"]);
        }
        let src_dur = a.dur * a.speed;
        cmd.args(["-ss", &format!("{:.3}", a.trim), "-t", &format!("{src_dur:.3}")]).arg("-i").arg(&a.path);
    }
    // the graph can get long (keyframed volumes, many clips): pass it as a file
    let script = out.with_extension("filters.txt");
    std::fs::write(&script, filter_graph(p))?;
    if ffmpeg_major() >= 7 {
        cmd.arg("-/filter_complex").arg(&script);
    } else {
        cmd.arg("-filter_complex_script").arg(&script);
    }
    cmd.args(["-map", "[out]", "-c:a", "aac", "-b:a", "192k"]).arg(out);
    let r = run(&mut cmd, "audio mix");
    let _ = std::fs::remove_file(&script);
    r?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::anim::KeyTime;

    #[test]
    fn atempo_chains_extremes() {
        assert_eq!(atempo(1.0), "");
        assert_eq!(atempo(4.0), "atempo=2.0,atempo=2.00000");
        assert_eq!(atempo(0.25), "atempo=0.5,atempo=0.50000");
    }

    #[test]
    fn volume_keys_become_expression() {
        let p = Prop::Keys(vec![(KeyTime::Abs(0.0), 1.0, Ease::Linear), (KeyTime::Abs(1.0), 0.0, Ease::Linear)]);
        let e = volume_expr(&p, 2.0);
        assert!(e.starts_with("'if(lt(t,") && e.ends_with(":eval=frame"), "{e}");
        assert_eq!(volume_expr(&Prop::Const(0.5), 2.0), "0.5000");
    }
}
