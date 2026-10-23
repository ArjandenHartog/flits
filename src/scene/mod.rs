//! Scene description (the JSON format) and validation.

pub mod anim;
pub mod color;
pub mod presets;

use anim::{Prop, Transition};
use anyhow::{Context, Result};
use color::Rgba;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn default_fps() -> f64 {
    30.0
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
    /// normalize the final mix to this loudness (LUFS, e.g. -14 for social, -16 for podcasts)
    #[serde(default)]
    pub loudness: Option<f64>,
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
    /// constant or keyframed (seconds from the track start)
    #[serde(default)]
    pub volume: Option<Prop>,
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
    #[serde(default, rename = "loop")]
    pub looped: bool,
    /// duck under the other audio (voice, video sound): true or the level to duck to (0..1)
    #[serde(default)]
    pub duck: Option<Duck>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Duck {
    On(bool),
    Level(f64),
}

impl Duck {
    pub fn level(&self) -> Option<f64> {
        match self {
            Duck::On(true) => Some(0.25),
            Duck::On(false) => None,
            Duck::Level(l) => Some(*l),
        }
    }
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

fn neutral() -> f32 {
    1.0
}

/// Colour grading for image and video layers.
#[derive(Deserialize, Clone, Copy, Debug)]
pub struct Filter {
    #[serde(default)]
    pub brightness: f32,
    #[serde(default = "neutral")]
    pub contrast: f32,
    #[serde(default = "neutral")]
    pub saturation: f32,
    #[serde(default)]
    pub warmth: f32,
    #[serde(default)]
    pub grayscale: bool,
}

impl Filter {
    pub fn adjust(&self) -> crate::media::colorspace::Adjust {
        [self.brightness, self.contrast, if self.grayscale { 0.0 } else { self.saturation }, self.warmth]
    }
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
    /// colour grading (image, video)
    pub filter: Option<Filter>,
    /// clip children to the group's box (groups)
    pub clip: bool,

    // image / video / audio-carrying layers
    pub src: Option<String>,
    pub fit: Option<String>,
    pub blur: f64,
    pub trim: f64,
    pub volume: Option<Prop>,
    #[serde(rename = "loop")]
    pub looped: bool,
    /// playback speed for video (0.5 = slow motion, 2 = double speed)
    pub speed: Option<f64>,
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
    /// "balance": even line lengths when wrapping (default: greedy)
    pub wrap: Option<String>,
    pub reveal_duration: Option<f64>,

    // captions
    /// named caption look (see `presets`); merged into the layer before parsing
    pub preset: Option<String>,
    pub style: Option<String>,
    pub highlight: Option<Rgba>,
    pub highlight_bg: Option<Rgba>,
    /// "pill" (default) or "marker"
    pub highlight_box: Option<String>,
    pub max_words: Option<usize>,
    pub max_lines: Option<usize>,
    /// active-word colours, cycled word by word
    pub colors: Option<Vec<Rgba>>,
    pub active_scale: Option<f64>,
    /// word colour per diarization speaker label
    pub speaker_colors: Option<HashMap<String, Rgba>>,

    // waveform
    pub bars: Option<usize>,
    pub mirror: bool,
    pub gap: Option<f64>,
    pub gain: Option<f64>,

    // counter
    pub value: Option<Prop>,
    pub decimals: Option<usize>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    /// thousands separator, e.g. "." (default) or ","
    pub separator: Option<String>,

    // group
    pub children: Vec<Layer>,
    pub sequence: bool,
    pub overlap: f64,
}

const LAYER_TYPES: &[&str] = &["rect", "image", "video", "text", "counter", "captions", "waveform", "group"];

/// Layer types are plain strings in the struct, so check them (recursively) here.
fn check_types(v: &Value, path: &str, warnings: &mut Vec<String>) {
    match v.get("type").and_then(Value::as_str) {
        Some(t) if !LAYER_TYPES.contains(&t) => {
            warnings.push(format!("{path}: unknown layer type '{t}' (expected one of {LAYER_TYPES:?})"))
        }
        None => warnings.push(format!("{path}: layer without \"type\"")),
        _ => {}
    }
    if let Some(Value::Array(children)) = v.get("children") {
        for (i, c) in children.iter().enumerate() {
            check_types(c, &format!("{path}.children.{i}"), warnings);
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

    pub fn from_value(mut value: Value, base_dir: PathBuf) -> Result<(Scene, Vec<String>)> {
        let mut warnings = Vec::new();
        let canvas = (value["width"].as_f64().unwrap_or(1920.0), value["height"].as_f64().unwrap_or(1080.0));
        if let Some(Value::Array(layers)) = value.get_mut("layers") {
            presets::apply(layers, canvas, "layers", &mut warnings)?;
        }
        if let Some(Value::Array(layers)) = value.get("layers") {
            for (i, l) in layers.iter().enumerate() {
                check_types(l, &format!("layers.{i}"), &mut warnings);
            }
        }
        // unknown keys (typos) are reported straight from the struct definitions
        let mut scene: Scene = serde_ignored::deserialize(value, |path| {
            let p = path.to_string();
            if !p.ends_with("comment") && !p.ends_with("$schema") {
                warnings.push(format!("{p}: unknown property (ignored)"));
            }
        })
        .context("invalid scene")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn warns_about_typos() {
        let v = json!({"width": 100, "height": 100, "durration": 3, "layers": [
            {"type": "rect", "colour": "#fff"},
            {"type": "group", "children": [{"type": "txt"}]}
        ]});
        let (_, w) = Scene::from_value(v, PathBuf::new()).unwrap();
        assert!(w.iter().any(|w| w.contains("durration")));
        assert!(w.iter().any(|w| w.contains("colour")));
        assert!(w.iter().any(|w| w.contains("'txt'")));
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Scene::from_value(json!({"width": 0, "height": 10}), PathBuf::new()).is_err());
        assert!(
            Scene::from_value(json!({"width": 10, "height": 10, "layers": [{"type": "rect", "color": "nope"}]}), PathBuf::new()).is_err()
        );
    }
}
