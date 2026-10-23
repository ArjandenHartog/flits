//! `flits preview`: serve frames over HTTP and hot-reload the scene file.

mod common;

use serde_json::{json, Value};
use std::io::Read;
use std::time::{Duration, Instant};

fn scene(width: u32) -> Value {
    json!({
        "width": width, "height": 180, "fps": 10, "duration": 2, "background": "#203040",
        "layers": [
            {"type": "rect", "width": 100, "height": 100, "color": "#f26328", "x": [[0, 50], [2, 300]]}
        ]
    })
}

fn get(url: &str) -> (u16, String, Vec<u8>) {
    let r = match ureq::get(url).timeout(Duration::from_secs(60)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("{url}: {e}"),
    };
    let (code, ct) = (r.status(), r.content_type().to_string());
    let mut body = Vec::new();
    r.into_reader().read_to_end(&mut body).unwrap();
    (code, ct, body)
}

fn json(url: &str) -> Value {
    serde_json::from_slice(&get(url).2).unwrap()
}

/// Long-poll until the server reports a version newer than `v`.
fn next_version(base: &str, v: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let i = json(&format!("{base}api/wait?v={v}"));
        if i["version"].as_u64().unwrap() > v || Instant::now() > deadline {
            return i;
        }
    }
}

#[test]
fn serves_frames_and_hot_reloads() {
    let dir = common::tmp("preview");
    let path = dir.join("scene.json");
    std::fs::write(&path, serde_json::to_string_pretty(&scene(320)).unwrap()).unwrap();
    let pv = flits::preview::Preview::start(&path, 0, 0.5, flits::pick_gpu("auto").unwrap()).unwrap();
    let base = pv.url();

    let (code, ct, page) = get(&base);
    assert_eq!(code, 200);
    assert!(ct.starts_with("text/html"));
    assert!(String::from_utf8_lossy(&page).contains("<canvas"));

    let i = json(&format!("{base}api/info"));
    assert_eq!(i["frames"], 20);
    assert_eq!(i["previewWidth"], 160);
    assert!(i["error"].is_null());
    let v1 = i["version"].as_u64().unwrap();

    let (code, ct, jpeg) = get(&format!("{base}frame?f=5"));
    assert_eq!(code, 200, "{}", String::from_utf8_lossy(&jpeg));
    assert_eq!(ct, "image/jpeg");
    let img = image::load_from_memory(&jpeg).unwrap();
    assert_eq!((img.width(), img.height()), (80 * 2, 90));
    // consecutive frames and seeking by time
    assert_eq!(get(&format!("{base}frame?f=6")).0, 200);
    assert_eq!(get(&format!("{base}frame?t=1.5")).0, 200);
    let thumb = image::load_from_memory(&get(&format!("{base}frame?f=0&w=64")).2).unwrap();
    assert_eq!((thumb.width(), thumb.height()), (64, 36));
    assert_eq!(get(&format!("{base}nope")).0, 404);

    // a broken file is reported, the last good version keeps rendering
    std::fs::write(&path, "{ \"width\": 320, oops").unwrap();
    let i = next_version(&base, v1);
    assert!(i["error"].as_str().is_some_and(|e| e.contains("parsing")), "{i}");
    assert_eq!(get(&format!("{base}frame?f=3")).0, 200);
    let v2 = i["version"].as_u64().unwrap();

    // fixing it reloads with the new content
    std::fs::write(&path, serde_json::to_string_pretty(&scene(400)).unwrap()).unwrap();
    let i = next_version(&base, v2);
    assert!(i["error"].is_null(), "{i}");
    assert_eq!(i["width"], 400);
    let img = image::load_from_memory(&get(&format!("{base}frame?f=0")).2).unwrap();
    assert_eq!(img.width(), 200);
}
