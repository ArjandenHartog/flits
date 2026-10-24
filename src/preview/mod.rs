//! `flits preview`: live preview in the browser with hot reload.
//!
//! A small local HTTP server renders single frames on demand (JPEG, GPU when available) for a
//! self-contained page with a scrubber, playback, frame stepping and a thumbnail strip. The scene
//! file and every file it references are watched (mtime polling); on a change the scene is
//! prepared again and the page, which long-polls `/api/wait`, fetches the current frame again.
//! Load errors are shown in the page; the last good version stays on screen.
//!
//! Routes: `/` (the page), `/api/info`, `/api/wait?v=<version>` (returns when the version
//! differs, or after a timeout), `/frame?f=<frame>|t=<seconds>[&w=<width>]` (JPEG).

use crate::compose::Prepared;
use crate::paint::gpu::Gpu;
use crate::render::FrameRenderer;
use crate::scene::{Layer, Scene};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

const PAGE: &str = include_str!("index.html");
/// Idle renderers kept between requests; each keeps its video decoders open, so consecutive
/// frames (playback, slow scrubbing) continue decoding instead of seeking.
const POOL: usize = 3;
/// Encoded frames kept in memory (thumbnails, replays of the same frames).
const CACHE: usize = 240;
/// How often watched files are checked for changes.
const POLL: Duration = Duration::from_millis(250);
/// Longest time `/api/wait` holds a request.
const LONG_POLL: Duration = Duration::from_secs(25);
const JPEG_QUALITY: u8 = 85;

/// The scene as last loaded (or why loading failed).
struct Loaded {
    /// bumped on every reload attempt, successful or not
    version: u64,
    /// last successfully prepared scene (kept on screen while the file has errors)
    prepared: Option<Arc<Prepared>>,
    /// scene size before preview scaling
    size: (u32, u32),
    warnings: Vec<String>,
    error: Option<String>,
    prepare_seconds: f64,
}

/// An idle renderer with the frame it would decode next.
struct Slot {
    version: u64,
    next: i64,
    r: FrameRenderer,
}

type CacheKey = (u64, i64, u32);

struct State {
    scene: PathBuf,
    scale: f64,
    gpu: Option<Arc<Gpu>>,
    loaded: Mutex<Loaded>,
    changed: Condvar,
    pool: Mutex<Vec<Slot>>,
    cache: Mutex<(HashMap<CacheKey, Arc<Vec<u8>>>, VecDeque<CacheKey>)>,
    stop: AtomicBool,
}

/// A running preview server; stops when dropped.
pub struct Preview {
    pub port: u16,
    state: Arc<State>,
    server: Arc<tiny_http::Server>,
    accept: Option<JoinHandle<()>>,
}

