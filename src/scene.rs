//! Scene description (the JSON format) and validation.

use crate::anim::{Prop, Transition};
use crate::color::Rgba;
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn default_fps() -> f64 {
    30.0
}
fn one() -> f64 {
    1.0
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: f64,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub background: Option<Rgba>,
    #[serde(default)]
    pub fonts: HashMap<String, String>,
    #[serde(default)]
    pub audio: Vec<AudioTrack>,
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// Directory relative paths are resolved against (not part of the JSON).
    #[serde(skip)]
    pub base_dir: PathBuf,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    pub src: String,
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub trim: f64,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default = "one")]
    pub volume: f64,
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
    #[serde(default, rename = "loop")]
    pub looped: bool,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Border {
    pub width: f64,
    pub color: Rgba,
}

fn shadow_color() -> Rgba {
    Rgba::rgba8(0, 0, 0, 0.5)
}
fn shadow_blur() -> f64 {
    24.0
}
fn shadow_y() -> f64 {
    12.0
}

#[derive(Deserialize, Clone, Debug)]
pub struct Shadow {
    #[serde(default = "shadow_color")]
    pub color: Rgba,
    #[serde(default = "shadow_blur")]
    pub blur: f64,
    #[serde(default)]
    pub x: f64,
    #[serde(default = "shadow_y")]
    pub y: f64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct StrokeSpec {
    pub width: f64,
    pub color: Rgba,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Padding {
    Same(f64),
    Xy([f64; 2]),
}

impl Padding {
    pub fn xy(&self) -> (f64, f64) {
        match self {
            Padding::Same(v) => (*v, *v),
            Padding::Xy([x, y]) => (*x, *y),
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct TextBg {
    pub color: Rgba,
    #[serde(default)]
    pub padding: Option<Padding>,
    #[serde(default)]
    pub radius: Option<f64>,
    /// "box" (one box around the block) or "lines" (one pill per line)
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Gradient {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// CSS-style angle in degrees: 180 = top to bottom, 90 = left to right
    #[serde(default)]
    pub angle: Option<f64>,
    #[serde(default)]
    pub stops: Vec<(f64, Rgba)>,
    #[serde(default)]
    pub colors: Vec<Rgba>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Layer {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Option<String>,
    pub start: f64,
    pub end: Option<f64>,
    pub duration: Option<f64>,

    pub x: Option<Prop>,
    pub y: Option<Prop>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub anchor: Option<[f64; 2]>,
    pub scale: Option<Prop>,
    pub scale_x: Option<Prop>,
    pub scale_y: Option<Prop>,
    pub rotation: Option<Prop>,
    pub opacity: Option<Prop>,
    pub zoom: Option<Prop>,
    pub pan_x: Option<Prop>,
    pub pan_y: Option<Prop>,
    #[serde(rename = "in")]
    pub enter: Option<Transition>,
    #[serde(rename = "out")]
    pub exit: Option<Transition>,
    pub radius: f64,
    pub border: Option<Border>,
    pub shadow: Option<Shadow>,

    // image / video / audio-carrying layers
    pub src: Option<String>,
    pub fit: Option<String>,
    pub blur: f64,
    pub trim: f64,
    pub volume: Option<f64>,
    #[serde(rename = "loop")]
    pub looped: bool,
    pub fade_in: f64,
    pub fade_out: f64,

    // rect
    pub color: Option<Rgba>,
    pub gradient: Option<Gradient>,

    // text
    pub text: Option<String>,
    pub font: Option<String>,
    pub size: Option<f64>,
    pub weight: Option<f64>,
    pub align: Option<String>,
    pub max_width: Option<f64>,
    pub line_height: Option<f64>,
    pub letter_spacing: f64,
    pub stroke: Option<StrokeSpec>,
    pub background: Option<TextBg>,
    pub uppercase: bool,
    pub reveal: Option<String>,
    pub reveal_duration: Option<f64>,

    // captions
    pub style: Option<String>,
    pub highlight: Option<Rgba>,
    pub highlight_bg: Option<Rgba>,
    pub max_words: Option<usize>,

    // waveform
    pub bars: Option<usize>,
    pub mirror: bool,
    pub gap: Option<f64>,
    pub gain: Option<f64>,

    // group
    pub children: Vec<Layer>,
    pub sequence: bool,
    pub overlap: f64,
}

const SCENE_KEYS: &[&str] = &["width", "height", "fps", "duration", "background", "fonts", "audio", "layers", "$schema", "comment"];
const AUDIO_KEYS: &[&str] = &["src", "start", "trim", "duration", "volume", "fadeIn", "fadeOut", "loop", "comment"];
const LAYER_KEYS: &[&str] = &[
    "type", "id", "start", "end", "duration", "x", "y", "width", "height", "anchor", "scale", "scaleX", "scaleY",
    "rotation", "opacity", "zoom", "panX", "panY", "in", "out", "radius", "border", "shadow", "src", "fit", "blur",
    "trim", "volume", "loop", "fadeIn", "fadeOut", "color", "gradient", "text", "font", "size", "weight", "align",
    "maxWidth", "lineHeight", "letterSpacing", "stroke", "background", "uppercase", "reveal", "revealDuration",
    "style", "highlight", "highlightBg", "maxWords", "bars", "mirror", "gap", "gain", "children", "sequence",
    "overlap", "comment",
];
const LAYER_TYPES: &[&str] = &["rect", "image", "video", "text", "captions", "waveform", "group"];

fn check_keys(v: &Value, allowed: &[&str], path: &str, warnings: &mut Vec<String>) {
    if let Value::Object(o) = v {
        for k in o.keys() {
            if !allowed.contains(&k.as_str()) {
                warnings.push(format!("{path}: unknown property '{k}' (ignored)"));
            }
        }
    }
}

fn check_layer(v: &Value, path: &str, warnings: &mut Vec<String>) {
    check_keys(v, LAYER_KEYS, path, warnings);
    if let Some(t) = v.get("type").and_then(Value::as_str) {
        if !LAYER_TYPES.contains(&t) {
            warnings.push(format!("{path}: unknown layer type '{t}' (expected one of {LAYER_TYPES:?})"));
        }
    } else {
        warnings.push(format!("{path}: layer without \"type\""));
    }
    if let Some(Value::Array(children)) = v.get("children") {
        for (i, c) in children.iter().enumerate() {
            check_layer(c, &format!("{path}.children[{i}]"), warnings);
        }
    }
}

impl Scene {
    pub fn load(path: &Path) -> Result<(Scene, Vec<String>)> {
        let txt = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let value: Value = serde_json::from_str(&txt).with_context(|| format!("parsing {}", path.display()))?;
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Scene::from_value(value, base)
    }

    pub fn from_value(value: Value, base_dir: PathBuf) -> Result<(Scene, Vec<String>)> {
        let mut warnings = Vec::new();
        check_keys(&value, SCENE_KEYS, "scene", &mut warnings);
        if let Some(Value::Array(a)) = value.get("audio") {
            for (i, t) in a.iter().enumerate() {
                check_keys(t, AUDIO_KEYS, &format!("audio[{i}]"), &mut warnings);
            }
        }
        if let Some(Value::Array(layers)) = value.get("layers") {
            for (i, l) in layers.iter().enumerate() {
                check_layer(l, &format!("layers[{i}]"), &mut warnings);
            }
        }
        let mut scene: Scene = serde_json::from_value(value).context("invalid scene")?;
        scene.base_dir = base_dir;
        if scene.width == 0 || scene.height == 0 || scene.fps <= 0.0 {
            anyhow::bail!("width, height and fps must be positive");
        }
        Ok((scene, warnings))
    }

    pub fn resolve(&self, p: &str) -> PathBuf {
        let path = PathBuf::from(p);
        if path.is_absolute() {
            path
        } else {
            self.base_dir.join(path)
        }
    }
}
