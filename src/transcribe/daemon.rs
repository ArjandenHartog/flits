//! The warm whisper worker: the helper started with `--serve` keeps its models loaded and takes
//! jobs over localhost, so only the first transcription pays for starting Python and loading the
//! model (~8 s of a 10 s run for a short clip). It exits by itself after 10 idle minutes
//! (`FLITS_WHISPER_IDLE` seconds); `FLITS_WHISPER_DAEMON=0` turns it off.
//!
//! Its address and token live in `dir` (`media::process::user_temp_dir`: private to this user,
//! cleared on reboot). The worker acknowledges every request at once, so a port that some other
//! program took over after the worker died is noticed within seconds instead of hanging.

use anyhow::{anyhow, bail, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a worker may take to acknowledge a request.
const ACK_WAIT: Duration = Duration::from_secs(3);

pub fn enabled() -> bool {
    std::env::var("FLITS_WHISPER_DAEMON").map_or(true, |v| v != "0")
}

/// Run `job` (a jobs.json for whisper.py) on the warm worker, starting one with `cmd` (the helper
/// command line, up to and including the interpreter) when none answers.
pub fn run(dir: &Path, mut cmd: Command, script: &Path, job: &Path) -> Result<()> {
    let state = dir.join("whisper-worker.json");
    if let Some(conn) = connect(&state) {
        match send(conn, job) {
            Err(Send::NotAWorker) => {}
            other => return other.map_err(Send::into_error),
        }
    }
    // start one, detached from us: it should outlive this flits
    let log = std::fs::File::create(dir.join("whisper-worker.log"))?;
    cmd.arg("-I").arg(script).arg("--serve").arg(&state);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        // Windows hands every inheritable handle to a child, including our own stdout/stderr: the
        // worker would keep whoever reads our output waiting for an end that never comes. Children
        // that should share them get duplicates from `Command`, so ours need not be inheritable.
        unsafe {
            use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
            use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
            for h in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                SetHandleInformation(GetStdHandle(h), HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
    let _ = std::fs::remove_file(&state);
    let mut child = cmd.spawn().context("starting the whisper worker")?;
    let t = Instant::now();
    loop {
        if let Some(conn) = connect(&state) {
            return send(conn, job).map_err(Send::into_error);
        }
        if let Some(status) = child.try_wait()? {
            let log = std::fs::read_to_string(dir.join("whisper-worker.log")).unwrap_or_default();
            bail!("the whisper worker stopped ({status}): {}", log.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
        }
        // the first start may install faster-whisper and the CUDA libraries
        if t.elapsed() > Duration::from_secs(900) {
            let _ = child.kill();
            bail!("the whisper worker did not start");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A connection to the worker named in `state`, with its token.
fn connect(state: &Path) -> Option<(TcpStream, String)> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(state).ok()?).ok()?;
    let port = u16::try_from(v["port"].as_u64()?).ok()?;
    let token = v["token"].as_str()?.to_string();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok().map(|s| (s, token))
}

enum Send {
    /// whatever listens there did not acknowledge: not (or no longer) our worker
    NotAWorker,
    Failed(anyhow::Error),
}

impl Send {
    fn into_error(self) -> anyhow::Error {
        match self {
            Send::NotAWorker => anyhow!("the whisper worker did not answer"),
            Send::Failed(e) => e,
        }
    }
}

/// Hand the job over and relay the worker's progress to stderr.
fn send((mut conn, token): (TcpStream, String), job: &Path) -> Result<(), Send> {
    let io = |e: std::io::Error| Send::Failed(e.into());
    let req = serde_json::json!({"token": token, "job": job});
    if conn.write_all(format!("{req}\n").as_bytes()).is_err() {
        return Err(Send::NotAWorker);
    }
    conn.set_read_timeout(Some(ACK_WAIT)).map_err(io)?;
    // read from the stream itself (not a clone): on Windows a clone keeps its own timeout
    let mut lines = BufReader::new(&conn).lines();
    match lines.next() {
        Some(Ok(l)) if l == "ACK" => {}
        Some(Ok(l)) if l.starts_with("ERR ") => return Err(Send::Failed(anyhow!("{}", &l[4..]))),
        _ => return Err(Send::NotAWorker),
    }
    // a long file can take minutes between progress lines
    conn.set_read_timeout(None).map_err(io)?;
    for line in lines {
        let line = line.map_err(io)?;
        if let Some(msg) = line.strip_prefix("LOG ") {
            eprintln!("{msg}");
        } else if line == "OK" {
            return Ok(());
        } else if let Some(e) = line.strip_prefix("ERR ") {
            return Err(Send::Failed(anyhow!("{e}")));
        }
    }
    Err(Send::Failed(anyhow!("the whisper worker hung up")))
}
