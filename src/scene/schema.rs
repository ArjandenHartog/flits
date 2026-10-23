//! JSON Schema for the scene format (`flits schema`, committed as `schema/scene.schema.json`).
//!
//! The object schemas are derived (schemars) from the scene structs in `scene/mod.rs`, so they
//! follow the Rust definitions (field names, defaults, doc comments). The values that have
//! hand-written parsers — animated numbers, transitions, colours, eases — get their schemas here,
//! as do the string enums of `Layer` (`type`, `fit`, `align`, ...).

use super::anim::{Ease, KeyTime, Prop, Transition};
use super::color::Rgba;
use super::{Scene, LAYER_TYPES};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde_json::{json, Value};
use std::borrow::Cow;

/// Where the committed schema can be fetched from (use it as `"$schema"` in scene files).
pub const SCHEMA_URL: &str = "https://raw.githubusercontent.com/ArjandenHartog/flits/main/schema/scene.schema.json";

/// Canonical ease names (`Ease::parse` also accepts the aliases in [`EASE_ALIASES`]).
pub const EASES: &[&str] = &["linear", "in", "out", "in-out", "smooth", "back-in", "back-out", "elastic", "bounce", "spring", "hold"];
pub const EASE_ALIASES: &[&str] = &[
    "ease-in",
    "easein",
    "ease-out",
    "easeout",
    "ease",
    "inout",
    "ease-in-out",
    "easeinout",
    "sine",
    "sine-in-out",
    "anticipate",
    "back",
    "overshoot",
    "step",
];

/// Canonical transition types (plus the short aliases in [`TRANSITION_ALIASES`]).
pub const TRANSITIONS: &[&str] = &[
    "fade",
    "slide-up",
    "slide-down",
    "slide-left",
    "slide-right",
    "zoom-in",
    "zoom-out",
    "pop",
    "wipe-left",
    "wipe-right",
    "wipe-up",
    "wipe-down",
];
pub const TRANSITION_ALIASES: &[&str] = &["up", "down", "left", "right", "zoom", "wipe"];

pub const FITS: &[&str] = &["cover", "contain", "fill", "stretch"];
pub const ALIGNS: &[&str] = &["left", "center", "right", "start", "middle", "end"];
pub const REVEALS: &[&str] = &["words", "lines", "chars", "typewriter", "none"];
pub const WRAPS: &[&str] = &["balance", "greedy"];
pub const CAPTION_STYLES: &[&str] = &["karaoke", "pop", "word", "line", "highlight", "tiktok", "single", "plain", "subtitle"];
pub const TEXT_BG_MODES: &[&str] = &["box", "lines"];
pub const GRADIENT_TYPES: &[&str] = &["linear", "radial"];

const NAMED_COLORS: &[&str] =
    &["white", "black", "transparent", "red", "green", "blue", "yellow", "orange", "purple", "pink", "gray", "grey"];

/// unsigned decimal number (as accepted by Rust's f64 parser, minus inf/nan)
const UNUM: &str = r"(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][-+]?[0-9]+)?";
const NUM: &str = r"[-+]?(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][-+]?[0-9]+)?";

/// The complete scene schema as JSON (draft 2020-12).
pub fn scene_schema() -> Value {
    let mut v = SchemaGenerator::default().into_root_schema_for::<Scene>().to_value();
    tidy(&mut v);
    let root = v.as_object_mut().expect("root schema is an object");
    root.insert("$id".into(), json!(SCHEMA_URL));
    root.insert("title".into(), json!("flits scene"));
    if let Some(Value::Object(props)) = root.get_mut("properties") {
        props.insert(
            "$schema".into(),
            json!({"type": "string", "description": "Optional URL or path of this JSON Schema (for editor completion); ignored by flits."}),
        );
    }
    v
}

/// The schema as pretty-printed JSON with a trailing newline (the format of the committed file).
pub fn scene_schema_pretty() -> String {
    let mut s = serde_json::to_string_pretty(&scene_schema()).expect("schema serializes");
    s.push('\n');
    s
}

