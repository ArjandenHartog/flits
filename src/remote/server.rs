//! `flits serve`: HTTP server that runs jobs on this machine's GPU.

use super::{pct_decode, rand_u64};
use anyhow::{anyhow, Context, Result};
use serde_json::json;
use std::fs::File;
use std::sync::{Arc, Mutex};

fn query(url: &str) -> Vec<(String, String)> {
    url.split_once('?')
        .map(|(_, q)| q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), pct_decode(v))).collect())
        .unwrap_or_default()
}

fn q<'a>(qs: &'a [(String, String)], k: &str) -> Option<&'a str> {
    qs.iter().find(|(a, _)| a == k).map(|(_, b)| b.as_str())
}

fn safe(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.' || *c == '_').collect()
}

pub fn serve(port: u16, token: String) -> Result<()> {
    let server = tiny_http::Server::http(("0.0.0.0", port)).map_err(|e| anyhow!("cannot listen on port {port}: {e}"))?;
    let gpu_name = crate::paint::gpu::Gpu::new().map(|g| g.name.clone()).ok();
    eprintln!("flits: serving on http://0.0.0.0:{port}  (GPU: {})", gpu_name.as_deref().unwrap_or("none"));
    let job = Arc::new(Mutex::new(()));
    let gpu_name = Arc::new(gpu_name);
    for req in server.incoming_requests() {
        let job = job.clone();
        let token = token.clone();
        let gpu_name = gpu_name.clone();
        std::thread::spawn(move || {
            let peer = req.remote_addr().map(|a| a.to_string()).unwrap_or_default();
            let what = format!("{} {}", req.method(), req.url());
            match handle(req, &token, &job, &gpu_name) {
                Ok(()) => eprintln!("flits: {peer} {what} ok"),
                Err(e) => eprintln!("flits: {peer} {what} failed: {e:#}"),
            }
        });
    }
    Ok(())
}

fn respond_text(req: tiny_http::Request, code: u16, msg: &str) -> Result<()> {
    req.respond(tiny_http::Response::from_string(msg).with_status_code(code))?;
    Ok(())
}

fn handle(mut req: tiny_http::Request, token: &str, job: &Mutex<()>, gpu_name: &Option<String>) -> Result<()> {
    let ok_auth =
        token.is_empty() || req.headers().iter().any(|h| h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {token}"));
    if !ok_auth {
        return respond_text(req, 401, "bad token");
    }
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();
    let qs = query(&url);
    match (req.method(), path.as_str()) {
        (tiny_http::Method::Get, "/health") => {
            let body = json!({"ok": true, "version": env!("CARGO_PKG_VERSION"), "gpu": gpu_name, "busy": job.try_lock().is_err()});
            req.respond(tiny_http::Response::from_string(body.to_string()))?;
            Ok(())
        }
        (tiny_http::Method::Post, "/transcribe") => {
            let dir = std::env::temp_dir().join(format!("flits-job-{}", std::process::id())).join(format!("t{}", rand_u64()));
            std::fs::create_dir_all(&dir)?;
            let input = dir.join(format!("input.{}", safe(q(&qs, "ext").unwrap_or("bin"))));
            std::io::copy(req.as_reader(), &mut File::create(&input)?)?;
            let out = dir.join("out.json");
            let opts = crate::transcribe::TranscribeOpts {
                model: safe(q(&qs, "model").unwrap_or("default")),
                language: safe(q(&qs, "lang").unwrap_or("auto")),
                device: "auto".into(),
                python: None,
                remote: Some("off".into()),
                initial_prompt: q(&qs, "prompt").filter(|s| !s.is_empty()).map(str::to_string),
                hotwords: q(&qs, "hotwords").filter(|s| !s.is_empty()).map(str::to_string),
                word_timestamps: q(&qs, "words") != Some("0"),
                diarize: q(&qs, "diarize") == Some("1"),
                speakers: q(&qs, "speakers").and_then(|s| s.parse().ok()),
            };
            let res = {
                let _g = job.lock().unwrap();
                crate::transcribe::transcribe(&input, &out, &opts)
            };
            let r = match res {
                Ok(_) => {
                    req.respond(tiny_http::Response::from_file(File::open(&out)?))?;
                    Ok(())
                }
                Err(e) => respond_text(req, 500, &format!("{e:#}")),
            };
            let _ = std::fs::remove_dir_all(&dir);
            r
        }
        (tiny_http::Method::Post, "/render") => {
            let dir = std::env::temp_dir().join(format!("flits-job-{}", std::process::id())).join(format!("r{}", rand_u64()));
            std::fs::create_dir_all(&dir)?;
            tar::Archive::new(req.as_reader()).unpack(&dir).context("unpacking bundle")?;
            let out = dir.join("out.mp4");
            let mut o = crate::Options::default();
            if let Some(s) = q(&qs, "scale").and_then(|s| s.parse().ok()) {
                o.scale = s;
            }
            if let Some(s) = q(&qs, "quality").and_then(|s| s.parse().ok()) {
                o.quality = s;
            }
            let res = {
                let _g = job.lock().unwrap();
                crate::render_file(&dir.join("scene.json"), &out, &o)
            };
            let r = match res {
                Ok(_) => {
                    req.respond(tiny_http::Response::from_file(File::open(&out)?))?;
                    Ok(())
                }
                Err(e) => respond_text(req, 500, &format!("{e:#}")),
            };
            let _ = std::fs::remove_dir_all(&dir);
            r
        }
        _ => respond_text(req, 404, "not found"),
    }
}
