//! Scene description (the JSON format) and validation.
//!
//! The doc comments on these structs end up as descriptions in the JSON Schema (`flits schema`).

pub mod anim;
pub mod color;
pub mod presets;
pub mod schema;
pub mod script;

use anim::{Prop, Transition};
use anyhow::{Context, Result};
use color::Rgba;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn default_fps() -> f64 {
    30.0
}

/// A flits scene: one video described as layers on a canvas. Units are pixels (origin top-left)
/// and seconds; relative file paths resolve against the scene file's folder.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    /// canvas width in pixels
    #[schemars(range(min = 1))]
    pub width: u32,
    /// canvas height in pixels
    #[schemars(range(min = 1))]
    pub height: u32,
    /// frames per second
    #[serde(default = "default_fps")]
    #[schemars(range(min = 1))]
    pub fps: f64,
    /// length in seconds (default: the end of the last layer)
    #[serde(default)]
    pub duration: Option<f64>,
    /// canvas colour behind all layers (default black)
    #[serde(default)]
    pub background: Option<Rgba>,
    /// font aliases usable in a layer's `font`, e.g. {"brand": "fonts/Rubik-Black.ttf"}
    #[serde(default)]
    pub fonts: HashMap<String, String>,
    /// fonts for characters a layer's font lacks (emoji, other scripts), tried in order before the
    /// system's fallback fonts: names, .ttf/.otf/.ttc paths or `fonts` aliases, e.g. ["Noto Color Emoji"]
    #[serde(default)]
    pub font_fallback: Vec<String>,
    /// soundtrack: music, voice-over, effects (audio of video layers is mixed in automatically)
    #[serde(default)]
    pub audio: Vec<AudioTrack>,
    /// normalize the final mix to this loudness (LUFS, e.g. -14 for social, -16 for podcasts)
    #[serde(default)]
    pub loudness: Option<f64>,
    /// motion blur for moving layers: `true` (up to 8 samples, 180° shutter) or `{ "samples": 12, "shutter": 0.5 }`
    #[serde(default)]
    pub motion_blur: Option<MotionBlur>,
    /// layers, drawn in order: the first is at the bottom
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// Directory relative paths are resolved against (not part of the JSON).
    #[serde(skip)]
    pub base_dir: PathBuf,
}

/// An audio file in the soundtrack.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    /// audio (or video) file
    pub src: String,
    /// scene time in seconds where the track starts
    #[serde(default)]
    pub start: f64,
    /// seconds to skip at the start of the source
    #[serde(default)]
    pub trim: f64,
    /// seconds to play (default: until the source ends)
    #[serde(default)]
    pub duration: Option<f64>,
    /// constant or keyframed (seconds from the track start)
    #[serde(default)]
    pub volume: Option<Prop>,
    /// fade-in in seconds
    #[serde(default)]
    pub fade_in: f64,
    /// fade-out in seconds
    #[serde(default)]
    pub fade_out: f64,
    /// repeat the source to fill `duration` (or the scene)
    #[serde(default, rename = "loop")]
    pub looped: bool,
    /// duck under the other audio (voice, video sound): true or the level to duck to (0..1)
    #[serde(default)]
    pub duck: Option<Duck>,
}

