//! Decoded video frames are planar YUV 4:2:0. Converting to RGB (and tone mapping
//! HDR to SDR) happens on the GPU in `composite.wgsl`; this module describes the
//! frame format and holds the matching CPU implementation for the software path.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Transfer {
    /// gamma-encoded SDR (BT.709 / sRGB)
    #[default]
    Sdr,
    /// HLG (iPhone / Android HDR video)
    Hlg,
    /// PQ / SMPTE 2084 (HDR10, Dolby Vision base layer)
    Pq,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Matrix {
    #[default]
    Bt709,
    Bt2020,
    Bt601,
}

/// Layout and colour of a planar YUV 4:2:0 frame: Y plane, then U, then V.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FrameFormat {
    /// samples are 16-bit little endian holding 10-bit values (yuv420p10le); else 8-bit
    pub high_bit: bool,
    pub transfer: Transfer,
    pub matrix: Matrix,
    pub full_range: bool,
}

impl FrameFormat {
    pub fn bytes_per_sample(&self) -> usize {
        if self.high_bit {
            2
        } else {
            1
        }
    }

    pub fn frame_size(&self, w: u32, h: u32) -> usize {
        (w * h + 2 * (w / 2) * (h / 2)) as usize * self.bytes_per_sample()
    }

    /// Factor that turns a normalized sample into a normalized code value
    /// (10-bit values stored in 16-bit containers: 65535 / 1023).
    pub fn sample_scale(&self) -> f32 {
        if self.high_bit {
            65535.0 / 1023.0
        } else {
            1.0
        }
    }

    /// Shader parameters: (transfer, matrix, full range, sample scale).
    pub fn shader_params(&self) -> [f32; 4] {
        let t = match self.transfer {
            Transfer::Sdr => 0.0,
            Transfer::Hlg => 1.0,
            Transfer::Pq => 2.0,
        };
        let m = match self.matrix {
            Matrix::Bt709 => 0.0,
            Matrix::Bt2020 => 1.0,
            Matrix::Bt601 => 2.0,
        };
        [t, m, if self.full_range { 1.0 } else { 0.0 }, self.sample_scale()]
    }

    pub fn from_probe(transfer: &str, matrix: &str, range: &str) -> FrameFormat {
        FrameFormat {
            high_bit: false,
            transfer: match transfer {
                "arib-std-b67" => Transfer::Hlg,
                "smpte2084" => Transfer::Pq,
                _ => Transfer::Sdr,
            },
            matrix: match matrix {
                "bt2020nc" | "bt2020c" | "bt2020_ncl" | "bt2020_cl" => Matrix::Bt2020,
                "smpte170m" | "bt470bg" | "fcc" => Matrix::Bt601,
                _ => Matrix::Bt709,
            },
            full_range: range == "pc" || range == "jpeg",
        }
    }

    pub fn is_hdr(&self) -> bool {
        self.transfer != Transfer::Sdr
    }
}

// ---------------------------------------------------------------- reference math (mirrors the shader)

fn hlg_inverse_oetf(e: f32) -> f32 {
    const A: f32 = 0.178_832_77;
    const B: f32 = 0.284_668_92;
    const C: f32 = 0.559_910_7;
    if e <= 0.5 {
        e * e / 3.0
    } else {
        (((e - C) / A).exp() + B) / 12.0
    }
}

fn pq_eotf(e: f32) -> f32 {
    const M1: f32 = 0.159_301_76;
    const M2: f32 = 78.84375;
    const C1: f32 = 0.835_937_5;
    const C2: f32 = 18.851_563;
    const C3: f32 = 18.6875;
    let p = e.max(0.0).powf(1.0 / M2);
    10000.0 * ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1)
}

fn hable(x: f32) -> f32 {
    const A: f32 = 0.15;
    const B: f32 = 0.50;
    const C: f32 = 0.10;
    const D: f32 = 0.20;
    const E: f32 = 0.02;
    const F: f32 = 0.30;
    (x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F) - E / F
}

fn bt709_oetf(l: f32) -> f32 {
    if l < 0.018 {
        4.5 * l
    } else {
        1.099 * l.powf(0.45) - 0.099
    }
}

/// Reference white for HDR -> SDR (nits), same as ffmpeg's zscale npl=203.
pub const SDR_WHITE: f32 = 203.0;

