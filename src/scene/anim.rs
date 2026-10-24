//! Animated properties, easing curves and enter/exit transitions.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    Linear,
    In,
    Out,
    InOut,
    SineInOut,
    BackIn,
    BackOut,
    Elastic,
    Bounce,
    Spring,
    Hold,
}

impl Ease {
    pub fn parse(s: &str) -> Result<Ease> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "linear" => Ease::Linear,
            "in" | "ease-in" | "easein" => Ease::In,
            "out" | "ease-out" | "easeout" | "ease" => Ease::Out,
            "in-out" | "inout" | "ease-in-out" | "easeinout" => Ease::InOut,
            "sine" | "sine-in-out" | "smooth" => Ease::SineInOut,
            "back-in" | "anticipate" => Ease::BackIn,
            "back-out" | "back" | "overshoot" => Ease::BackOut,
            "elastic" => Ease::Elastic,
            "bounce" => Ease::Bounce,
            "spring" => Ease::Spring,
            "hold" | "step" => Ease::Hold,
            other => bail!("unknown ease '{other}' (linear, in, out, in-out, smooth, back-in, back-out, elastic, bounce, spring, hold)"),
        })
    }

    pub fn apply(self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Ease::Linear => x,
            Ease::In => x * x * x,
            Ease::Out => 1.0 - (1.0 - x).powi(3),
            Ease::InOut => {
                if x < 0.5 {
                    4.0 * x * x * x
                } else {
                    1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
                }
            }
            Ease::SineInOut => -((std::f64::consts::PI * x).cos() - 1.0) / 2.0,
            Ease::BackIn => {
                let c1 = 1.70158;
                (c1 + 1.0) * x * x * x - c1 * x * x
            }
            Ease::BackOut => {
                let c1 = 1.70158;
                let c3 = c1 + 1.0;
                1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
            }
            Ease::Elastic => {
                if x == 0.0 || x == 1.0 {
                    x
                } else {
                    let c4 = 2.0 * std::f64::consts::PI / 3.0;
                    2f64.powf(-10.0 * x) * ((x * 10.0 - 0.75) * c4).sin() + 1.0
                }
            }
            Ease::Bounce => {
                let (n1, d1) = (7.5625, 2.75);
                if x < 1.0 / d1 {
                    n1 * x * x
                } else if x < 2.0 / d1 {
                    let x = x - 1.5 / d1;
                    n1 * x * x + 0.75
                } else if x < 2.5 / d1 {
                    let x = x - 2.25 / d1;
                    n1 * x * x + 0.9375
                } else {
                    let x = x - 2.625 / d1;
                    n1 * x * x + 0.984375
                }
            }
            Ease::Spring => {
                if x >= 1.0 {
                    1.0
                } else {
                    1.0 - (-6.5 * x).exp() * (11.0 * x).cos()
                }
            }
            Ease::Hold => {
                if x >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// Time of a keyframe: seconds from layer start, or relative to the layer end.
#[derive(Clone, Copy, Debug)]
pub enum KeyTime {
    Abs(f64),
    FromEnd(f64),
}

impl KeyTime {
    fn parse(v: &Value) -> Result<KeyTime> {
        match v {
            Value::Number(n) => Ok(KeyTime::Abs(n.as_f64().unwrap_or(0.0))),
            Value::String(s) => {
                let s = s.trim();
                if s == "end" {
                    return Ok(KeyTime::FromEnd(0.0));
                }
                if let Some(rest) = s.strip_prefix("end") {
                    let rest = rest.trim().replace(' ', "");
                    let off: f64 = rest.parse().map_err(|_| anyhow!("bad key time '{s}' (use e.g. \"end\" or \"end-0.5\")"))?;
                    return Ok(KeyTime::FromEnd(off));
                }
                Ok(KeyTime::Abs(s.parse().map_err(|_| anyhow!("bad key time '{s}'"))?))
            }
            _ => bail!("key time must be a number or \"end-<sec>\""),
        }
    }
    pub fn at(self, dur: f64) -> f64 {
        match self {
            KeyTime::Abs(t) => t,
            KeyTime::FromEnd(off) => dur + off,
        }
    }
}

/// A number that can be animated over the lifetime of a layer.
#[derive(Clone, Debug)]
pub enum Prop {
    Const(f64),
    Keys(Vec<(KeyTime, f64, Ease)>),
}

impl Prop {
    pub fn eval(&self, t: f64, dur: f64) -> f64 {
        match self {
            Prop::Const(v) => *v,
            Prop::Keys(keys) => {
                let first = &keys[0];
                if t <= first.0.at(dur) {
                    return first.1;
                }
                for w in keys.windows(2) {
                    let (t0, t1) = (w[0].0.at(dur), w[1].0.at(dur));
                    if t < t1 {
                        let span = (t1 - t0).max(1e-9);
                        let p = w[1].2.apply((t - t0) / span);
                        return w[0].1 + (w[1].1 - w[0].1) * p;
                    }
                }
                keys[keys.len() - 1].1
            }
        }
    }

    /// Largest absolute value the property can reach (used to size assets).
    pub fn max_abs(&self) -> f64 {
        match self {
            Prop::Const(v) => v.abs(),
            Prop::Keys(k) => k.iter().map(|k| k.1.abs()).fold(0.0, f64::max) * 1.12, // headroom for overshoot
        }
    }

    pub fn is_const(&self) -> bool {
        matches!(self, Prop::Const(_))
    }

    pub fn from_json(v: &Value) -> Result<Prop> {
        match v {
            Value::Number(n) => Ok(Prop::Const(n.as_f64().unwrap_or(0.0))),
            Value::Array(items) => {
                let mut keys = Vec::new();
                for it in items {
                    let arr = it.as_array().ok_or_else(|| anyhow!("keyframe must be [time, value] or [time, value, \"ease\"]"))?;
                    if arr.len() < 2 {
                        bail!("keyframe must be [time, value] or [time, value, \"ease\"]");
                    }
                    let t = KeyTime::parse(&arr[0])?;
                    let val = arr[1].as_f64().ok_or_else(|| anyhow!("keyframe value must be a number"))?;
                    let ease = match arr.get(2) {
                        Some(Value::String(s)) => Ease::parse(s)?,
                        _ => Ease::InOut,
                    };
                    keys.push((t, val, ease));
                }
                if keys.is_empty() {
                    bail!("empty keyframe list");
                }
                Ok(Prop::Keys(keys))
            }
            Value::Object(o) => {
                let from = o.get("from").and_then(Value::as_f64).ok_or_else(|| anyhow!("{{from,to}} needs a numeric 'from'"))?;
                let to = o.get("to").and_then(Value::as_f64).ok_or_else(|| anyhow!("{{from,to}} needs a numeric 'to'"))?;
                let ease = match o.get("ease") {
                    Some(Value::String(s)) => Ease::parse(s)?,
                    _ => Ease::Linear,
                };
                let start = match o.get("start") {
                    Some(v) => KeyTime::parse(v)?,
                    None => KeyTime::Abs(0.0),
                };
                let end = match o.get("end") {
                    Some(v) => KeyTime::parse(v)?,
                    None => KeyTime::FromEnd(0.0),
                };
                Ok(Prop::Keys(vec![(start, from, Ease::Linear), (end, to, ease)]))
            }
            _ => bail!("animated value must be a number, [[t,v,ease],...] or {{\"from\":a,\"to\":b}}"),
        }
    }
}

impl<'de> Deserialize<'de> for Prop {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        Prop::from_json(&v).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TKind {
    Fade,
    SlideUp,
    SlideDown,
    SlideLeft,
    SlideRight,
    ZoomIn,
    ZoomOut,
    Pop,
    WipeLeft,
    WipeRight,
    WipeUp,
    WipeDown,
    PushLeft,
    PushRight,
    PushUp,
    PushDown,
    ZoomFade,
    SlideFade,
    RotateIn,
    FlipX,
    FlipY,
    Iris,
    ZoomBlur,
}

/// Canonical transition names (the schema and the docs list these, in this order).
pub const TRANSITION_NAMES: &[(&str, TKind)] = &[
    ("fade", TKind::Fade),
    ("slide-up", TKind::SlideUp),
    ("slide-down", TKind::SlideDown),
    ("slide-left", TKind::SlideLeft),
    ("slide-right", TKind::SlideRight),
    ("slide-fade", TKind::SlideFade),
    ("zoom-in", TKind::ZoomIn),
    ("zoom-out", TKind::ZoomOut),
    ("zoom-fade", TKind::ZoomFade),
    ("zoom-blur", TKind::ZoomBlur),
    ("pop", TKind::Pop),
    ("wipe-left", TKind::WipeLeft),
    ("wipe-right", TKind::WipeRight),
    ("wipe-up", TKind::WipeUp),
    ("wipe-down", TKind::WipeDown),
    ("push-left", TKind::PushLeft),
    ("push-right", TKind::PushRight),
    ("push-up", TKind::PushUp),
    ("push-down", TKind::PushDown),
    ("rotate-in", TKind::RotateIn),
    ("flip-x", TKind::FlipX),
    ("flip-y", TKind::FlipY),
    ("iris", TKind::Iris),
];

/// Short aliases accepted by the parser.
pub const TRANSITION_ALIASES: &[(&str, TKind)] = &[
    ("up", TKind::SlideUp),
    ("down", TKind::SlideDown),
    ("left", TKind::SlideLeft),
    ("right", TKind::SlideRight),
    ("rise", TKind::SlideFade),
    ("zoom", TKind::ZoomIn),
    ("wipe", TKind::WipeRight),
    ("push", TKind::PushRight),
    ("rotate", TKind::RotateIn),
    ("flip", TKind::FlipX),
    ("circle", TKind::Iris),
];

#[derive(Clone, Debug)]
pub struct Transition {
    pub kind: TKind,
    pub dur: f64,
    pub ease: Ease,
    pub dist: f64,
    /// rotate-in: degrees turned during the transition
    pub angle: f64,
}

impl Transition {
    fn parse_kind(s: &str) -> Result<TKind> {
        let s = s.trim().to_ascii_lowercase();
        TRANSITION_NAMES.iter().chain(TRANSITION_ALIASES).find(|(name, _)| *name == s).map(|&(_, k)| k).ok_or_else(|| {
            let names: Vec<&str> = TRANSITION_NAMES.iter().map(|(n, _)| *n).collect();
            anyhow!("unknown transition '{s}' ({})", names.join(", "))
        })
    }

    pub fn from_json(v: &Value) -> Result<Transition> {
        let (kind, dur, ease, dist, angle) = match v {
            Value::String(s) => {
                // "fade" or "fade 0.4"
                let mut it = s.split_whitespace();
                let k = Self::parse_kind(it.next().unwrap_or("fade"))?;
                let d = it.next().map(|x| x.trim_end_matches('s').parse::<f64>()).transpose()?;
                (k, d, None, None, None)
            }
            Value::Object(o) => {
                let k = Self::parse_kind(o.get("type").and_then(Value::as_str).unwrap_or("fade"))?;
                let d = o.get("duration").and_then(Value::as_f64);
                let e = o.get("ease").and_then(Value::as_str).map(Ease::parse).transpose()?;
                let dist = o.get("distance").and_then(Value::as_f64);
                let angle = o.get("angle").and_then(Value::as_f64);
                (k, d, e, dist, angle)
            }
            _ => bail!("transition must be a string like \"fade\" or an object {{type,duration,ease,distance,angle}}"),
        };
        let push = matches!(kind, TKind::PushLeft | TKind::PushRight | TKind::PushUp | TKind::PushDown);
        let default_ease = match kind {
            TKind::Pop => Ease::BackOut,
            // symmetric curves: an exit and an enter of the same length line up seamlessly
            TKind::WipeLeft | TKind::WipeRight | TKind::WipeUp | TKind::WipeDown | TKind::Iris => Ease::InOut,
            _ if push => Ease::InOut,
            _ => Ease::Out,
        };
        let default_dur = match kind {
            TKind::Pop => 0.45,
            TKind::Iris => 0.6,
            _ if push => 0.6,
            _ => 0.5,
        };
        Ok(Transition {
            kind,
            dur: dur.unwrap_or(default_dur).max(0.001),
            ease: ease.unwrap_or(default_ease),
            dist: dist.unwrap_or(if kind == TKind::SlideFade { 40.0 } else { 120.0 }),
            angle: angle.unwrap_or(90.0),
        })
    }

    /// Apply as an enter transition; `t` = seconds since layer start, `area` = parent box size
    /// (push transitions travel exactly that far, so the layer starts fully outside it).
    pub fn apply_in(&self, t: f64, area: (f64, f64), fx: &mut Fx) {
        if t >= self.dur {
            return;
        }
        let raw = (t / self.dur).clamp(0.0, 1.0);
        self.apply(self.ease.apply(raw), raw, false, area, fx);
    }

    /// Apply as an exit transition; `remaining` = seconds until layer end.
    pub fn apply_out(&self, remaining: f64, area: (f64, f64), fx: &mut Fx) {
        if remaining >= self.dur {
            return;
        }
        let raw = (remaining / self.dur).clamp(0.0, 1.0);
        // exit uses the mirrored curve so "out" motion accelerates away
        self.apply(self.ease.apply(raw), raw, true, area, fx);
    }

    fn apply(&self, p: f64, raw: f64, exit: bool, area: (f64, f64), fx: &mut Fx) {
        let q = 1.0 - p; // remaining distance
        let dir = if exit { -1.0 } else { 1.0 };
        let fade = (raw * 1.6).min(1.0) as f32;
        match self.kind {
            TKind::Fade => fx.opacity *= p.clamp(0.0, 1.0) as f32,
            TKind::SlideUp => {
                fx.dy += dir * q * self.dist;
                fx.opacity *= fade;
            }
            TKind::SlideDown => {
                fx.dy -= dir * q * self.dist;
                fx.opacity *= fade;
            }
            TKind::SlideLeft => {
                fx.dx += dir * q * self.dist;
                fx.opacity *= fade;
            }
            TKind::SlideRight => {
                fx.dx -= dir * q * self.dist;
                fx.opacity *= fade;
            }
            TKind::ZoomIn => {
                fx.scale *= if exit { 1.0 + 0.15 * q } else { 0.85 + 0.15 * p };
                fx.opacity *= fade;
            }
            TKind::ZoomOut => {
                fx.scale *= if exit { 1.0 - 0.15 * q } else { 1.15 - 0.15 * p };
                fx.opacity *= fade;
            }
            TKind::Pop => {
                fx.scale *= p.max(0.0);
                fx.opacity *= (raw * 4.0).min(1.0) as f32;
            }
            TKind::WipeLeft => fx.clip = Some(Clip { side: if exit { Side::Right } else { Side::Left }, frac: p }),
            TKind::WipeRight => fx.clip = Some(Clip { side: if exit { Side::Left } else { Side::Right }, frac: p }),
            TKind::WipeUp => fx.clip = Some(Clip { side: if exit { Side::Bottom } else { Side::Top }, frac: p }),
            TKind::WipeDown => fx.clip = Some(Clip { side: if exit { Side::Top } else { Side::Bottom }, frac: p }),
            // push: enters from (or leaves past) the parent's edge, without fading
            TKind::PushLeft => fx.dx += dir * q * area.0,
            TKind::PushRight => fx.dx -= dir * q * area.0,
            TKind::PushUp => fx.dy += dir * q * area.1,
            TKind::PushDown => fx.dy -= dir * q * area.1,
            TKind::ZoomFade => {
                fx.scale *= 1.0 + 0.25 * q;
                fx.opacity *= p.clamp(0.0, 1.0) as f32;
            }
            TKind::SlideFade => {
                fx.dy += dir * q * self.dist;
                fx.opacity *= p.clamp(0.0, 1.0) as f32;
            }
            TKind::RotateIn => {
                fx.rot -= dir * q * self.angle;
                fx.scale *= 0.5 + 0.5 * p;
                fx.opacity *= fade;
            }
            TKind::FlipX => {
                fx.sx *= p;
                fx.opacity *= (raw * 4.0).min(1.0) as f32;
            }
            TKind::FlipY => {
                fx.sy *= p;
                fx.opacity *= (raw * 4.0).min(1.0) as f32;
            }
            TKind::Iris => fx.clip = Some(Clip { side: Side::Center, frac: p }),
            TKind::ZoomBlur => {
                // rushes in from 130% (exit: out to 130%) with a radial blur that settles as it stops
                fx.scale *= 1.0 + 0.3 * q;
                fx.blur = 0.25 * q;
                fx.opacity *= fade;
            }
        }
    }
}

impl<'de> Deserialize<'de> for Transition {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        Transition::from_json(&v).map_err(serde::de::Error::custom)
    }
}

/// Which edge a wipe reveals toward (`Center`: a circle growing from the middle).
#[derive(Clone, Copy, Debug)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

#[derive(Clone, Copy, Debug)]
pub struct Clip {
    pub side: Side,
    pub frac: f64,
}

impl Clip {
    /// Visible part of a w*h box as (x, y, w, h).
    pub fn rect(&self, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let f = self.frac.clamp(0.0, 1.0);
        match self.side {
            Side::Right => (0.0, 0.0, w * f, h),
            Side::Left => (w * (1.0 - f), 0.0, w * f, h),
            Side::Bottom => (0.0, 0.0, w, h * f),
            Side::Top => (0.0, h * (1.0 - f), w, h * f),
            Side::Center => {
                let r = self.radius(w, h);
                (w / 2.0 - r, h / 2.0 - r, r * 2.0, r * 2.0)
            }
        }
    }

    /// Corner radius of the visible part (a circle for iris reveals).
    pub fn radius(&self, w: f64, h: f64) -> f64 {
        match self.side {
            Side::Center => (w * w + h * h).sqrt() / 2.0 * self.frac.clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

/// Accumulated effect of transitions on a layer for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Fx {
    pub opacity: f32,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    /// extra horizontal / vertical scale (flips)
    pub sx: f64,
    pub sy: f64,
    /// extra rotation in degrees
    pub rot: f64,
    pub clip: Option<Clip>,
    /// zoom blur: extra scale of the outermost of the copies the layer is drawn as (0 = sharp)
    pub blur: f64,
}

impl Default for Fx {
    fn default() -> Self {
        Fx { opacity: 1.0, dx: 0.0, dy: 0.0, scale: 1.0, sx: 1.0, sy: 1.0, rot: 0.0, clip: None, blur: 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn eases_hit_endpoints() {
        for e in [
            Ease::Linear,
            Ease::In,
            Ease::Out,
            Ease::InOut,
            Ease::SineInOut,
            Ease::BackIn,
            Ease::BackOut,
            Ease::Elastic,
            Ease::Bounce,
            Ease::Spring,
        ] {
            assert!(e.apply(0.0).abs() < 1e-6, "{e:?}(0)");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6, "{e:?}(1)");
        }
        assert!(Ease::BackOut.apply(0.6) > 1.0, "back-out overshoots");
    }

    #[test]
    fn keyframes_interpolate_and_hold() {
        let p = Prop::from_json(&json!([[1, 10], [3, 20, "linear"], ["end-1", 0, "hold"]])).unwrap();
        assert_eq!(p.eval(0.0, 10.0), 10.0);
        assert_eq!(p.eval(2.0, 10.0), 15.0);
        assert_eq!(p.eval(8.5, 10.0), 20.0); // hold until the key at 9
        assert_eq!(p.eval(9.5, 10.0), 0.0);
    }

    #[test]
    fn from_to_spans_layer() {
        let p = Prop::from_json(&json!({"from": 1, "to": 2})).unwrap();
        assert_eq!(p.eval(0.0, 4.0), 1.0);
        assert_eq!(p.eval(2.0, 4.0), 1.5);
        assert_eq!(p.eval(4.0, 4.0), 2.0);
    }

    #[test]
    fn transitions_parse_and_apply() {
        let t = Transition::from_json(&json!("slide-up 0.4")).unwrap();
        assert_eq!(t.kind, TKind::SlideUp);
        assert!((t.dur - 0.4).abs() < 1e-9);
        let mut fx = Fx::default();
        t.apply_in(0.0, (100.0, 100.0), &mut fx);
        assert!(fx.dy > 0.0 && fx.opacity == 0.0);
        let mut fx = Fx::default();
        t.apply_in(1.0, (100.0, 100.0), &mut fx);
        assert_eq!(fx.dy, 0.0);
        assert!(Transition::from_json(&json!("explode")).is_err());
    }

    #[test]
    fn push_travels_the_parent_box_and_lines_up() {
        let t = Transition::from_json(&json!("push-left")).unwrap();
        let area = (1920.0, 1080.0);
        let mut fx = Fx::default();
        t.apply_in(0.0, area, &mut fx);
        assert_eq!(fx.dx, 1920.0, "starts one parent width to the right");
        assert_eq!(fx.opacity, 1.0, "push does not fade");
        // an exit and an enter of the same length stay exactly one box apart
        for k in 1..10 {
            let s = k as f64 * t.dur / 10.0;
            let (mut a, mut b) = (Fx::default(), Fx::default());
            t.apply_out(t.dur - s, area, &mut a);
            t.apply_in(s, area, &mut b);
            assert!((b.dx - a.dx - 1920.0).abs() < 1e-6, "at {s}: {} vs {}", a.dx, b.dx);
        }
        let mut fx = Fx::default();
        Transition::from_json(&json!("push-up")).unwrap().apply_in(0.0, area, &mut fx);
        assert_eq!(fx.dy, 1080.0);
    }

    #[test]
    fn new_transitions_settle_at_identity() {
        for name in ["zoom-fade", "zoom-blur", "slide-fade", "rotate-in", "flip-x", "flip-y", "iris"] {
            let t = Transition::from_json(&json!(name)).unwrap();
            let mut start = Fx::default();
            t.apply_in(0.0, (100.0, 100.0), &mut start);
            let mut end = Fx::default();
            t.apply_in(t.dur * 0.999_999, (100.0, 100.0), &mut end);
            assert!((end.scale - 1.0).abs() < 1e-3 && (end.sx - 1.0).abs() < 1e-3 && end.rot.abs() < 1e-3, "{name}");
            assert!(end.opacity > 0.99 && end.blur < 1e-3, "{name}");
            let moved = start.opacity < 1.0 || start.scale != 1.0 || start.sx != 1.0 || start.sy != 1.0 || start.clip.is_some();
            assert!(moved, "{name} does something at the start");
        }
        let t = Transition::from_json(&json!({"type": "rotate-in", "angle": 30})).unwrap();
        let mut fx = Fx::default();
        t.apply_in(0.0, (1.0, 1.0), &mut fx);
        assert_eq!(fx.rot, -30.0);
    }

    #[test]
    fn wipe_reveals_from_side() {
        let c = Clip { side: Side::Right, frac: 0.25 };
        assert_eq!(c.rect(100.0, 50.0), (0.0, 0.0, 25.0, 50.0));
        assert_eq!(c.radius(100.0, 50.0), 0.0);
    }

    #[test]
    fn iris_is_a_circle_that_covers_the_box() {
        let c = Clip { side: Side::Center, frac: 1.0 };
        assert_eq!(c.rect(60.0, 80.0), (-20.0, -10.0, 100.0, 100.0));
        assert_eq!(c.radius(60.0, 80.0), 50.0, "radius reaches the corners");
        let c = Clip { side: Side::Center, frac: 0.0 };
        assert_eq!(c.rect(60.0, 80.0).2, 0.0);
    }
}
