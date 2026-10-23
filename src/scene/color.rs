use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer};

/// Straight (non-premultiplied) RGBA color, components 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

    pub fn rgba8(r: u8, g: u8, b: u8, a: f32) -> Rgba {
        Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
    }

    pub fn parse(input: &str) -> Result<Rgba> {
        let s = input.trim().to_ascii_lowercase();
        let named = match s.as_str() {
            "white" => Some("#ffffff"),
            "black" => Some("#000000"),
            "transparent" => Some("#00000000"),
            "red" => Some("#ff3b30"),
            "green" => Some("#34c759"),
            "blue" => Some("#0a84ff"),
            "yellow" => Some("#ffd60a"),
            "orange" => Some("#ff9f0a"),
            "purple" => Some("#bf5af2"),
            "pink" => Some("#ff375f"),
            "gray" | "grey" => Some("#8e8e93"),
            _ => None,
        };
        let s = named.map(str::to_string).unwrap_or(s);
        if let Some(h) = s.strip_prefix('#') {
            let hex = |i: usize| -> Result<u8> { u8::from_str_radix(&h[i..i + 2], 16).with_context(|| format!("bad color '{input}'")) };
            let short = |i: usize| -> Result<u8> {
                let v = u8::from_str_radix(&h[i..i + 1], 16).with_context(|| format!("bad color '{input}'"))?;
                Ok(v * 17)
            };
            return Ok(match h.len() {
                3 => Rgba::rgba8(short(0)?, short(1)?, short(2)?, 1.0),
                4 => Rgba::rgba8(short(0)?, short(1)?, short(2)?, short(3)? as f32 / 255.0),
                6 => Rgba::rgba8(hex(0)?, hex(2)?, hex(4)?, 1.0),
                8 => Rgba::rgba8(hex(0)?, hex(2)?, hex(4)?, hex(6)? as f32 / 255.0),
                _ => bail!("bad color '{input}'"),
            });
        }
        let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb(")).and_then(|x| x.strip_suffix(')'));
        if let Some(inner) = inner {
            let parts: Vec<f32> = inner
                .split(',')
                .map(|p| p.trim().parse::<f32>())
                .collect::<Result<_, _>>()
                .with_context(|| format!("bad color '{input}'"))?;
            if parts.len() == 3 || parts.len() == 4 {
                return Ok(Rgba { r: parts[0] / 255.0, g: parts[1] / 255.0, b: parts[2] / 255.0, a: *parts.get(3).unwrap_or(&1.0) });
            }
        }
        bail!("unknown color '{input}' (use #rrggbb, #rrggbbaa or rgba(r,g,b,a))")
    }

    pub fn skia(&self, opacity: f32) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(
            self.r.clamp(0.0, 1.0),
            self.g.clamp(0.0, 1.0),
            self.b.clamp(0.0, 1.0),
            (self.a * opacity).clamp(0.0, 1.0),
        )
        .unwrap()
    }

    pub fn with_alpha(self, a: f32) -> Rgba {
        Rgba { a, ..self }
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgba::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::Rgba;

    #[test]
    fn parses_all_forms() {
        assert_eq!(Rgba::parse("#fff").unwrap(), Rgba::WHITE);
        assert_eq!(Rgba::parse("#000000").unwrap(), Rgba::BLACK);
        let c = Rgba::parse("#ff000080").unwrap();
        assert!((c.a - 128.0 / 255.0).abs() < 1e-6 && c.r == 1.0);
        let c = Rgba::parse("rgba(255, 0, 0, 0.5)").unwrap();
        assert_eq!((c.r, c.a), (1.0, 0.5));
        assert_eq!(Rgba::parse(" White ").unwrap(), Rgba::WHITE);
        assert!(Rgba::parse("#12").is_err());
        assert!(Rgba::parse("blurple").is_err());
    }
}
