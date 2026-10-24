//! Scenes written in JavaScript/TypeScript: run through the loader with the SDK from
//! bindings/typescript (skipped when no JavaScript runtime is installed).

use flits::scene::script;
use flits::{Prepared, Scene};
use std::path::{Path, PathBuf};

/// file:// URL of the SDK in this repository.
fn sdk() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bindings/typescript/index.js");
    let p = p.display().to_string().replace('\\', "/").replace(' ', "%20");
    format!("file:///{}", p.trim_start_matches('/'))
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("flits-tests").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn skip(path: &Path) -> bool {
    if script::can_run(path) {
        return false;
    }
    eprintln!("no JavaScript runtime for {}: skipping", path.display());
    true
}

#[test]
fn module_scene_with_components_props_and_function_props() {
    let d = dir("script-mjs");
    let scene = d.join("video.mjs");
    std::fs::write(
        &scene,
        format!(
            r##"import {{ Composition, Sequence, Text, Shape, spring, formats }} from "{}";
export default ({{ title = "default" }}) => Composition({{
  ...formats.hd, fps: 25, duration: 2,
  children: [
    Sequence({{ children: [
      Text({{ text: title, duration: 1, scale: (t) => spring({{ t }}) }}),
      Shape({{ shape: "circle", duration: 1, stroke: {{ width: 8, color: "#fff" }}, progress: (t) => t }}),
    ] }}),
  ],
}});
"##,
            sdk()
        ),
    )
    .unwrap();
    if skip(&scene) {
        return;
    }
    script::set_props(serde_json::json!({"title": "from props"}));
    let (s, warnings) = Scene::load(&scene).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!((s.width, s.height, s.fps), (1280, 720, 25.0));
    let seq = &s.layers[0];
    assert_eq!(seq.children[0].text.as_deref(), Some("from props"));
    let p = Prepared::build(&s, 0.25).unwrap();
    assert_eq!(p.frames, 50);
}

#[test]
fn typescript_scene_and_errors() {
    let d = dir("script-ts");
    let scene = d.join("video.ts");
    std::fs::write(
        &scene,
        format!(
            r#"import {{ Composition, Text, type Scene }} from "{}";
const words: string[] = ["a", "b", "c"];
const scene: Scene = Composition({{ width: 100, height: 100, duration: 3, children: words.map((w, i) => Text({{ text: w, start: i, duration: 1 }})) }});
console.log("hello from the script");
export default scene;
"#,
            sdk()
        ),
    )
    .unwrap();
    if skip(&scene) {
        return;
    }
    let (s, _) = Scene::load(&scene).unwrap();
    assert_eq!(s.layers.len(), 3);
    assert_eq!(s.layers[2].start, 2.0);

    let bad = d.join("bad.ts");
    std::fs::write(&bad, "throw new Error('boom in the scene');\n").unwrap();
    let err = format!("{:#}", Scene::load(&bad).unwrap_err());
    assert!(err.contains("boom in the scene"), "{err}");

    let none = d.join("none.ts");
    std::fs::write(&none, "export const x = 1;\n").unwrap();
    let err = format!("{:#}", Scene::load(&none).unwrap_err());
    assert!(err.contains("no scene exported"), "{err}");
}
