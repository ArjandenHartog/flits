//! Records the git commit and build date for `flits --version` and `flits doctor` (which warns when
//! the running binary is older than the checkout it was built from). Works without git: a crates.io
//! build just has no commit.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).output().ok().filter(|o| o.status.success())?;
    let s = String::from_utf8(o.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `YYYY-MM-DD` (UTC) of a Unix time.
fn date(secs: u64) -> String {
    // days to civil date (Howard Hinnant's algorithm)
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    // only a checkout of its own: not a parent repository that happens to contain the crate
    let top = git(&["-C", &dir, "rev-parse", "--show-toplevel"]);
    let own = top.as_deref().is_some_and(|t| {
        let canon = |p: &str| std::fs::canonicalize(p).ok();
        canon(t).is_some() && canon(t) == canon(&dir)
    });
    let commit = if own { git(&["-C", &dir, "rev-parse", "HEAD"]) } else { None };
    if own {
        // rebuild when HEAD moves (a commit, a checkout); these paths work in worktrees too
        for p in ["HEAD", "packed-refs"] {
            if let Some(path) = git(&["-C", &dir, "rev-parse", "--path-format=absolute", "--git-path", p]) {
                println!("cargo:rerun-if-changed={path}");
            }
        }
        if let Some(r) = git(&["-C", &dir, "symbolic-ref", "-q", "HEAD"]) {
            if let Some(path) = git(&["-C", &dir, "rev-parse", "--path-format=absolute", "--git-path", &r]) {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let now = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    let built = date(now);
    let commit = commit.unwrap_or_default();
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let long = if commit.is_empty() {
        format!("{version} (built {built})")
    } else {
        format!("{version} ({} {built})", &commit[..9.min(commit.len())])
    };
    println!("cargo:rustc-env=FLITS_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=FLITS_BUILD_DATE={built}");
    println!("cargo:rustc-env=FLITS_LONG_VERSION={long}");
}
