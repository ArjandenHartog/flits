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
    ("youtube-intro", "16:9 channel intro: iris-revealed logo, channel name, tagline"),
    ("podcast-quote", "9:16 podcast quote: episode label, quote, speaker, animated bars, progress"),
    ("product-promo", "1:1 three-card product promo joined with push transitions, price and button"),
    ("news-headline", "16:9 breaking-news lower third with live badge and scrolling ticker"),
    ("end-screen", "16:9 end screen: two 'watch next' slots, avatar and subscribe button"),
    ("kinetic-typography", "1:1 words hitting the beat, each with its own transition and colours"),
];

/// Deterministic 0..1 noise for the starters' animated bars.
fn noise(i: usize, k: usize) -> f64 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (k as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x % 10_000) as f64 / 10_000.0
}

/// Audio-like bars made of rects: `n` bars in a `w` x `h` box, `secs` long.
fn bars(n: usize, w: f64, h: f64, secs: f64, color: &str) -> Vec<Value> {
    let slot = w / n as f64;
    let step = 0.25;
    (0..n)
        .map(|i| {
            // louder in the middle, like speech
            let env = 1.0 - ((i as f64 + 0.5) / n as f64 - 0.5).abs() * 1.3;
            let keys: Vec<Value> = (0..=(secs / step) as usize)
                .map(|k| json!([k as f64 * step, ((0.12 + 0.88 * noise(i, k) * env) * 100.0).round() / 100.0, "smooth"]))
                .collect();
            json!({"type": "rect", "x": slot * (i as f64 + 0.5), "width": (slot * 0.55).round(), "height": h,
                   "radius": (slot * 0.3).round(), "color": color, "scaleY": keys})
        })
        .collect()
}

/// A rounded button: pill + label.
fn button(label: &str, w: f64, h: f64, bg: &str, fg: &str) -> Vec<Value> {
    vec![
        json!({"type": "rect", "radius": h / 2.0, "color": bg, "shadow": {"blur": 24, "y": 10, "color": "rgba(0,0,0,0.35)"}}),
        json!({"type": "text", "text": label, "size": (h * 0.4).round(), "font": "Segoe UI Black", "color": fg, "letterSpacing": 2,
               "maxWidth": w - h * 0.5}),
    ]
}

