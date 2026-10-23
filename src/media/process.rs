//! Spawning ffmpeg.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn ffmpeg() -> Command {
    let mut c = Command::new(std::env::var("FLITS_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()));
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    c
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

/// Pass a filter graph file: `-/filter_complex` on ffmpeg >= 7, `-filter_complex_script` before.
pub fn filter_script(cmd: &mut Command, path: &Path) {
    if ffmpeg_major() >= 7 {
        cmd.arg("-/filter_complex").arg(path);
    } else {
        cmd.arg("-filter_complex_script").arg(path);
    }
}

pub fn ffmpeg_major() -> u32 {
    static V: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        let o = Command::new("ffmpeg").arg("-version").output().ok();
        o.and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            let v = s.split_whitespace().nth(2)?.to_string();
            v.trim_start_matches(|c: char| !c.is_ascii_digit()).split('.').next()?.parse().ok()
        })
        .unwrap_or(7)
    })
}
