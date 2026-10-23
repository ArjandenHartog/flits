//! Offloading to a GPU machine: `flits serve` runs there, other machines send
//! transcription and render jobs to it over HTTP and fall back to local work
//! when it is unreachable.

mod server;
pub use server::serve;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Remote {
    pub url: String,
    pub token: String,
}

fn config_path() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
    };
    base.join("flits").join("remote.json")
}

/// Remote from FLITS_REMOTE/FLITS_TOKEN or the saved config (`flits remote <url>`).
pub fn configured() -> Option<Remote> {
    if let Ok(url) = std::env::var("FLITS_REMOTE") {
        if url == "off" || url.is_empty() {
            return None;
        }
        return Some(Remote { url: url.trim_end_matches('/').to_string(), token: std::env::var("FLITS_TOKEN").unwrap_or_default() });
    }
    let v: Value = serde_json::from_str(&std::fs::read_to_string(config_path()).ok()?).ok()?;
    Some(Remote { url: v["url"].as_str()?.trim_end_matches('/').to_string(), token: v["token"].as_str().unwrap_or("").to_string() })
}

pub fn save(url: &str, token: &str) -> Result<PathBuf> {
    let p = config_path();
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, serde_json::to_string_pretty(&json!({"url": url.trim_end_matches('/'), "token": token}))?)?;
    Ok(p)
}

fn agent(connect: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(connect).build()
}

impl Remote {
    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// Quick reachability check; returns the server's health JSON.
    pub fn health(&self) -> Result<Value> {
        let r = agent(Duration::from_millis(1500))
            .get(&format!("{}/health", self.url))
            .set("Authorization", &self.auth())
            .timeout(Duration::from_secs(4))
            .call()
            .map_err(|e| anyhow!("{e}"))?;
        Ok(serde_json::from_str(&r.into_string()?)?)
    }

    fn post_file(&self, path: &str, body: &Path, out: &Path) -> Result<()> {
        let size = std::fs::metadata(body)?.len();
        eprintln!("flits: sending {:.1} MB to {}", size as f64 / 1e6, self.url);
        let resp = agent(Duration::from_secs(3))
            .post(&format!("{}{}", self.url, path))
            .set("Authorization", &self.auth())
            .set("Content-Length", &size.to_string())
            .send(File::open(body)?);
        let resp = match resp {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => bail!("remote error {code}: {}", r.into_string().unwrap_or_default()),
            Err(e) => bail!("remote request failed: {e}"),
        };
        let mut f = File::create(out)?;
        std::io::copy(&mut resp.into_reader(), &mut f)?;
        Ok(())
    }

    /// Transcribe on the server. Writes the transcript JSON to `out_json`.
    pub fn transcribe(&self, input: &Path, out_json: &Path, o: &crate::transcribe::TranscribeOpts) -> Result<()> {
        let t = Instant::now();
        // video or big files: send only a compact mono audio track
        let info = crate::media::probe(input)?;
        let size = std::fs::metadata(input)?.len();
        let tmp = std::env::temp_dir().join(format!("flits-up-{}.m4a", std::process::id()));
        let send = if info.has_video || size > 80_000_000 {
            crate::media::run(
                crate::media::ffmpeg()
                    .arg("-y")
                    .arg("-i")
                    .arg(input)
                    .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "aac", "-aac_coder", "fast", "-b:a", "48k"])
                    .arg(&tmp),
                "audio extract",
            )?;
            tmp.clone()
        } else {
            input.to_path_buf()
        };
        let ext = send.extension().and_then(|e| e.to_str()).unwrap_or("bin");
        let mut query = format!("/transcribe?lang={}&model={}&ext={ext}", pct_encode(&o.language), pct_encode(&o.model));
        if !o.word_timestamps {
            query += "&words=0";
        }
        if let Some(p) = &o.initial_prompt {
            query += &format!("&prompt={}", pct_encode(p));
        }
        if let Some(h) = &o.hotwords {
            query += &format!("&hotwords={}", pct_encode(h));
        }
        if o.diarize {
            query += "&diarize=1";
        }
        if let Some(n) = o.speakers {
            query += &format!("&speakers={n}");
        }
        let r = self.post_file(&query, &send, out_json);
        let _ = std::fs::remove_file(&tmp);
        r?;
        eprintln!("flits: transcribed remotely in {:.1}s", t.elapsed().as_secs_f64());
        Ok(())
    }

    /// Bundle scene + all referenced files, render on the server, download the MP4.
    pub fn render(&self, scene: &Path, out: &Path, query: &str) -> Result<()> {
        let t = Instant::now();
        let tar_path = std::env::temp_dir().join(format!("flits-bundle-{}.tar", std::process::id()));
        bundle(scene, &tar_path)?;
        let r = self.post_file(&format!("/render?{query}"), &tar_path, out);
        let _ = std::fs::remove_file(&tar_path);
        r?;
        eprintln!("flits: rendered remotely in {:.1}s -> {}", t.elapsed().as_secs_f64(), out.display());
        Ok(())
    }
}

