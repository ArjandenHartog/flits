//! The JSON Schema (`flits schema`, committed as schema/scene.schema.json) matches the Rust
//! structs, accepts the scenes flits itself produces and catches the usual mistakes. The
//! TypeScript and Python bindings are checked to cover every property and enum value.
//!
//! After changing the scene format, regenerate the committed copy with
//! `cargo run --profile fast -- schema -o schema/scene.schema.json`
//! (or run this test with FLITS_UPDATE_SCHEMA=1).

use flits::scene::schema::scene_schema;
use flits::templates::starters::{starter, STARTERS};
use flits::Scene;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(p)
}

fn validator() -> jsonschema::Validator {
    jsonschema::validator_for(&scene_schema()).expect("schema compiles")
}

fn errors(v: &jsonschema::Validator, scene: &Value) -> Vec<String> {
    v.iter_errors(scene).map(|e| format!("{e} (at {})", e.instance_path())).collect()
}

#[test]
fn committed_schema_is_up_to_date() {
    let path = repo("schema/scene.schema.json");
    if std::env::var_os("FLITS_UPDATE_SCHEMA").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, flits::scene::schema::scene_schema_pretty()).unwrap();
    }
    let committed: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("schema/scene.schema.json exists")).unwrap();
    assert!(
        committed == scene_schema(),
        "schema/scene.schema.json is out of date: run `cargo run --profile fast -- schema -o schema/scene.schema.json`"
    );
}

#[test]
fn schema_is_a_valid_json_schema() {
    jsonschema::meta::validate(&scene_schema()).expect("valid against the draft 2020-12 meta-schema");
}

#[test]
fn starters_validate() {
    let v = validator();
    for (name, _) in STARTERS {
        let scene = starter(name).unwrap();
        let errs = errors(&v, &scene);
        assert!(errs.is_empty(), "starter '{name}': {errs:#?}");
    }
}

/// Every ```json block in SPEC.md validates: full scenes (with "width") as they are, other blocks as a
/// single layer of a scene.
#[test]
fn spec_examples_validate() {
    let spec = std::fs::read_to_string(repo("SPEC.md")).unwrap();
    let blocks: Vec<&str> = spec.split("```json\n").skip(1).filter_map(|s| s.split("```").next()).collect();
    assert!(blocks.iter().any(|b| b.contains("\"width\"")), "SPEC.md has a full ```json scene example");
    let v = validator();
    for block in blocks {
        let value: Value = serde_json::from_str(block).unwrap_or_else(|e| panic!("SPEC.md example is not valid JSON ({e}):\n{block}"));
        let scene = if value.get("width").is_some() { value } else { scene_with(value) };
        let errs = errors(&v, &scene);
        assert!(errs.is_empty(), "SPEC.md example {block}: {errs:#?}");
    }
}

fn scene_with(layer: Value) -> Value {
    json!({"width": 1080, "height": 1920, "layers": [layer]})
}

#[test]
fn accepts_every_documented_form() {
    let v = validator();
    let good = [
        json!({"$schema": "../schema/scene.schema.json", "comment": "hi", "width": 640, "height": 360, "fps": 25, "duration": 3,
               "background": "rgba(16,16,20,1)", "fonts": {"brand": "Rubik.ttf"}, "loudness": -14,
               "audio": [{"src": "m.mp3", "volume": [[0, 0], [1, 0.8]], "duck": 0.2, "loop": true, "fadeIn": 1},
                         {"src": "v.wav", "duck": true}],
               "layers": []}),
        scene_with(json!({"type": "rect", "color": "#fff8", "x": [[0, 0], ["end-0.5", 100, "back-out"], ["end", 0]],
                          "in": "fade 0.4", "out": {"type": "wipe-left", "duration": 0.3, "ease": "in"}, "comment": "note"})),
        scene_with(json!({"type": "rect", "gradient": {"type": "linear", "angle": 90, "stops": [[0, "black"], [1, "#00000000"]]}})),
        scene_with(
            json!({"type": "image", "src": "a.jpg", "fit": "contain", "zoom": {"from": 1, "to": 1.2, "ease": "smooth", "start": 0.5, "end": "end-1"},
                          "filter": {"warmth": 0.3, "grayscale": true}, "border": {"width": 8, "color": "white"}, "shadow": {"blur": 30}}),
        ),
        scene_with(json!({"type": "video", "src": "c.mp4", "speed": 0.5, "volume": 0, "trim": 2, "loop": true})),
        scene_with(json!({"type": "text", "text": "Hi", "align": "left", "wrap": "balance", "reveal": "typewriter", "revealDuration": 2,
                          "background": {"color": "#000a", "padding": [24, 12], "radius": 16, "mode": "lines"},
                          "stroke": {"width": 4, "color": "#000"}, "anchor": [0, 0.5], "uppercase": true})),
        scene_with(json!({"type": "counter", "value": {"from": 0, "to": 98.4}, "decimals": 1, "suffix": "%", "separator": "."})),
        scene_with(json!({"type": "captions", "src": "v.words.json", "style": "pop", "highlight": "yellow", "maxWords": 3})),
        scene_with(json!({"type": "captions", "src": "v.words.json", "preset": "karaoke-box", "highlightBox": "marker", "maxLines": 1,
                          "colors": ["#4ade80", "#facc15"], "activeScale": 1.1, "speakerColors": {"SPEAKER_00": "#fff"}})),
        scene_with(json!({"type": "rect", "in": "iris 0.6", "out": {"type": "rotate-in", "angle": 45}})),
        scene_with(json!({"type": "waveform", "src": "a.mp3", "bars": 64, "mirror": true, "gap": 0.3})),
        scene_with(json!({"type": "group", "sequence": true, "overlap": 0.5, "clip": true,
                          "children": [{"type": "rect", "duration": 1}, {"type": "text", "text": "x", "duration": 1, "in": "pop"}]})),
    ];
    for scene in &good {
        let errs = errors(&v, scene);
        assert!(errs.is_empty(), "{scene}: {errs:#?}");
        // and flits itself agrees: parses without warnings
        let (_, warnings) = Scene::from_value(scene.clone(), PathBuf::new()).unwrap();
        assert!(warnings.is_empty(), "{scene}: {warnings:?}");
    }
}

