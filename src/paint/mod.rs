//! The drawing interface between the scene tree and a backend (CPU or GPU).
//! Every visual is reduced to one primitive: a (rounded) rectangle filled with a
//! solid color or an affinely mapped bitmap.

pub mod cpu;
pub mod gpu;

use crate::gfx::Sprite;
use crate::scene::color::Rgba;
use tiny_skia::{Pixmap, Transform};

pub enum Src<'a> {
    Solid(Rgba),
    /// static bitmap; its address is used as a cache key by the GPU backend
    Image(&'a Pixmap),
    /// a decoded video frame (opaque RGBA), `ver` changes whenever the content does
    Frame { key: usize, ver: i64, data: &'a [u8], w: u32, h: u32 },
}

#[derive(Clone, Copy)]
pub struct Clip {
    /// clip-local -> canvas pixels
    pub ts: Transform,
    pub rect: [f32; 4],
}

pub struct Op<'a> {
    pub src: Src<'a>,
    /// shape-local -> canvas pixels
    pub ts: Transform,
    /// x0, y0, x1, y1 in shape-local units
    pub shape: [f32; 4],
    pub radius: f32,
    /// source pixels -> shape-local
    pub uv: Transform,
    pub opacity: f32,
    pub clip: Option<Clip>,
}

pub trait Painter<'a> {
    fn draw(&mut self, op: Op<'a>);
}

/// Draw a pre-rendered sprite so its logical box lands at local (0,0).
pub fn sprite_op<'a>(s: &'a Sprite, ts: Transform, opacity: f32, clip: Option<Clip>) -> Op<'a> {
    let (pw, ph) = (s.pm.width() as f32, s.pm.height() as f32);
    let p = s.ppu;
    Op {
        src: Src::Image(&s.pm),
        ts,
        shape: [-s.ox / p, -s.oy / p, (pw - s.ox) / p, (ph - s.oy) / p],
        radius: 0.0,
        uv: Transform::from_row(1.0 / p, 0.0, 0.0, 1.0 / p, -s.ox / p, -s.oy / p),
        opacity,
        clip,
    }
}