/// Ducking: `true` (duck to 0.25) or the volume level (0..1) to duck to while other audio plays.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(untagged)]
pub enum Duck {
    On(bool),
    Level(#[schemars(range(min = 0.0, max = 1.0))] f64),
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

/// Motion blur: `true`/`false`, or the number of samples and the shutter.
#[derive(Deserialize, JsonSchema, Clone, Copy, Debug)]
#[serde(untagged)]
pub enum MotionBlur {
    On(bool),
    Custom {
        /// most sub-frame samples per moving layer (default 8; layers that move little get fewer)
        #[serde(default)]
        #[schemars(range(min = 2, max = 64))]
        samples: Option<u32>,
        /// fraction of the frame interval the shutter is open (default 0.5 = 180°; 1 = smear over the whole frame)
        #[serde(default)]
        #[schemars(range(min = 0.0, max = 2.0))]
        shutter: Option<f64>,
    },
}

impl MotionBlur {
    /// (max samples, shutter) when enabled
    pub fn settings(&self) -> Option<(usize, f64)> {
        match *self {
            MotionBlur::On(false) => None,
            MotionBlur::On(true) => Some((8, 0.5)),
            MotionBlur::Custom { samples, shutter } => {
                let shutter = shutter.unwrap_or(0.5).clamp(0.0, 2.0);
                (shutter > 0.0).then(|| (samples.unwrap_or(8).clamp(2, 64) as usize, shutter))
            }
        }
    }
}

/// Border drawn outside the box.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
pub struct Border {
    /// px
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

/// Drop shadow (text: text shadow; other layers: box shadow).
#[derive(Deserialize, JsonSchema, Clone, Debug)]
pub struct Shadow {
    /// default rgba(0,0,0,0.5)
    #[serde(default = "shadow_color")]
    pub color: Rgba,
    /// blur radius in px
    #[serde(default = "shadow_blur")]
    pub blur: f64,
    /// horizontal offset in px
    #[serde(default)]
    pub x: f64,
    /// vertical offset in px
    #[serde(default = "shadow_y")]
    pub y: f64,
}

/// Text outline.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
pub struct StrokeSpec {
    /// px
    pub width: f64,
    pub color: Rgba,
}

/// Padding in px: one number or [horizontal, vertical].
#[derive(Deserialize, JsonSchema, Clone, Debug)]
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

/// Box (or one pill per line) behind text.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
pub struct TextBg {
    pub color: Rgba,
    /// px: one number or [horizontal, vertical]
    #[serde(default)]
    pub padding: Option<Padding>,
    /// corner radius in px (default: a quarter of the font size)
    #[serde(default)]
    pub radius: Option<f64>,
    /// "box" (one box around the block) or "lines" (one pill per line)
    #[serde(default)]
    #[schemars(schema_with = "schema::text_bg_mode")]
    pub mode: Option<String>,
}

fn neutral() -> f32 {
    1.0
}

/// Colour grading for image and video layers.
#[derive(Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct Filter {
    /// added brightness, e.g. 0.05 (0 = unchanged)
    #[serde(default)]
    pub brightness: f32,
    /// contrast factor (1 = unchanged)
    #[serde(default = "neutral")]
    pub contrast: f32,
    /// saturation factor (1 = unchanged, 0 = gray)
    #[serde(default = "neutral")]
    pub saturation: f32,
    /// warm (positive) or cool (negative) tint, e.g. 0.3
    #[serde(default)]
    pub warmth: f32,
    /// black and white
    #[serde(default)]
    pub grayscale: bool,
}

impl Filter {
    pub fn adjust(&self) -> crate::media::colorspace::Adjust {
        [self.brightness, self.contrast, if self.grayscale { 0.0 } else { self.saturation }, self.warmth]
    }
}

/// Gradient fill for rect layers: `stops` ([offset 0..1, colour] pairs) or evenly spaced `colors`.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
pub struct Gradient {
    #[serde(rename = "type", default)]
    #[schemars(schema_with = "schema::gradient_type")]
    pub kind: Option<String>,
    /// CSS-style angle in degrees: 180 = top to bottom, 90 = left to right
    #[serde(default)]
    pub angle: Option<f64>,
    /// [[offset 0..1, colour], ...]
    #[serde(default)]
    pub stops: Vec<(f64, Rgba)>,
    /// colours spread evenly (alternative to `stops`)
    #[serde(default)]
    pub colors: Vec<Rgba>,
}

/// One layer. All layer types share this flat set of properties; each type uses the ones that
/// apply to it (see the descriptions). Positions and sizes are relative to the parent (canvas or group).
#[derive(Deserialize, JsonSchema, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
#[schemars(transform = schema::layer_rules)]
pub struct Layer {
    #[serde(rename = "type")]
    #[schemars(schema_with = "schema::layer_type")]
    pub kind: String,
    /// optional name (for your own bookkeeping)
    pub id: Option<String>,
    /// start time in seconds (relative to the parent group)
    pub start: f64,
    /// end time in seconds (alternative to `duration`)
    pub end: Option<f64>,
    /// lifetime in seconds (videos default to their (re-timed) length)
    pub duration: Option<f64>,

    /// x of the anchor point in px (default: centre of the parent)
    pub x: Option<Prop>,
    /// y of the anchor point in px (default: centre of the parent)
    pub y: Option<Prop>,
    /// box width in px (default: parent width; text sizes itself; image/video keep their aspect ratio when only one side is given)
    pub width: Option<f64>,
    /// box height in px (see `width`)
    pub height: Option<f64>,
    /// which point of the box sits at (x, y): [0.5, 0.5] = centre (default), [0, 0] = top-left, [1, 1] = bottom-right
    pub anchor: Option<[f64; 2]>,
    /// uniform scale (1 = 100%)
    pub scale: Option<Prop>,
    /// horizontal scale
    pub scale_x: Option<Prop>,
    /// vertical scale
    pub scale_y: Option<Prop>,
    /// rotation in degrees (clockwise)
    pub rotation: Option<Prop>,
    /// 0..1
    pub opacity: Option<Prop>,
    /// image/video: zoom inside the box (Ken Burns), 1 = fit
    pub zoom: Option<Prop>,
    /// image/video: horizontal pan inside the box, -1..1
    pub pan_x: Option<Prop>,
    /// image/video: vertical pan inside the box, -1..1
    pub pan_y: Option<Prop>,
    /// enter transition
    #[serde(rename = "in")]
    pub enter: Option<Transition>,
    /// exit transition
    #[serde(rename = "out")]
    pub exit: Option<Transition>,
    /// corner radius in px (>= half the size gives a circle/pill)
    pub radius: f64,
    pub border: Option<Border>,
    pub shadow: Option<Shadow>,
    /// colour grading (image, video)
    pub filter: Option<Filter>,
    /// clip children to the group's box (groups)
    pub clip: bool,
    /// `false`: no motion blur for this layer and its children (when the scene has `motionBlur`)
    pub motion_blur: Option<bool>,

    // image / video / audio-carrying layers
    /// image/video: media file (jpg/png/webp/gif/bmp/svg, HEIC/AVIF via ffmpeg, any video ffmpeg reads);
    /// captions: transcript (.words.json/.srt/.vtt); waveform: audio or video file
    pub src: Option<String>,
    #[schemars(schema_with = "schema::fit")]
    pub fit: Option<String>,
    /// image: blur radius in px (done once, cheap)
    pub blur: f64,
    /// video/captions/waveform: seconds to skip at the start of the source
    pub trim: f64,
    /// video: audio volume (default 1, 0 = mute)
    pub volume: Option<Prop>,
    /// video: loop the clip
    #[serde(rename = "loop")]
    pub looped: bool,
    /// playback speed for video (0.5 = slow motion, 2 = double speed)
    pub speed: Option<f64>,
    /// video: audio fade-in in seconds
    pub fade_in: f64,
    /// video: audio fade-out in seconds
    pub fade_out: f64,

    // rect
    /// rect/shape: fill colour; text/counter/captions: text colour; line/arrow: line colour (default white)
    pub color: Option<Rgba>,
    /// rect/shape: gradient fill (instead of `color`)
    pub gradient: Option<Gradient>,

    // text
    /// text: the text (\n for line breaks; &shy; or U+00AD for soft hyphens)
    pub text: Option<String>,
    /// installed font name ("Segoe UI Black"), a .ttf/.otf path or a `fonts` alias
    pub font: Option<String>,
    /// font size in px (text 64, counter 96)
    pub size: Option<f64>,
    /// weight for variable fonts / font families (e.g. 900)
    pub weight: Option<f64>,
    /// variable font axes: 4-letter tag -> value, e.g. {"wdth": 75, "opsz": 36, "slnt": -8} (`weight` wins over "wght")
    pub variations: Option<HashMap<String, f64>>,
    #[schemars(schema_with = "schema::align")]
    pub align: Option<String>,
    /// wrap text at this width in px
    pub max_width: Option<f64>,
    /// line height as a multiple of the font size (default 1.15)
    pub line_height: Option<f64>,
    /// extra space between letters in px
    pub letter_spacing: f64,
    pub stroke: Option<StrokeSpec>,
    pub background: Option<TextBg>,
    pub uppercase: bool,
    #[schemars(schema_with = "schema::reveal")]
    pub reveal: Option<String>,
    /// "balance": even line lengths when wrapping (default: greedy)
    #[schemars(schema_with = "schema::wrap")]
    pub wrap: Option<String>,
    /// seconds over which the `reveal` pieces appear
    pub reveal_duration: Option<f64>,
    /// how each `reveal` piece comes in: rise (default), mask (slides up out of its own line, clipped),
    /// fade, or blur (sharpens while it rises)
    #[schemars(schema_with = "schema::reveal_style")]
    pub reveal_style: Option<String>,

    // captions
    /// captions: named look (see `presets`); merged into the layer before parsing
    #[schemars(schema_with = "schema::caption_preset")]
    pub preset: Option<String>,
    #[schemars(schema_with = "schema::caption_style")]
    pub style: Option<String>,
    /// captions: colour of the active word (default yellow)
    pub highlight: Option<Rgba>,
    /// captions: box colour behind the active word (shape: `highlightBox`)
    pub highlight_bg: Option<Rgba>,
    #[schemars(schema_with = "schema::highlight_box")]
    pub highlight_box: Option<String>,
    /// captions: most words per chunk
    pub max_words: Option<usize>,
    /// captions: most lines per chunk (default 2; a chunk ends before it would wrap further)
    pub max_lines: Option<usize>,
    /// captions: most characters per chunk, spaces included (default 9 per word of `maxWords`, at least 12)
    pub max_chars: Option<usize>,
    #[schemars(schema_with = "schema::caption_breaks")]
    pub breaks: Option<String>,
    /// captions: words (case-insensitive) a chunk never ends on, e.g. articles and prepositions
    /// ["de", "het", "een", "van", ...]; implies `breaks: "phrase"`
    pub no_break_after: Option<Vec<String>>,
    /// captions: language for the built-in `noBreakAfter` list: nl, en, or auto (the transcript's own
    /// language); implies `breaks: "phrase"`
    pub language: Option<String>,
    /// captions: a chunk spoken faster than this (seconds) merges with a neighbour (not across a sentence
    /// end, a pause over 0.2 s or a new speaker, and only within `maxLines`); also the shortest time a
    /// chunk stays on screen, as far as the next one allows (default: off)
    pub min_duration: Option<f64>,
    /// captions: seconds a chunk appears before its first word is spoken (default 0)
    pub lead: Option<f64>,
    /// captions: seconds a chunk stays after its last word (default 0.25)
    pub hold: Option<f64>,
    /// captions: a gap to the next chunk shorter than this (seconds) is closed, so the chunk stays up until
    /// the next one (default 0.6). Chunks never overlap.
    pub gap_close: Option<f64>,
    /// captions: seconds over which a chunk fades and scales in (default 0.12; 0 = it appears at once)
    pub appear: Option<f64>,
    /// captions: active-word colours, cycled word by word (instead of `highlight`)
    pub colors: Option<Vec<Rgba>>,
    /// captions: size of the active word relative to the others (default 1.12 for pop, 1 otherwise)
    pub active_scale: Option<f64>,
    /// captions: word colour per diarization speaker label ({"SPEAKER_00": "#ffffff", ...})
    pub speaker_colors: Option<HashMap<String, Rgba>>,

    // shape
    #[schemars(schema_with = "schema::shape_kind")]
    pub shape: Option<String>,
    /// shape: points [[x, y], ...] in the layer box (line, arrow: a polyline; polygon: its corners)
    pub points: Option<Vec<[f64; 2]>>,
    /// shape "path": SVG path data in layer-box units, e.g. "M 0 100 C 50 0 150 0 200 100"
    pub d: Option<String>,
    /// shape: corners of a regular polygon (default 6) or points of a star (default 5)
    pub sides: Option<usize>,
    /// shape "star": inner radius as a fraction of the outer one (default 0.5)
    pub inner_radius: Option<f64>,
    #[schemars(schema_with = "schema::line_cap")]
    pub line_cap: Option<String>,
    /// shape: dash pattern of the stroke in px, e.g. [20, 10]
    pub dash: Option<Vec<f64>>,
    /// shape: how much of the outline is drawn, 0..1 (animate it to draw the shape on; default 1)
    pub progress: Option<Prop>,
    /// shape "arrow": length of the arrow head in px (default 4 x the stroke width)
    pub head_size: Option<f64>,

    // waveform
    /// waveform: number of bars (default 48)
    pub bars: Option<usize>,
    /// waveform: bars grow from the centre line
    pub mirror: bool,
    /// waveform: gap between bars as a fraction of a slot (0..1)
    pub gap: Option<f64>,
    /// waveform: amplification
    pub gain: Option<f64>,

    // counter
    /// counter: the number (animate it, e.g. {"from": 0, "to": 12840, "ease": "out", "end": 2.5})
    pub value: Option<Prop>,
    /// counter: digits after the decimal mark
    pub decimals: Option<usize>,
    /// counter: text before the number
    pub prefix: Option<String>,
    /// counter: text after the number
    pub suffix: Option<String>,
    /// counter: thousands separator: "," (default, 1,234.5), "." (1.234,5) or "none"
    pub separator: Option<String>,

    // group
    /// group: child layers (positioned and timed relative to the group)
    pub children: Vec<Layer>,
    /// group: play children one after another (each needs a duration; clips use their length)
    pub sequence: bool,
    /// group with `sequence`: seconds consecutive children overlap (use with "in": "fade" for crossfades)
    pub overlap: f64,
}

const LAYER_TYPES: &[&str] = &["rect", "image", "video", "text", "counter", "captions", "waveform", "shape", "group"];

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
    /// Load a scene file: JSON, or a TypeScript/JavaScript module exporting a scene (see `script`).
    pub fn load(path: &Path) -> Result<(Scene, Vec<String>)> {
        let value: Value = if script::is_script(path) {
            script::load(path)?
        } else {
            let txt = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            serde_json::from_str(&txt).with_context(|| format!("parsing {}", path.display()))?
        };
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