/// Post-processing of the generated schema:
/// * optional fields are simply omitted, so drop the `null` alternatives serde would also accept;
/// * objects are closed (`additionalProperties: false`) like the typo warnings of `flits check`,
///   except that every object may carry a free-form `"comment"` (ignored by flits);
/// * drop `"default": null` and numeric `format`s (noise for editors and LLMs).
fn tidy(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for (_, child) in o.iter_mut() {
                tidy(child);
            }
            // "type": ["number", "null"] -> "type": "number"
            if let Some(Value::Array(types)) = o.get_mut("type") {
                types.retain(|t| t != "null");
                if types.len() == 1 {
                    let t = types.remove(0);
                    o.insert("type".into(), t);
                }
            }
            // "anyOf": [X, {"type": "null"}] -> X (merged into this schema)
            if let Some(Value::Array(any)) = o.get_mut("anyOf") {
                any.retain(|s| s.get("type") != Some(&json!("null")) || s.as_object().is_some_and(|m| m.len() > 1));
                if any.len() == 1 {
                    if let Value::Object(only) = any.remove(0) {
                        o.remove("anyOf");
                        for (k, val) in only {
                            o.entry(k).or_insert(val);
                        }
                    }
                }
            }
            if o.get("default") == Some(&Value::Null) {
                o.remove("default");
            }
            if let Some(Value::String(f)) = o.get("format") {
                if matches!(f.as_str(), "double" | "float" | "uint" | "uint8" | "uint16" | "uint32" | "uint64" | "int" | "int32" | "int64")
                {
                    o.remove("format");
                }
            }
            // doc comments wrap at the source width; descriptions are single paragraphs
            if let Some(Value::String(d)) = o.get_mut("description") {
                *d = d.replace('\n', " ");
            }
            // close object schemas (not the partial ones inside "if")
            if o.get("type") == Some(&json!("object")) {
                if let Some(Value::Object(props)) = o.get_mut("properties") {
                    props
                        .entry("comment")
                        .or_insert_with(|| json!({"type": "string", "description": "Free-form note for humans; ignored by flits."}));
                    o.entry("additionalProperties").or_insert(json!(false));
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(tidy),
        _ => {}
    }
}

fn string_enum(description: &str, values: &[&str]) -> Schema {
    json_schema!({ "type": "string", "enum": values, "description": description })
}

/// `a|b|c` for a regex (the names only contain letters and hyphens, which need no escaping
/// outside a character class — and escaping them is an error in ECMAScript `u` mode).
fn alternation(values: &[&str]) -> String {
    values.join("|")
}

// ---------------------------------------------------------------- string enums of Layer & co

pub(crate) fn layer_type(_: &mut SchemaGenerator) -> Schema {
    string_enum(
        "Layer type. rect: solid colour or gradient box; image: photo/SVG; video: clip (with sound); text: styled text; \
         counter: animated number; captions: word-by-word subtitles from a transcript; waveform: live audio spectrum; \
         group: container for `children`.",
        LAYER_TYPES,
    )
}

pub(crate) fn fit(_: &mut SchemaGenerator) -> Schema {
    string_enum("How an image/video fills its box: cover (default, crop), contain (letterbox), fill (stretch; alias stretch).", FITS)
}

pub(crate) fn align(_: &mut SchemaGenerator) -> Schema {
    string_enum("Text alignment: left, center (default), right (aliases start, middle, end).", ALIGNS)
}

pub(crate) fn reveal(_: &mut SchemaGenerator) -> Schema {
    string_enum("Reveal the text piece by piece over `revealDuration` seconds: words, lines, chars or typewriter.", REVEALS)
}

pub(crate) fn wrap(_: &mut SchemaGenerator) -> Schema {
    string_enum("Line wrapping with `maxWidth`: balance (even line lengths) or greedy (default).", WRAPS)
}

pub(crate) fn caption_style(_: &mut SchemaGenerator) -> Schema {
    string_enum(
        "Captions style: karaoke (default; active word highlighted), pop (words pop in as spoken, TikTok-style), \
         word (one big word at a time), line (plain subtitles). Aliases: highlight, tiktok, single, plain, subtitle.",
        CAPTION_STYLES,
    )
}

pub(crate) fn text_bg_mode(_: &mut SchemaGenerator) -> Schema {
    string_enum("box (default): one box around the whole text block; lines: one pill per line.", TEXT_BG_MODES)
}

pub(crate) fn gradient_type(_: &mut SchemaGenerator) -> Schema {
    string_enum("linear (default; uses `angle`) or radial (centre to edge).", GRADIENT_TYPES)
}

/// Per-type requirements of a layer (what `Prepared::build` errors on when missing).
pub(crate) fn layer_rules(schema: &mut Schema) {
    let rule = |types: &[&str], required: &[&str]| json!({"if": {"properties": {"type": {"enum": types}}, "required": ["type"]}, "then": {"required": required}});
    schema.insert(
        "allOf".into(),
        json!([rule(&["image", "video", "captions", "waveform"], &["src"]), rule(&["text"], &["text"]), rule(&["counter"], &["value"]),]),
    );
    // every layer needs a "type" (serde fills in "" thanks to the container default, so say it here)
    schema.insert("required".into(), json!(["type"]));
    if let Some(Value::Object(t)) = schema.get_mut("properties").and_then(|p| p.get_mut("type")) {
        t.remove("default");
    }
}

// ---------------------------------------------------------------- hand-parsed values

impl JsonSchema for Rgba {
    fn schema_name() -> Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let pattern = format!(
            r"^\s*(#([0-9a-fA-F]{{3,4}}|[0-9a-fA-F]{{6}}|[0-9a-fA-F]{{8}})|rgba?\(\s*{NUM}\s*(,\s*{NUM}\s*){{2,3}}\)|{})\s*$",
            NAMED_COLORS.join("|")
        );
        json_schema!({
            "type": "string",
            "description": "Colour: #rgb, #rgba, #rrggbb, #rrggbbaa, rgb(r,g,b), rgba(r,g,b,a) (r,g,b 0..255, a 0..1) or a name \
                            (white, black, transparent, red, green, blue, yellow, orange, purple, pink, gray).",
            "pattern": pattern,
            "examples": ["#ffffff", "#f26328", "#00000080", "rgba(0,0,0,0.5)", "white"]
        })
    }
}

