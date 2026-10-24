//! Spawning ffmpeg.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn ffmpeg() -> Command {
    contain_children();
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

/// Make sure every ffmpeg we start dies with us. On Windows a child keeps running (and
/// keeps inherited pipe handles open, hanging whoever reads our output) when flits exits
/// or crashes; a job object with kill-on-close prevents that. Unix kills nothing by
/// default either, but there children get SIGPIPE as soon as we're gone.
pub fn contain_children() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::JobObjects::*;
            use windows_sys::Win32::System::Threading::GetCurrentProcess;
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            // breakaway allowed: the warm whisper worker (transcribe::daemon) is meant to outlive us
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            AssignProcessToJobObject(job, GetCurrentProcess());
            // the handle is intentionally leaked: the OS closes it when we exit
        }
    });
}

/// A temp folder only this user can use: `<temp>/flits` on Windows (%TEMP% is per user), and
/// `<temp>/flits-<uid>` with mode 0700 and checked ownership elsewhere (/tmp is shared: another
/// user could otherwise swap scripts or job files flits hands to helper processes). Being in the
/// temp folder, it is cleared on reboot.
pub fn user_temp_dir() -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        let uid = unsafe { libc::getuid() };
        let dir = std::env::temp_dir().join(format!("flits-{uid}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
        }
        let m = std::fs::symlink_metadata(&dir)?;
        if !m.is_dir() || m.uid() != uid {
            anyhow::bail!("{} is not a folder owned by you: remove it and try again", dir.display());
        }
        if m.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(dir)
    }
    #[cfg(not(unix))]
    {
        let dir = std::env::temp_dir().join("flits");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}
