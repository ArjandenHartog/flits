//! `flits new <starter>`: small, self-contained example scenes (no assets needed) to
//! start from. They double as documentation of the scene format.

use anyhow::{bail, Result};
use serde_json::{json, Value};

pub const STARTERS: &[(&str, &str)] = &[
    ("intro", "16:9 title intro: gradient, sliding title, accent line, subtitle"),
    ("quote", "9:16 quote card for reels/stories with word-by-word reveal"),
    ("stats", "16:9 animated numbers (counters) for results or year-in-review"),
    ("lower-third", "transparent-style name/role banner over a dark background"),
    ("countdown", "1:1 five second countdown with pop-in numbers"),
];

pub fn starter(name: &str) -> Result<Value> {
    Ok(match name {
        "intro" => json!({
            "width": 1920, "height": 1080, "fps": 30, "duration": 5, "background": "#0f0b16",
            "layers": [
                {"type": "rect", "gradient": {"type": "linear", "angle": 135, "colors": ["#4b1b51", "#0f0b16", "#123a44"]}},
                {"type": "rect", "x": 1500, "y": 300, "width": 700, "height": 700, "radius": 350, "color": "#f2632822",
                 "scale": {"from": 0.9, "to": 1.15}},
                {"type": "text", "text": "Jouw titel hier", "y": 500, "size": 120, "font": "Segoe UI Black",
                 "in": {"type": "slide-up", "duration": 0.8}, "out": {"type": "fade", "duration": 0.6}},
                {"type": "rect", "y": 600, "width": 260, "height": 10, "radius": 5, "color": "#f26328",
                 "scaleX": [[0.4, 0], [1.1, 1, "out"]], "out": "fade"},
                {"type": "text", "text": "Een ondertitel of datum", "y": 670, "size": 46, "color": "rgba(255,255,255,0.8)",
                 "start": 0.6, "in": "fade", "out": "fade"}
            ]
        }),
        "quote" => json!({
            "width": 1080, "height": 1920, "fps": 30, "duration": 7, "background": "#111014",
            "layers": [
                {"type": "rect", "gradient": {"type": "radial", "colors": ["#2b1d3b", "#0d0b10"]}},
                {"type": "text", "text": "\u{201C}", "y": 620, "size": 300, "color": "#f26328", "in": "pop"},
                {"type": "text", "text": "Snelheid is geen luxe, het is wat het verschil maakt tussen een idee en een video.",
                 "y": 960, "size": 72, "maxWidth": 880, "lineHeight": 1.25, "font": "Segoe UI Black",
                 "reveal": "words", "revealDuration": 2.5},
                {"type": "text", "text": "\u{2014} flits", "y": 1330, "size": 44, "color": "rgba(255,255,255,0.7)", "start": 3, "in": "fade"}
            ]
        }),
        "stats" => json!({
            "width": 1920, "height": 1080, "fps": 30, "duration": 6, "background": "#0d1117",
            "layers": [
                {"type": "text", "text": "Ons jaar in cijfers", "y": 200, "size": 80, "font": "Segoe UI Black", "in": "slide-down"},
                {"type": "group", "y": 560, "children": [
                    {"type": "counter", "x": 400, "value": {"from": 0, "to": 12840, "ease": "out", "end": 2.5}, "size": 120, "color": "#4ac5da", "font": "Segoe UI Black"},
                    {"type": "text", "x": 400, "y": 640, "text": "bezoekers", "size": 40, "color": "rgba(255,255,255,0.7)"},
                    {"type": "counter", "x": 960, "value": {"from": 0, "to": 98.4, "ease": "out", "end": 2.5}, "decimals": 1, "suffix": "%", "size": 120, "color": "#f26328", "font": "Segoe UI Black", "start": 0.3},
                    {"type": "text", "x": 960, "y": 640, "text": "tevreden", "size": 40, "color": "rgba(255,255,255,0.7)", "start": 0.3},
                    {"type": "counter", "x": 1520, "value": {"from": 0, "to": 37, "ease": "out", "end": 2.5}, "size": 120, "color": "#ffd60a", "font": "Segoe UI Black", "start": 0.6},
                    {"type": "text", "x": 1520, "y": 640, "text": "activiteiten", "size": 40, "color": "rgba(255,255,255,0.7)", "start": 0.6}
                ]}
            ]
        }),
        "lower-third" => json!({
            "width": 1920, "height": 1080, "fps": 30, "duration": 6, "background": "#202024",
            "layers": [
                {"type": "group", "x": 140, "y": 860, "anchor": [0, 0.5], "width": 760, "height": 150, "clip": true,
                 "in": {"type": "wipe-right", "duration": 0.6}, "out": {"type": "wipe-left", "duration": 0.5}, "children": [
                    {"type": "rect", "color": "#f26328", "radius": 8},
                    {"type": "text", "text": "Arjan den Hartog", "x": 40, "y": 55, "anchor": [0, 0.5], "align": "left", "size": 52, "font": "Segoe UI Black"},
                    {"type": "text", "text": "Bedenker van flits", "x": 40, "y": 108, "anchor": [0, 0.5], "align": "left", "size": 32, "color": "rgba(255,255,255,0.85)"}
                ]}
            ]
        }),
        "countdown" => {
            let nums: Vec<Value> = (1..=5)
                .rev()
                .map(|n| json!({"type": "text", "text": n.to_string(), "duration": 1, "size": 420, "font": "Segoe UI Black", "in": "pop", "out": {"type": "zoom-in", "duration": 0.3}}))
                .collect();
            json!({
                "width": 1080, "height": 1080, "fps": 30, "background": "#101014",
                "layers": [
                    {"type": "rect", "width": 760, "height": 760, "radius": 380, "color": "#f2632833", "scale": [[0, 1], [0.5, 1.08, "smooth"], [1, 1, "smooth"]]},
                    {"type": "group", "sequence": true, "children": nums}
                ]
            })
        }
        o => bail!("unknown starter '{o}' (one of: {})", STARTERS.iter().map(|s| s.0).collect::<Vec<_>>().join(", ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_parse_without_warnings() {
        for (name, _) in STARTERS {
            let (_, w) = crate::Scene::from_value(starter(name).unwrap(), Default::default()).unwrap();
            assert!(w.is_empty(), "{name}: {w:?}");
        }
    }
}