impl JsonSchema for Ease {
    fn schema_name() -> Cow<'static, str> {
        "Ease".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let all: Vec<&str> = EASES.iter().chain(EASE_ALIASES).copied().collect();
        json_schema!({
            "type": "string",
            "enum": all,
            "description": "Easing curve: linear, in, out, in-out (default for keyframes), smooth (sine in-out), back-in, \
                            back-out (overshoot), elastic, bounce, spring, hold (jump at the end of the segment)."
        })
    }
}

impl JsonSchema for KeyTime {
    fn schema_name() -> Cow<'static, str> {
        "KeyTime".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Keyframe time: seconds from the layer start, or \"end\" / \"end-0.5\" (relative to the layer end).",
            "anyOf": [
                {"type": "number"},
                {"type": "string", "pattern": format!(r"^\s*(end(\s*[-+]\s*{UNUM})?|{NUM})\s*$")}
            ],
            "examples": [0, 1.5, "end", "end-0.5"]
        })
    }
}

/// `[time, value]` or `[time, value, ease]`.
struct Keyframe;

impl JsonSchema for Keyframe {
    fn schema_name() -> Cow<'static, str> {
        "Keyframe".into()
    }

    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "[time, value] or [time, value, ease]; the ease applies to the segment ending at this key.",
            "type": "array",
            "prefixItems": [g.subschema_for::<KeyTime>(), {"type": "number"}, g.subschema_for::<Ease>()],
            "items": false,
            "minItems": 2,
            "maxItems": 3
        })
    }
}

