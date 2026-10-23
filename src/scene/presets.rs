//! Named caption looks (`"preset": "tiktok"` on a captions layer). A preset is a set of
//! default layer properties merged under the layer before parsing: anything the layer sets
//! itself wins (objects such as `stroke` are merged key by key).

use anyhow::{bail, Result};
use serde_json::{json, Map, Value};

pub const CAPTION_PRESETS: &[(&str, &str)] = &[
    ("tiktok", "bold uppercase words that pop in, yellow active word, thick black outline"),
    ("karaoke-box", "white words, the active word sits on a coloured pill that glides along"),
    ("bounce", "words drop in with a bounce, active word in cycling colours"),
    ("minimal", "clean white sentence-case lines with a soft shadow"),
    ("neon", "glowing uppercase words, the active word lights up pink"),
    ("highlighter", "the active word gets a marker stroke behind it"),
    ("subtitle", "classic two-line subtitles on semi-transparent boxes (put it near the bottom with \"y\")"),
    ("word-pop-big", "one huge word at a time, colours cycling"),
];

/// Base caption size for a canvas: a little larger on vertical video.
fn base_size(w: f64, h: f64) -> f64 {
    (w.min(h) * if h > w { 0.08 } else { 0.06 }).round()
}

/// The layer properties of a preset. `size` = the layer's own size, if it sets one
/// (outline and shadow widths follow the size actually used).
pub fn caption_preset(name: &str, canvas: (f64, f64), size: Option<f64>) -> Result<Value> {
    let base = base_size(canvas.0, canvas.1);
    let s = |k: f64| size.unwrap_or((base * k).round());
    let px = |v: f64| (v * 10.0).round() / 10.0;
    let black = "#000000";
    Ok(match name {
        "tiktok" => {
            let s = s(1.0);
            json!({"style": "pop", "font": "Segoe UI Black", "uppercase": true, "size": s, "highlight": "#ffd60a",
                   "stroke": {"width": px(s * 0.1), "color": black},
                   "shadow": {"color": "rgba(0,0,0,0.55)", "blur": px(s * 0.1), "x": 0, "y": px(s * 0.05)},
                   "maxWords": 3, "activeScale": 1.08})
        }
        "karaoke-box" => {
            let s = s(0.95);
            json!({"style": "karaoke", "font": "Segoe UI Black", "size": s, "color": "#ffffff", "highlight": "#ffffff",
                   "highlightBg": "#7c3aed", "highlightBox": "pill", "maxWords": 4, "activeScale": 1.06,
                   "stroke": {"width": px(s * 0.06), "color": "#000000"},
                   "shadow": {"color": "rgba(0,0,0,0.6)", "blur": px(s * 0.12), "x": 0, "y": px(s * 0.04)}})
        }
        "bounce" => {
            let s = s(1.0);
            json!({"style": "bounce", "font": "Segoe UI Black", "uppercase": true, "size": s,
                   "colors": ["#4ade80", "#facc15", "#f472b6", "#38bdf8"], "activeScale": 1.05, "maxWords": 3,
                   "stroke": {"width": px(s * 0.09), "color": black},
                   "shadow": {"color": "rgba(0,0,0,0.5)", "blur": px(s * 0.1), "x": 0, "y": px(s * 0.05)}})
        }
        "minimal" => {
            let s = s(0.8);
            json!({"style": "line", "font": "Segoe UI Semibold", "size": s, "color": "#ffffff", "maxWords": 8, "lineHeight": 1.2,
                   "wrap": "balance",
                   "shadow": {"color": "rgba(0,0,0,0.55)", "blur": px(s * 0.2), "x": 0, "y": px(s * 0.05)}})
        }
        "neon" => {
            let s = s(1.0);
            json!({"style": "karaoke", "font": "Segoe UI Black", "uppercase": true, "size": s, "color": "#e6fdff",
                   "highlight": "#ff4fd8", "maxWords": 4, "activeScale": 1.08,
                   "stroke": {"width": px(s * 0.03), "color": "#22d3ee"},
                   "shadow": {"color": "#00e5ff", "blur": px(s * 0.14), "x": 0, "y": 0}})
        }
        "highlighter" => {
            let s = s(1.0);
            json!({"style": "karaoke", "font": "Segoe UI Black", "size": s, "color": "#ffffff", "highlight": "#111111",
                   "highlightBg": "#ffe14d", "highlightBox": "marker", "maxWords": 4,
                   "shadow": {"color": "rgba(0,0,0,0.7)", "blur": px(s * 0.08), "x": 0, "y": px(s * 0.04)}})
        }
        "subtitle" => {
            let s = s(0.7);
            json!({"style": "line", "font": "Segoe UI Semibold", "size": s, "color": "#ffffff", "maxWords": 16, "maxLines": 2,
                   "lineHeight": 1.4, "wrap": "balance",
                   "background": {"color": "rgba(0,0,0,0.62)", "padding": [px(s * 0.35), px(s * 0.02)], "radius": px(s * 0.14), "mode": "lines"},
                   "shadow": {"color": "rgba(0,0,0,0.4)", "blur": px(s * 0.06), "x": 0, "y": px(s * 0.03)}})
        }
        "word-pop-big" => {
            let s = s(2.2);
            json!({"style": "word", "font": "Segoe UI Black", "uppercase": true, "size": s,
                   "colors": ["#ffffff", "#ffd60a", "#4ade80", "#ff4fd8"],
                   "stroke": {"width": px(s * 0.07), "color": black},
                   "shadow": {"color": "rgba(0,0,0,0.6)", "blur": px(s * 0.08), "x": 0, "y": px(s * 0.04)}})
        }
        o => bail!("unknown captions preset '{o}' (one of: {})", CAPTION_PRESETS.iter().map(|p| p.0).collect::<Vec<_>>().join(", ")),
    })
}