#[test]
fn rejects_common_mistakes() {
    let v = validator();
    let bad = [
        ("typo in a layer key", scene_with(json!({"type": "rect", "colour": "#fff"}))),
        ("typo in a scene key", json!({"width": 10, "height": 10, "durration": 3})),
        ("unknown layer type", scene_with(json!({"type": "txt", "text": "x"}))),
        ("layer without type", scene_with(json!({"text": "x"}))),
        ("text without text", scene_with(json!({"type": "text"}))),
        ("image without src", scene_with(json!({"type": "image"}))),
        ("counter without value", scene_with(json!({"type": "counter"}))),
        ("bad colour", scene_with(json!({"type": "rect", "color": "nope"}))),
        ("bad hex colour", scene_with(json!({"type": "rect", "color": "#12"}))),
        ("unknown ease", scene_with(json!({"type": "rect", "x": [[0, 0], [1, 5, "wobble"]]}))),
        ("unknown transition", scene_with(json!({"type": "rect", "in": "explode"}))),
        ("bad key time", scene_with(json!({"type": "rect", "x": [["later", 0]]}))),
        ("keyframe too long", scene_with(json!({"type": "rect", "x": [[0, 0, "in", 4]]}))),
        ("from without to", scene_with(json!({"type": "rect", "x": {"from": 0}}))),
        ("unknown fit", scene_with(json!({"type": "image", "src": "a.jpg", "fit": "zoom"}))),
        ("unknown captions style", scene_with(json!({"type": "captions", "src": "a.srt", "style": "fancy"}))),
        ("unknown captions preset", scene_with(json!({"type": "captions", "src": "a.srt", "preset": "fancy"}))),
        ("unknown highlight box", scene_with(json!({"type": "captions", "src": "a.srt", "highlightBox": "circle"}))),
        ("duck level above 1", json!({"width": 10, "height": 10, "audio": [{"src": "a.mp3", "duck": 2}]})),
        ("missing height", json!({"width": 10})),
        ("zero width", json!({"width": 0, "height": 10})),
        ("typo in a child", scene_with(json!({"type": "group", "children": [{"type": "rect", "opactiy": 1}]}))),
    ];
    for (what, scene) in &bad {
        assert!(!v.is_valid(scene), "{what} should be rejected: {scene}");
    }
}

/// Every property name and enum value in the schema.
fn schema_names() -> (BTreeSet<String>, BTreeSet<String>) {
    fn walk(v: &Value, props: &mut BTreeSet<String>, enums: &mut BTreeSet<String>) {
        match v {
            Value::Object(o) => {
                if let Some(Value::Object(p)) = o.get("properties") {
                    props.extend(p.keys().cloned());
                }
                if let Some(Value::Array(e)) = o.get("enum") {
                    enums.extend(e.iter().filter_map(Value::as_str).map(str::to_string));
                }
                o.values().for_each(|c| walk(c, props, enums));
            }
            Value::Array(a) => a.iter().for_each(|c| walk(c, props, enums)),
            _ => {}
        }
    }
    let (mut props, mut enums) = (BTreeSet::new(), BTreeSet::new());
    walk(&scene_schema(), &mut props, &mut enums);
    (props, enums)
}

#[test]
fn bindings_cover_the_schema() {
    let (props, enums) = schema_names();
    let ts = std::fs::read_to_string(repo("bindings/typescript/scene.d.ts")).unwrap();
    let py = std::fs::read_to_string(repo("bindings/python/flits/types.py")).unwrap();
    for p in &props {
        let in_ts = ts.contains(&format!("\n  {p}?:")) || ts.contains(&format!("\n  {p}:")) || ts.contains(&format!("\n  \"{p}\"?:"));
        assert!(in_ts, "bindings/typescript/scene.d.ts misses property '{p}'");
        assert!(
            py.contains(&format!("\"{p}\":")) || py.contains(&format!("    {p}:")),
            "bindings/python/flits/types.py misses property '{p}'"
        );
    }
    for e in &enums {
        assert!(ts.contains(&format!("\"{e}\"")), "bindings/typescript/scene.d.ts misses value \"{e}\"");
        assert!(py.contains(&format!("\"{e}\"")), "bindings/python/flits/types.py misses value \"{e}\"");
    }
}