impl Preview {
    /// Load the scene, start watching it and serve on `127.0.0.1:port` (0 = any free port).
    /// A scene that fails to load does not stop the server: the error is shown in the page.
    pub fn start(scene: &Path, port: u16, scale: f64, gpu: Option<Arc<Gpu>>) -> Result<Preview> {
        let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| anyhow!("cannot listen on port {port}: {e}"))?;
        let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(port);
        let server = Arc::new(server);
        let state = Arc::new(State {
            scene: scene.to_path_buf(),
            scale,
            gpu,
            loaded: Mutex::new(Loaded { version: 0, prepared: None, size: (0, 0), warnings: vec![], error: None, prepare_seconds: 0.0 }),
            changed: Condvar::new(),
            pool: Mutex::new(Vec::new()),
            cache: Mutex::new((HashMap::new(), VecDeque::new())),
            stop: AtomicBool::new(false),
        });
        let files = reload(&state);
        let st = state.clone();
        std::thread::spawn(move || watch(st, files));
        let (st, srv) = (state.clone(), server.clone());
        let accept = std::thread::spawn(move || {
            for req in srv.incoming_requests() {
                let st = st.clone();
                std::thread::spawn(move || {
                    let what = format!("{} {}", req.method(), req.url());
                    if let Err(e) = handle(req, &st) {
                        eprintln!("flits: {what} failed: {e:#}");
                    }
                });
            }
        });
        Ok(Preview { port, state, server, accept: Some(accept) })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    /// Block until the server stops.
    pub fn wait(mut self) {
        if let Some(a) = self.accept.take() {
            let _ = a.join();
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        self.state.changed.notify_all();
        self.server.unblock();
    }
}

/// Open `url` in the default browser (best effort).
pub fn open_browser(url: &str) -> bool {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().is_ok()
}

/// Files a scene reads besides itself: media, transcripts, fonts.
fn referenced_files(s: &Scene) -> Vec<PathBuf> {
    fn walk(layers: &[Layer], s: &Scene, out: &mut Vec<PathBuf>) {
        for l in layers {
            if let Some(src) = &l.src {
                out.push(s.resolve(src));
            }
            // font file paths (installed font names have no extension)
            if let Some(f) = l.font.as_deref().filter(|f| Path::new(f).extension().is_some()) {
                out.push(s.resolve(f));
            }
            walk(&l.children, s, out);
        }
    }
    let mut out: Vec<PathBuf> = s.fonts.values().map(|f| s.resolve(f)).chain(s.audio.iter().map(|a| s.resolve(&a.src))).collect();
    walk(&s.layers, s, &mut out);
    out.sort();
    out.dedup();
    out
}

type Stamp = Vec<Option<(Option<SystemTime>, u64)>>;

fn stamp(files: &[PathBuf]) -> Stamp {
    files.iter().map(|f| std::fs::metadata(f).ok().map(|m| (m.modified().ok(), m.len()))).collect()
}

/// Load and prepare the scene again; returns the files to watch.
fn reload(st: &State) -> Vec<PathBuf> {
    let t = Instant::now();
    let mut files = if crate::scene::script::is_script(&st.scene) {
        // a script scene: its own folder's sources (imports, data files) too
        crate::scene::script::sources(&st.scene)
    } else {
        vec![st.scene.clone()]
    };
    let mut warnings = Vec::new();
    let res = Scene::load(&st.scene).and_then(|(s, w)| {
        warnings = w;
        files.extend(referenced_files(&s));
        Ok((Prepared::build(&s, st.scale)?, (s.width, s.height)))
    });
    let secs = t.elapsed().as_secs_f64();
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    let mut l = st.loaded.lock().unwrap();
    l.version += 1;
    l.warnings = warnings;
    l.prepare_seconds = secs;
    match res {
        Ok((p, size)) => {
            eprintln!("flits: prepared scene in {secs:.2}s ({} frames)", p.frames);
            l.prepared = Some(Arc::new(p));
            l.size = size;
            l.error = None;
        }
        Err(e) => {
            eprintln!("flits: scene error: {e:#}");
            l.error = Some(format!("{e:#}"));
        }
    }
    drop(l);
    // decoders and cached frames belong to the previous version
    st.pool.lock().unwrap().clear();
    *st.cache.lock().unwrap() = (HashMap::new(), VecDeque::new());
    st.changed.notify_all();
    files
}

fn watch(st: Arc<State>, mut files: Vec<PathBuf>) {
    let mut last = stamp(&files);
    while !st.stop.load(Ordering::SeqCst) {
        std::thread::sleep(POLL);
        let now = stamp(&files);
        if now == last {
            continue;
        }
        // let the editor finish writing
        std::thread::sleep(Duration::from_millis(100));
        let before = stamp(&files);
        let new_files = reload(&st);
        last = if new_files == files { before } else { stamp(&new_files) };
        files = new_files;
    }
}

fn info(st: &State) -> Value {
    let l = st.loaded.lock().unwrap();
    let p = l.prepared.as_deref();
    json!({
        "version": l.version,
        "scene": st.scene.display().to_string(),
        "name": st.scene.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        "width": l.size.0,
        "height": l.size.1,
        "previewWidth": p.map(|p| p.width),
        "previewHeight": p.map(|p| p.height),
        "scale": st.scale,
        "fps": p.map(|p| p.fps),
        "frames": p.map(|p| p.frames).unwrap_or(0),
        "duration": p.map(|p| p.duration).unwrap_or(0.0),
        "warnings": l.warnings,
        "error": l.error,
        "prepareSeconds": l.prepare_seconds,
        "renderer": match &st.gpu { Some(g) => format!("GPU ({})", g.name), None => "CPU".into() },
    })
}

/// Long-poll: wait until the version differs from `v` (or a timeout / shutdown).
fn wait_info(st: &State, v: u64) -> Value {
    let l = st.loaded.lock().unwrap();
    let _l = st.changed.wait_timeout_while(l, LONG_POLL, |l| l.version == v && !st.stop.load(Ordering::SeqCst)).unwrap();
    drop(_l);
    info(st)
}

impl State {
    /// An idle renderer for `frame`: preferably one whose decoders are just behind it.
    fn take(&self, version: u64, frame: i64, p: &Prepared) -> Result<Slot> {
        let mut pool = self.pool.lock().unwrap();
        pool.retain(|s| s.version == version);
        let reach = (p.fps * 2.0) as i64;
        let near = pool.iter().enumerate().filter(|(_, s)| frame >= s.next - 1 && frame - s.next <= reach).max_by_key(|(_, s)| s.next);
        if let Some((i, _)) = near {
            return Ok(pool.remove(i));
        }
        if !pool.is_empty() {
            // jumping elsewhere: reuse the least recently used one with fresh decoders
            let mut s = pool.remove(0);
            s.r.reset_videos();
            return Ok(s);
        }
        drop(pool);
        Ok(Slot { version, next: 0, r: FrameRenderer::new(p, self.gpu.clone())? })
    }