/// Animate from one value to another over the layer (or between `start` and `end`).
#[derive(JsonSchema)]
#[allow(dead_code)]
struct FromTo {
    /// value at `start`
    from: f64,
    /// value at `end`
    to: f64,
    /// easing curve (default linear)
    ease: Option<Ease>,
    /// when the animation starts (default 0 = layer start)
    start: Option<KeyTime>,
    /// when the animation ends (default "end" = layer end)
    end: Option<KeyTime>,
}

impl JsonSchema for Prop {
    fn schema_name() -> Cow<'static, str> {
        "Animated".into()
    }

    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "An animatable number: a constant, keyframes [[time, value, ease?], ...] or {\"from\", \"to\", \"ease\", \"start\", \"end\"}.",
            "anyOf": [
                {"type": "number"},
                {"type": "array", "items": g.subschema_for::<Keyframe>(), "minItems": 1},
                g.subschema_for::<FromTo>()
            ],
            "examples": [0.8, [[0, 1200], [0.6, 540, "back-out"], ["end-0.5", 540], ["end", -200, "in"]], {"from": 1.0, "to": 1.15}]
        })
    }
}

/// Transition type name.
struct TransitionType;

impl JsonSchema for TransitionType {
    fn schema_name() -> Cow<'static, str> {
        "TransitionType".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let all: Vec<&str> = TRANSITIONS.iter().chain(TRANSITION_ALIASES).copied().collect();
        json_schema!({
            "type": "string",
            "enum": all,
            "description": "fade, slide-up/down/left/right, zoom-in, zoom-out, pop, wipe-left/right/up/down \
                            (aliases: up, down, left, right, zoom = zoom-in, wipe = wipe-right)."
        })
    }
}

/// Enter/exit transition as an object.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct TransitionSpec {
    /// transition type (default fade)
    #[serde(rename = "type")]
    kind: Option<TransitionType>,
    /// seconds (default 0.5; pop 0.45)
    duration: Option<f64>,
    /// easing curve (default: out; pop: back-out; wipes: in-out)
    ease: Option<Ease>,
    /// slide distance in px (default 120)
    distance: Option<f64>,
}

impl JsonSchema for Transition {
    fn schema_name() -> Cow<'static, str> {
        "Transition".into()
    }

    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        let kinds = alternation(&TRANSITIONS.iter().chain(TRANSITION_ALIASES).copied().collect::<Vec<_>>());
        json_schema!({
            "description": "Enter (`in`) or exit (`out`) transition: \"fade\", \"pop 0.4\" (type + seconds) or {\"type\", \"duration\", \"ease\", \"distance\"}.",
            "anyOf": [
                {"type": "string", "pattern": format!(r"^\s*({kinds})(\s+([0-9]+\.?[0-9]*|\.[0-9]+)s?)?\s*$")},
                g.subschema_for::<TransitionSpec>()
            ],
            "examples": ["fade", "pop 0.4", {"type": "slide-up", "duration": 0.6, "ease": "out", "distance": 200}]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_values_parse() {
        for e in EASES.iter().chain(EASE_ALIASES) {
            assert!(Ease::parse(e).is_ok(), "{e}");
        }
        for t in TRANSITIONS.iter().chain(TRANSITION_ALIASES) {
            assert!(Transition::from_json(&json!(t)).is_ok(), "{t}");
        }
        for a in ALIGNS {
            assert!(crate::text::Align::parse(Some(a)).is_ok(), "{a}");
        }
        for c in NAMED_COLORS {
            assert!(Rgba::parse(c).is_ok(), "{c}");
        }
    }

    #[test]
    fn schema_is_closed_and_has_no_nulls() {
        let s = scene_schema_pretty();
        assert!(!s.contains("\"null\""), "null alternatives should be stripped");
        let v = scene_schema();
        assert_eq!(v["additionalProperties"], json!(false));
        assert_eq!(v["$defs"]["Layer"]["additionalProperties"], json!(false));
        assert!(v["$defs"]["Layer"]["required"].as_array().unwrap().contains(&json!("type")));
    }
}