/// HDR (BT.2020, HLG/PQ encoded) -> SDR BT.709 gamma-encoded RGB, hable tone curve.
pub fn hdr_to_sdr(rgb: [f32; 3], transfer: Transfer) -> [f32; 3] {
    let (nits, peak) = match transfer {
        Transfer::Hlg => {
            let e = rgb.map(hlg_inverse_oetf);
            let ys = 0.2627 * e[0] + 0.6780 * e[1] + 0.0593 * e[2];
            let g = 1000.0 * ys.max(1e-6).powf(0.2); // HLG OOTF, 1000-nit display
            (e.map(|c| c * g), 1000.0)
        }
        Transfer::Pq => (rgb.map(pq_eotf), 1000.0),
        Transfer::Sdr => return rgb,
    };
    let x = nits.map(|n| n / SDR_WHITE);
    // BT.2020 -> BT.709 primaries (linear)
    let l = [
        1.6605 * x[0] - 0.5876 * x[1] - 0.0728 * x[2],
        -0.1246 * x[0] + 1.1329 * x[1] - 0.0083 * x[2],
        -0.0182 * x[0] - 0.1006 * x[1] + 1.1187 * x[2],
    ]
    .map(|c| c.max(0.0));
    let sig = l[0].max(l[1]).max(l[2]).max(1e-6);
    let mapped = hable(sig) / hable(peak / SDR_WHITE);
    let k = mapped / sig;
    l.map(|c| bt709_oetf((c * k).clamp(0.0, 1.0)))
}

/// One YUV sample triple (normalized 0..1 code values) -> RGB 0..1.
pub fn yuv_to_rgb(y: f32, cb: f32, cr: f32, f: &FrameFormat) -> [f32; 3] {
    let (y, cb, cr) = if f.full_range {
        (y, cb - 0.5, cr - 0.5)
    } else {
        ((y - 16.0 / 255.0) * (255.0 / 219.0), (cb - 128.0 / 255.0) * (255.0 / 224.0), (cr - 128.0 / 255.0) * (255.0 / 224.0))
    };
    let (kr, kb) = match f.matrix {
        Matrix::Bt709 => (0.2126, 0.0722),
        Matrix::Bt2020 => (0.2627, 0.0593),
        Matrix::Bt601 => (0.299, 0.114),
    };
    let kg = 1.0 - kr - kb;
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    let g = (y - kr * r - kb * b) / kg;
    let rgb = [r, g, b].map(|c| c.clamp(0.0, 1.0));
    if f.is_hdr() {
        hdr_to_sdr(rgb, f.transfer)
    } else {
        rgb
    }
}

/// Convert a whole planar frame to opaque RGBA (software path).
pub fn frame_to_rgba(data: &[u8], w: u32, h: u32, f: &FrameFormat) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let (cw, ch) = (w / 2, h / 2);
    let bps = f.bytes_per_sample();
    let sample = |plane_off: usize, idx: usize| -> f32 {
        if bps == 2 {
            let i = plane_off + idx * 2;
            u16::from_le_bytes([data[i], data[i + 1]]) as f32 / 1023.0
        } else {
            data[plane_off + idx] as f32 / 255.0
        }
    };
    let u_off = w * h * bps;
    let v_off = u_off + cw * ch * bps;
    let mut out = vec![255u8; w * h * 4];
    // SDR 8-bit: per-pixel float math is fine for a fallback path; HDR needs it anyway
    for y in 0..h {
        for x in 0..w {
            let ci = (y / 2).min(ch.saturating_sub(1)) * cw + (x / 2).min(cw.saturating_sub(1));
            let rgb = yuv_to_rgb(sample(0, y * w + x), sample(u_off, ci), sample(v_off, ci), f);
            let o = (y * w + x) * 4;
            out[o] = (rgb[0] * 255.0 + 0.5) as u8;
            out[o + 1] = (rgb[1] * 255.0 + 0.5) as u8;
            out[o + 2] = (rgb[2] * 255.0 + 0.5) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdr_limited_range_white_and_black() {
        let f = FrameFormat::default();
        let w = yuv_to_rgb(235.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, &f);
        let b = yuv_to_rgb(16.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, &f);
        assert!(w.iter().all(|c| (c - 1.0).abs() < 1e-3), "{w:?}");
        assert!(b.iter().all(|c| c.abs() < 1e-3), "{b:?}");
    }

    #[test]
    fn hlg_reference_white_maps_near_sdr_white() {
        // HLG 75% signal = reference white (203 nits)
        let out = hdr_to_sdr([0.75, 0.75, 0.75], Transfer::Hlg);
        assert!(out[0] > 0.6 && out[0] < 1.0, "{out:?}");
        assert!((out[0] - out[1]).abs() < 1e-3, "neutral stays neutral");
    }

    #[test]
    fn converts_frames() {
        let f = FrameFormat::default();
        let (w, h) = (4u32, 2u32);
        let mut d = vec![235u8; (w * h) as usize];
        d.extend(vec![128u8; 4]); // U + V planes (2x1 each)
        let rgba = frame_to_rgba(&d, w, h, &f);
        assert_eq!(rgba.len(), 32);
        assert!(rgba.iter().all(|&v| v >= 254));
    }
}