    fn put(&self, s: Slot) {
        let mut pool = self.pool.lock().unwrap();
        pool.push(s);
        while pool.len() > POOL {
            pool.remove(0);
        }
    }

    /// JPEG of one frame (`width`: downscale, e.g. for thumbnails; 0 = preview size).
    fn frame_jpeg(&self, frame: Option<i64>, time: Option<f64>, width: u32) -> Result<Arc<Vec<u8>>, (u16, String)> {
        let (version, p, error) = {
            let l = self.loaded.lock().unwrap();
            (l.version, l.prepared.clone(), l.error.clone())
        };
        let p = p.ok_or_else(|| (503, error.unwrap_or_else(|| "scene not loaded".into())))?;
        let frame = frame.or_else(|| time.map(|t| (t * p.fps).round() as i64)).unwrap_or(0).clamp(0, (p.frames - 1).max(0));
        let width = if width == 0 || width >= p.width { 0 } else { width };
        let key = (version, frame, width);
        if let Some(j) = self.cache.lock().unwrap().0.get(&key) {
            return Ok(j.clone());
        }
        let jpeg = (|| -> Result<Vec<u8>> {
            // thumbnails jump around: render them on their own so they don't close the decoders
            // the playback / scrubbing renderers are streaming from
            let pooled = width == 0;
            let mut slot = match pooled {
                true => self.take(version, frame, &p)?,
                false => Slot { version, next: 0, r: FrameRenderer::new(&p, self.gpu.clone())? },
            };
            let pm = slot.r.rgba(&p, frame);
            slot.next = frame + 1;
            if pooled && pm.is_ok() {
                self.put(slot);
            }
            encode_jpeg(&pm?, width)
        })()
        .map_err(|e| (500, format!("frame {frame}: {e:#}")))?;
        let jpeg = Arc::new(jpeg);
        let mut c = self.cache.lock().unwrap();
        if self.loaded.lock().unwrap().version == version {
            c.0.insert(key, jpeg.clone());
            c.1.push_back(key);
            while c.1.len() > CACHE {
                let k = c.1.pop_front().unwrap();
                c.0.remove(&k);
            }
        }
        Ok(jpeg)
    }
}

fn encode_jpeg(pm: &tiny_skia::Pixmap, width: u32) -> Result<Vec<u8>> {
    // frames are opaque (the background is filled), so dropping alpha is enough
    let rgb: Vec<u8> = pm.data().as_chunks::<4>().0.iter().flat_map(|px| [px[0], px[1], px[2]]).collect();
    let mut img = image::RgbImage::from_raw(pm.width(), pm.height(), rgb).ok_or_else(|| anyhow!("bad frame size"))?;
    if width > 0 {
        let h = ((pm.height() as f64 * width as f64 / pm.width() as f64).round() as u32).max(1);
        img = image::imageops::resize(&img, width, h, image::imageops::FilterType::Triangle);
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY).encode(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(out)
}

fn query(url: &str) -> HashMap<String, String> {
    url.split_once('?')
        .map(|(_, q)| q.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect())
        .unwrap_or_default()
}

fn header(k: &str, v: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

fn handle(req: tiny_http::Request, st: &State) -> Result<()> {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("");
    let qs = query(&url);
    let num = |k: &str| qs.get(k).and_then(|v| v.parse::<f64>().ok());
    let no_store = header("Cache-Control", "no-store");
    let json_type = header("Content-Type", "application/json");
    let resp = match (req.method(), path) {
        (tiny_http::Method::Get, "/" | "/index.html") => {
            tiny_http::Response::from_string(PAGE).with_header(header("Content-Type", "text/html; charset=utf-8"))
        }
        (tiny_http::Method::Get, "/api/info") => tiny_http::Response::from_string(info(st).to_string()).with_header(json_type),
        (tiny_http::Method::Get, "/api/wait") => {
            let v = num("v").map(|v| v as u64).unwrap_or(0);
            tiny_http::Response::from_string(wait_info(st, v).to_string()).with_header(json_type)
        }
        (tiny_http::Method::Get, "/frame") => {
            let t = Instant::now();
            match st.frame_jpeg(num("f").map(|f| f as i64), num("t"), num("w").unwrap_or(0.0) as u32) {
                Ok(j) => tiny_http::Response::from_data(j.as_slice())
                    .with_header(header("Content-Type", "image/jpeg"))
                    .with_header(header("X-Render-Ms", &format!("{:.1}", t.elapsed().as_secs_f64() * 1000.0))),
                Err((code, msg)) => tiny_http::Response::from_string(msg).with_status_code(code),
            }
        }
        (tiny_http::Method::Get, "/favicon.ico") => tiny_http::Response::from_string("").with_status_code(204),
        _ => tiny_http::Response::from_string("not found").with_status_code(404),
    };
    req.respond(resp.with_header(no_store))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_queries() {
        let q = query("/frame?f=12&w=160&v=3");
        assert_eq!(q.get("f").map(String::as_str), Some("12"));
        assert_eq!(q.get("w").map(String::as_str), Some("160"));
        assert!(query("/api/info").is_empty());
    }

    #[test]
    fn finds_referenced_files() {
        let v = json!({"width": 100, "height": 100, "fonts": {"brand": "fonts/a.ttf"},
        "audio": [{"src": "music.mp3"}],
        "layers": [
            {"type": "text", "text": "x", "font": "Segoe UI Black"},
            {"type": "group", "children": [{"type": "image", "src": "a.jpg"}, {"type": "text", "text": "y", "font": "b.otf"}]}
        ]});
        let (s, _) = Scene::from_value(v, PathBuf::from("base")).unwrap();
        let f = referenced_files(&s);
        let names: Vec<String> = f.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names.len(), 4);
        for n in ["a.ttf", "music.mp3", "a.jpg", "b.otf"] {
            assert!(names.iter().any(|x| x == n), "{n} missing in {names:?}");
        }
    }
}