/// `over` wins; objects present on both sides are merged recursively.
pub fn merge(under: &Value, over: &Value) -> Value {
    match (under, over) {
        (Value::Object(a), Value::Object(b)) => {
            let mut out: Map<String, Value> = a.clone();
            for (k, v) in b {
                let merged = match a.get(k) {
                    Some(av) => merge(av, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        _ => over.clone(),
    }
}

/// Expand `preset` on every captions layer (recursively through groups).
pub fn apply(layers: &mut [Value], canvas: (f64, f64), path: &str, warnings: &mut Vec<String>) -> Result<()> {
    for (i, l) in layers.iter_mut().enumerate() {
        let here = format!("{path}.{i}");
        if let Some(name) = l.get("preset").and_then(Value::as_str).map(str::to_string) {
            if l.get("type").and_then(Value::as_str) == Some("captions") {
                let size = l.get("size").and_then(Value::as_f64);
                let p = caption_preset(&name, canvas, size).map_err(|e| e.context(here.clone()))?;
                *l = merge(&p, l);
            } else {
                warnings.push(format!("{here}: \"preset\" only applies to captions layers (ignored)"));
            }
        }
        if let Some(Value::Array(children)) = l.get_mut("children") {
            apply(children, canvas, &format!("{here}.children"), warnings)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scene;

    #[test]
    fn layer_values_override_preset_values() {
        let layer = json!({"type": "captions", "preset": "tiktok", "highlight": "#ff0000", "stroke": {"width": 3}});
        let p = caption_preset("tiktok", (1080.0, 1920.0), None).unwrap();
        let m = merge(&p, &layer);
        assert_eq!(m["highlight"], "#ff0000", "explicit value wins");
        assert_eq!(m["style"], "pop", "preset fills in the rest");
        assert_eq!(m["stroke"]["width"], 3.0, "nested keys merge");
        assert_eq!(m["stroke"]["color"], "#000000");
        assert_eq!(m["type"], "captions");
    }

    #[test]
    fn sizes_follow_canvas_and_layer() {
        let v = caption_preset("tiktok", (1080.0, 1920.0), None).unwrap();
        assert_eq!(v["size"], 86.0);
        let v = caption_preset("tiktok", (1920.0, 1080.0), Some(100.0)).unwrap();
        assert_eq!(v["size"], 100.0);
        assert_eq!(v["stroke"]["width"], 10.0, "outline follows the layer's own size");
    }

    #[test]
    fn every_preset_parses_cleanly_in_a_scene() {
        for (name, _) in CAPTION_PRESETS {
            let scene = json!({"width": 1080, "height": 1920, "layers": [
                {"type": "group", "children": [{"type": "captions", "src": "t.json", "preset": name}]}
            ]});
            let (s, w) = Scene::from_value(scene, Default::default()).unwrap();
            assert!(w.is_empty(), "{name}: {w:?}");
            let l = &s.layers[0].children[0];
            assert!(l.style.is_some() && l.size.is_some(), "{name} was expanded inside a group");
        }
        let bad = json!({"width": 100, "height": 100, "layers": [{"type": "captions", "src": "t.json", "preset": "nope"}]});
        assert!(Scene::from_value(bad, Default::default()).is_err());
        let misplaced = json!({"width": 100, "height": 100, "layers": [{"type": "rect", "preset": "tiktok"}]});
        let (_, w) = Scene::from_value(misplaced, Default::default()).unwrap();
        assert!(w.iter().any(|w| w.contains("only applies to captions")));
    }
}
