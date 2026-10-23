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