pub fn starter(name: &str) -> Result<Value> {
    Ok(match name {
        "intro" => json!({
            "width": 1920, "height": 1080, "fps": 30, "duration": 5, "background": "#0f0b16",
            "layers": [
                {"type": "rect", "gradient": {"type": "linear", "angle": 135, "colors": ["#4b1b51", "#0f0b16", "#123a44"]}},
                {"type": "rect", "x": 1500, "y": 300, "width": 700, "height": 700, "radius": 350, "color": "#f2632822",
                 "scale": {"from": 0.9, "to": 1.15}},
                {"type": "text", "text": "Your title here", "y": 500, "size": 120, "font": "Segoe UI Black",
                 "in": {"type": "slide-up", "duration": 0.8}, "out": {"type": "fade", "duration": 0.6}},
                {"type": "rect", "y": 600, "width": 260, "height": 10, "radius": 5, "color": "#f26328",
                 "scaleX": [[0.4, 0], [1.1, 1, "out"]], "out": "fade"},
                {"type": "text", "text": "A subtitle or date", "y": 670, "size": 46, "color": "rgba(255,255,255,0.8)",
                 "start": 0.6, "in": "fade", "out": "fade"}
            ]
        }),
        "quote" => json!({
            "width": 1080, "height": 1920, "fps": 30, "duration": 7, "background": "#111014",
            "layers": [
                {"type": "rect", "gradient": {"type": "radial", "colors": ["#2b1d3b", "#0d0b10"]}},
                {"type": "text", "text": "\u{201C}", "y": 620, "size": 300, "color": "#f26328", "in": "pop"},
                {"type": "text", "text": "Speed is not a luxury, it is what turns an idea into a video.",
                 "y": 960, "size": 72, "maxWidth": 880, "lineHeight": 1.25, "font": "Segoe UI Black",
                 "reveal": "words", "revealDuration": 2.5},
                {"type": "text", "text": "\u{2014} flits", "y": 1330, "size": 44, "color": "rgba(255,255,255,0.7)", "start": 3, "in": "fade"}
            ]
        }),
        "stats" => json!({
            "width": 1920, "height": 1080, "fps": 30, "duration": 6, "background": "#0d1117",
            "layers": [
                {"type": "text", "text": "Our year in numbers", "y": 200, "size": 80, "font": "Segoe UI Black", "in": "slide-down"},
                {"type": "group", "y": 560, "children": [
                    {"type": "counter", "x": 400, "value": {"from": 0, "to": 12840, "ease": "out", "end": 2.5}, "size": 120, "color": "#4ac5da", "font": "Segoe UI Black"},
                    {"type": "text", "x": 400, "y": 640, "text": "visitors", "size": 40, "color": "rgba(255,255,255,0.7)"},
                    {"type": "counter", "x": 960, "value": {"from": 0, "to": 98.4, "ease": "out", "end": 2.5}, "decimals": 1, "suffix": "%", "size": 120, "color": "#f26328", "font": "Segoe UI Black", "start": 0.3},
                    {"type": "text", "x": 960, "y": 640, "text": "happy", "size": 40, "color": "rgba(255,255,255,0.7)", "start": 0.3},
                    {"type": "counter", "x": 1520, "value": {"from": 0, "to": 37, "ease": "out", "end": 2.5}, "size": 120, "color": "#ffd60a", "font": "Segoe UI Black", "start": 0.6},
                    {"type": "text", "x": 1520, "y": 640, "text": "events", "size": 40, "color": "rgba(255,255,255,0.7)", "start": 0.6}
                ]}
            ]
        }),
        "lower-third" => json!({
            "width": 1920, "height": 1080, "fps": 30, "duration": 6, "background": "#202024",
            "layers": [
                {"type": "group", "x": 140, "y": 860, "anchor": [0, 0.5], "width": 760, "height": 150, "clip": true,
                 "in": {"type": "wipe-right", "duration": 0.6}, "out": {"type": "wipe-left", "duration": 0.5}, "children": [
                    {"type": "rect", "color": "#f26328", "radius": 8},
                    {"type": "text", "text": "Sam Taylor", "x": 40, "y": 55, "anchor": [0, 0.5], "align": "left", "size": 52, "font": "Segoe UI Black"},
                    {"type": "text", "text": "Role or title", "x": 40, "y": 108, "anchor": [0, 0.5], "align": "left", "size": 32, "color": "rgba(255,255,255,0.85)"}
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
        "youtube-intro" => json!({
            "width": 1920, "height": 1080, "fps": 30, "duration": 5, "background": "#0b0d17",
            "layers": [
                {"type": "rect", "gradient": {"type": "linear", "angle": 135, "colors": ["#1a1036", "#0b0d17", "#0d2236"]}},
                {"type": "rect", "x": 1650, "y": 200, "width": 560, "height": 560, "radius": 280, "color": "#ff4d4d14",
                 "scale": {"from": 0.9, "to": 1.2}},
                {"type": "rect", "x": 250, "y": 900, "width": 400, "height": 400, "radius": 200, "color": "#4ac5da12",
                 "scale": {"from": 1.15, "to": 0.95}},
                {"type": "group", "out": {"type": "zoom-fade", "duration": 0.6}, "children": [
                    {"type": "group", "y": 390, "width": 240, "height": 240, "in": {"type": "iris", "duration": 0.7},
                     "scale": [[0, 0.9], [1.2, 1, "out"]], "children": [
                        {"type": "rect", "radius": 120, "gradient": {"type": "linear", "angle": 135, "colors": ["#ff6a3d", "#e0245e"]}},
                        {"type": "text", "text": "YC", "size": 100, "font": "Segoe UI Black"}
                    ]},
                    {"type": "text", "text": "YOUR CHANNEL", "y": 630, "size": 120, "font": "Segoe UI Black", "letterSpacing": 6,
                     "start": 0.5, "in": {"type": "slide-fade", "duration": 0.7}},
                    {"type": "rect", "y": 725, "width": 220, "height": 8, "radius": 4, "color": "#ff6a3d", "start": 0.8,
                     "scaleX": [[0, 0], [0.6, 1, "out"]]},
                    {"type": "text", "text": "New videos every Tuesday", "y": 790, "size": 44, "color": "rgba(255,255,255,0.75)",
                     "start": 1.1, "in": "slide-fade"}
                ]}
            ]
        }),
        "podcast-quote" => json!({
            "width": 1080, "height": 1920, "fps": 30, "duration": 9, "background": "#0b0f19",
            "layers": [
                {"type": "rect", "gradient": {"type": "radial", "colors": ["#22304f", "#0b0f19"]}},
                {"type": "group", "y": 300, "width": 600, "height": 64, "in": "slide-fade", "children": [
                    {"type": "rect", "radius": 32, "color": "#ffb54722", "border": {"width": 2, "color": "#ffb54766"}},
                    {"type": "text", "text": "THE MAKERS PODCAST  \u{00b7}  EP 12", "size": 28, "letterSpacing": 3, "color": "#ffb547"}
                ]},
                {"type": "text", "text": "\u{201C}", "y": 560, "size": 260, "color": "#ffb547", "font": "Georgia Bold", "start": 0.3, "in": "pop"},
                {"type": "text", "text": "The best tools disappear. You stop thinking about them and start thinking about your story.",
                 "y": 880, "size": 68, "maxWidth": 900, "lineHeight": 1.25, "font": "Segoe UI Black", "wrap": "balance",
                 "start": 0.5, "reveal": "words", "revealDuration": 3},
                {"type": "rect", "x": 330, "y": 1270, "width": 120, "height": 120, "radius": 60, "start": 3.6, "in": "iris",
                 "gradient": {"type": "linear", "angle": 135, "colors": ["#ffb547", "#ff5f6d"]}},
                {"type": "text", "text": "AM", "x": 330, "y": 1270, "size": 44, "font": "Segoe UI Black", "color": "#0b0f19",
                 "start": 3.8, "in": "fade"},
                {"type": "text", "text": "Alex Morgan", "x": 420, "y": 1245, "anchor": [0, 0.5], "align": "left", "size": 50,
                 "font": "Segoe UI Black", "start": 3.7, "in": "slide-fade"},
                {"type": "text", "text": "Founder, Studio North", "x": 420, "y": 1305, "anchor": [0, 0.5], "align": "left", "size": 36,
                 "color": "rgba(255,255,255,0.65)", "start": 3.9, "in": "slide-fade"},
                {"type": "group", "y": 1560, "width": 760, "height": 170, "start": 0.2, "in": "fade", "children": bars(28, 760.0, 170.0, 9.0, "#ffb547")},
                {"type": "rect", "x": 160, "y": 1780, "anchor": [0, 0.5], "width": 760, "height": 8, "radius": 4, "color": "rgba(255,255,255,0.15)"},
                {"type": "rect", "x": 160, "y": 1780, "anchor": [0, 0.5], "width": 760, "height": 8, "radius": 4, "color": "#ffb547",
                 "scaleX": {"from": 0, "to": 1}}
            ]
        }),
        "product-promo" => json!({
            "width": 1080, "height": 1080, "fps": 30, "background": "#111111",
            "layers": [
                {"type": "group", "sequence": true, "overlap": 0.6, "children": [
                    {"type": "group", "duration": 3.4, "in": "fade", "out": "push-left", "children": [
                        {"type": "rect", "gradient": {"type": "linear", "angle": 160, "colors": ["#3b1d6e", "#140b26"]}},
                        {"type": "rect", "y": 420, "width": 330, "height": 470, "radius": 60, "rotation": -8, "in": {"type": "rotate-in", "duration": 0.8, "angle": 40},
                         "gradient": {"type": "linear", "angle": 150, "colors": ["#c084fc", "#6d28d9"]}, "shadow": {"blur": 50, "y": 30, "color": "rgba(0,0,0,0.5)"}},
                        {"type": "rect", "y": 420, "width": 150, "height": 150, "radius": 75, "rotation": -8, "color": "rgba(255,255,255,0.18)",
                         "start": 0.4, "in": "pop"},
                        {"type": "text", "text": "Meet Nova", "y": 790, "size": 110, "font": "Segoe UI Black", "start": 0.5, "in": "slide-fade"},
                        {"type": "text", "text": "The speaker that listens less.", "y": 880, "size": 42, "color": "rgba(255,255,255,0.75)",
                         "start": 0.8, "in": "slide-fade"}
                    ]},
                    {"type": "group", "duration": 3.4, "in": "push-left", "out": "push-left", "children": [
                        {"type": "rect", "gradient": {"type": "linear", "angle": 160, "colors": ["#0f766e", "#062925"]}},
                        {"type": "text", "text": "Why you'll love it", "y": 260, "size": 76, "font": "Segoe UI Black", "start": 0.4, "in": "slide-fade"},
                        {"type": "group", "y": 450, "height": 100, "start": 0.7, "in": "slide-fade", "children": [
                            {"type": "rect", "x": 230, "width": 64, "height": 64, "radius": 32, "color": "#5eead4"},
                            {"type": "text", "text": "30-hour battery", "x": 290, "anchor": [0, 0.5], "align": "left", "size": 56}
                        ]},
                        {"type": "group", "y": 590, "height": 100, "start": 0.9, "in": "slide-fade", "children": [
                            {"type": "rect", "x": 230, "width": 64, "height": 64, "radius": 32, "color": "#5eead4"},
                            {"type": "text", "text": "Studio-grade sound", "x": 290, "anchor": [0, 0.5], "align": "left", "size": 56}
                        ]},
                        {"type": "group", "y": 730, "height": 100, "start": 1.1, "in": "slide-fade", "children": [
                            {"type": "rect", "x": 230, "width": 64, "height": 64, "radius": 32, "color": "#5eead4"},
                            {"type": "text", "text": "Private by design", "x": 290, "anchor": [0, 0.5], "align": "left", "size": 56}
                        ]}
                    ]},
                    {"type": "group", "duration": 3.6, "in": "push-left", "out": "fade", "children": [
                        {"type": "rect", "gradient": {"type": "linear", "angle": 160, "colors": ["#f97316", "#9a2c0b"]}},
                        {"type": "text", "text": "From", "y": 330, "size": 56, "color": "rgba(255,255,255,0.8)", "start": 0.4, "in": "fade"},
                        {"type": "counter", "y": 470, "value": {"from": 0, "to": 49, "ease": "out", "start": 0.5, "end": 1.6}, "prefix": "$",
                         "size": 210, "font": "Segoe UI Black"},
                        {"type": "group", "y": 700, "width": 420, "height": 110, "start": 1.2, "in": "pop",
                         "scale": [[1.6, 1], [1.9, 1.06, "smooth"], [2.2, 1, "smooth"], [2.5, 1.06, "smooth"], [2.8, 1, "smooth"]],
                         "children": button("SHOP NOW", 420.0, 110.0, "#ffffff", "#9a2c0b")},
                        {"type": "text", "text": "nova.example.com", "y": 830, "size": 36, "color": "rgba(255,255,255,0.75)", "start": 1.5, "in": "fade"}
                    ]}
                ]}
            ]
        }),
        "news-headline" => json!({
            "width": 1920, "height": 1080, "fps": 30, "duration": 8, "background": "#04070f",
            "layers": [
                {"type": "rect", "gradient": {"type": "linear", "angle": 120, "colors": ["#0d2350", "#04070f"]}},
                {"type": "rect", "x": 1450, "y": 380, "width": 900, "height": 900, "radius": 450, "color": "#ffffff06", "border": {"width": 3, "color": "#ffffff10"},
                 "rotation": {"from": 0, "to": 20}},
                {"type": "rect", "x": 1450, "y": 380, "width": 600, "height": 600, "radius": 300, "color": "#00000000", "border": {"width": 3, "color": "#ffffff0d"}},
                {"type": "group", "x": 110, "y": 90, "anchor": [0, 0.5], "width": 150, "height": 54, "in": "pop", "children": [
                    {"type": "rect", "radius": 10, "color": "#e11d2e"},
                    {"type": "rect", "x": 32, "width": 16, "height": 16, "radius": 8, "color": "#ffffff",
                     "opacity": [[0, 1], [0.5, 0.2, "smooth"], [1, 1, "smooth"], [1.5, 0.2, "smooth"], [2, 1, "smooth"], [2.5, 0.2, "smooth"],
                                 [3, 1, "smooth"], [3.5, 0.2, "smooth"], [4, 1, "smooth"], [4.5, 0.2, "smooth"], [5, 1, "smooth"]]},
                    {"type": "text", "text": "LIVE", "x": 92, "size": 30, "font": "Segoe UI Black"}
                ]},
                {"type": "text", "text": "09:41", "x": 1810, "y": 90, "anchor": [1, 0.5], "size": 40, "font": "Segoe UI Semibold",
                 "color": "rgba(255,255,255,0.8)", "in": "fade"},
                {"type": "group", "x": 110, "y": 700, "anchor": [0, 0.5], "width": 470, "height": 76, "clip": true, "start": 0.4,
                 "in": {"type": "wipe-right", "duration": 0.5}, "children": [
                    {"type": "rect", "color": "#e11d2e"},
                    {"type": "text", "text": "BREAKING NEWS", "x": 30, "anchor": [0, 0.5], "align": "left", "size": 42, "font": "Segoe UI Black",
                     "letterSpacing": 2}
                ]},
                {"type": "group", "x": 110, "y": 818, "anchor": [0, 0.5], "width": 1700, "height": 140, "clip": true, "start": 0.7,
                 "in": {"type": "wipe-right", "duration": 0.7}, "children": [
                    {"type": "rect", "color": "#ffffff"},
                    {"type": "text", "text": "Local team ships a video renderer 50x faster", "x": 30, "anchor": [0, 0.5], "align": "left",
                     "size": 74, "font": "Segoe UI Black", "color": "#0b1020", "start": 0.3, "in": "slide-fade"}
                ]},
                {"type": "group", "x": 110, "y": 925, "anchor": [0, 0.5], "width": 1700, "height": 64, "start": 1.3, "in": "slide-fade", "children": [
                    {"type": "rect", "color": "rgba(10,16,32,0.85)"},
                    {"type": "text", "text": "Benchmarks show 10,000 frames rendered in under a minute", "x": 30, "anchor": [0, 0.5], "align": "left",
                     "size": 36, "color": "rgba(255,255,255,0.9)"}
                ]},
                {"type": "group", "y": 1035, "height": 70, "clip": true, "children": [
                    {"type": "rect", "color": "#0a0d16"},
                    {"type": "text", "x": [[0, 1300], ["end", -900, "linear"]], "anchor": [0, 0.5], "align": "left", "size": 32,
                     "color": "rgba(255,255,255,0.85)",
                     "text": "MARKETS  +2.4%   \u{2022}   WEATHER  sunny, 24\u{00b0}C   \u{2022}   SPORTS  city wins the final 3-1   \u{2022}   TECH  new GPU drivers out today"},
                    {"type": "group", "x": 0, "anchor": [0, 0.5], "width": 290, "height": 70, "children": [
                        {"type": "rect", "color": "#e11d2e"},
                        {"type": "text", "text": "TOP STORIES", "size": 30, "font": "Segoe UI Black"}
                    ]}
                ]}
            ]
        }),
        "end-screen" => json!({
            "width": 1920, "height": 1080, "fps": 30, "duration": 8, "background": "#0e0e12",
            "layers": [
                {"type": "rect", "gradient": {"type": "radial", "colors": ["#23233a", "#0e0e12"]}},
                {"type": "text", "text": "Thanks for watching!", "y": 150, "size": 84, "font": "Segoe UI Black", "in": "slide-fade"},
                {"type": "group", "x": 540, "y": 510, "width": 780, "height": 440, "start": 0.3, "in": {"type": "flip-x", "duration": 0.6},
                 "children": [
                    {"type": "rect", "radius": 24, "gradient": {"type": "linear", "angle": 135, "colors": ["#334155", "#1e293b"]},
                     "border": {"width": 4, "color": "#ffffff22"}, "shadow": {"blur": 40, "y": 20}},
                    {"type": "rect", "width": 120, "height": 120, "radius": 60, "color": "rgba(255,255,255,0.18)"},
                    {"type": "rect", "width": 44, "height": 44, "radius": 22, "color": "#ffffff"},
                    {"type": "text", "text": "Watch next", "x": 36, "y": 50, "anchor": [0, 0.5], "align": "left", "size": 36, "font": "Segoe UI Black"}
                ]},
                {"type": "group", "x": 1380, "y": 510, "width": 780, "height": 440, "start": 0.5, "in": {"type": "flip-x", "duration": 0.6},
                 "children": [
                    {"type": "rect", "radius": 24, "gradient": {"type": "linear", "angle": 135, "colors": ["#4c1d95", "#1e1b4b"]},
                     "border": {"width": 4, "color": "#ffffff22"}, "shadow": {"blur": 40, "y": 20}},
                    {"type": "rect", "width": 120, "height": 120, "radius": 60, "color": "rgba(255,255,255,0.18)"},
                    {"type": "rect", "width": 44, "height": 44, "radius": 22, "color": "#ffffff"},
                    {"type": "text", "text": "Most popular", "x": 36, "y": 50, "anchor": [0, 0.5], "align": "left", "size": 36, "font": "Segoe UI Black"}
                ]},
                {"type": "group", "x": 560, "y": 900, "width": 150, "height": 150, "start": 0.9, "in": "iris", "children": [
                    {"type": "rect", "radius": 75, "gradient": {"type": "linear", "angle": 135, "colors": ["#ff6a3d", "#e0245e"]}},
                    {"type": "text", "text": "YC", "size": 60, "font": "Segoe UI Black"}
                ]},
                {"type": "text", "text": "@yourchannel", "x": 665, "y": 878, "anchor": [0, 0.5], "align": "left", "size": 52, "font": "Segoe UI Black",
                 "start": 1.1, "in": "slide-fade"},
                {"type": "text", "text": "New videos every Tuesday", "x": 665, "y": 935, "anchor": [0, 0.5], "align": "left", "size": 34,
                 "color": "rgba(255,255,255,0.6)", "start": 1.2, "in": "slide-fade"},
                {"type": "group", "x": 1500, "y": 900, "width": 420, "height": 110, "start": 1.4, "in": "pop",
                 "scale": [[1, 1], [1.4, 1.06, "smooth"], [1.8, 1, "smooth"], [3, 1], [3.4, 1.06, "smooth"], [3.8, 1, "smooth"],
                           [5, 1], [5.4, 1.06, "smooth"], [5.8, 1, "smooth"]],
                 "children": button("SUBSCRIBE", 420.0, 110.0, "#e11d2e", "#ffffff")}
            ]
        }),
        "kinetic-typography" => {
            // (word, background, text colour, transition, size)
            let beats = [
                ("THIS", "#111111", "#ffffff", "pop", 260),
                ("IS", "#111111", "#ffd60a", "slide-up", 320),
                ("KINETIC", "#ffd60a", "#111111", "zoom-fade", 200),
                ("TYPE.", "#ffd60a", "#111111", "rotate-in", 270),
                ("EVERY", "#ff3b6b", "#ffffff", "push-left", 230),
                ("WORD", "#ff3b6b", "#111111", "flip-x", 270),
                ("HITS", "#111111", "#ff3b6b", "pop", 320),
                ("THE", "#111111", "#ffffff", "slide-down", 300),
                ("BEAT.", "#4ac5da", "#111111", "iris", 270),
                ("MAKE", "#4ac5da", "#ffffff", "push-up", 260),
                ("IT", "#ffffff", "#111111", "zoom-in", 380),
            ];
            let mut children: Vec<Value> = beats
                .iter()
                .map(|(word, bg, fg, tr, size)| {
                    json!({"type": "group", "duration": 0.5, "children": [
                        {"type": "rect", "color": bg},
                        {"type": "text", "text": word, "size": size, "font": "Segoe UI Black", "color": fg,
                         "in": {"type": tr, "duration": 0.25}}
                    ]})
                })
                .collect();
            children.push(json!({"type": "group", "duration": 1.6, "children": [
                {"type": "rect", "color": "#111111"},
                {"type": "text", "text": "MOVE.", "size": 270, "font": "Segoe UI Black", "color": "#ffd60a", "in": "pop",
                 "scale": [[0.4, 1], [1.6, 1.08, "linear"]]},
                {"type": "rect", "y": 720, "width": 420, "height": 14, "radius": 7, "color": "#ffd60a", "start": 0.3,
                 "scaleX": [[0, 0], [0.4, 1, "out"]]}
            ]}));
            json!({
                "width": 1080, "height": 1080, "fps": 30, "background": "#111111",
                "layers": [{"type": "group", "sequence": true, "children": children}]
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