/// Rewrite every local file reference in the scene to a bundled name and tar it all up.
fn bundle(scene: &Path, tar_path: &Path) -> Result<()> {
    let base = scene.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(scene)?)?;
    let mut files: Vec<(PathBuf, String)> = vec![];
    fn walk(v: &mut Value, base: &Path, files: &mut Vec<(PathBuf, String)>, key: Option<&str>) {
        match v {
            Value::Object(o) => {
                let font_map = key == Some("fonts");
                for (k, child) in o.iter_mut() {
                    let is_ref = font_map || k == "src" || k == "font";
                    if is_ref {
                        if let Value::String(s) = child {
                            let p = if Path::new(s.as_str()).is_absolute() { PathBuf::from(&*s) } else { base.join(&*s) };
                            if p.is_file() {
                                let name = match files.iter().find(|(q, _)| *q == p) {
                                    Some((_, n)) => n.clone(),
                                    None => {
                                        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("bin");
                                        let n = format!("assets/f{}.{ext}", files.len());
                                        files.push((p.clone(), n.clone()));
                                        n
                                    }
                                };
                                *s = name;
                                continue;
                            }
                        }
                    }
                    walk(child, base, files, Some(k.as_str()));
                }
            }
            Value::Array(a) => {
                for c in a {
                    walk(c, base, files, None);
                }
            }
            _ => {}
        }
    }
    walk(&mut v, &base, &mut files, None);
    let mut tb = tar::Builder::new(File::create(tar_path)?);
    let json = serde_json::to_vec(&v)?;
    let mut h = tar::Header::new_gnu();
    h.set_size(json.len() as u64);
    h.set_mode(0o644);
    h.set_cksum();
    tb.append_data(&mut h, "scene.json", json.as_slice())?;
    for (p, n) in &files {
        tb.append_path_with_name(p, n).with_context(|| format!("bundling {}", p.display()))?;
    }
    tb.finish()?;
    Ok(())
}

/// Percent-encode a query value (everything but unreserved characters).
pub(crate) fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out += &format!("%{b:02X}"),
        }
    }
    out
}

/// Inverse of `pct_encode` (invalid escapes are kept as they are).
pub(crate) fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------- server

pub(crate) fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(Instant::now().elapsed().as_nanos());
    h.finish()
}

/// Random token for `flits serve` when none is given.
pub fn new_token() -> String {
    format!("{:016x}{:016x}", rand_u64(), rand_u64())
}

#[cfg(test)]
mod tests {
    #[test]
    fn percent_encoding_round_trips() {
        let s = "Arjan den Hartog, café & co: 100% ✓";
        let e = super::pct_encode(s);
        assert!(!e.contains(' ') && !e.contains('&'));
        assert_eq!(super::pct_decode(&e), s);
        assert_eq!(super::pct_decode("50%"), "50%");
    }
}
