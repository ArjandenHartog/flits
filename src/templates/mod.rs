//! Ready-made scenes. Each template returns a scene as JSON, which the CLI writes next to
//! the output (so it can be edited) and renders.

pub mod captions;
pub mod cut;
pub mod edit;
pub mod export;
pub mod fillers;
pub mod podcast;
pub mod slideshow;
pub mod starters;

pub use captions::{captioned_video, CaptionOpts};
pub use podcast::{podcast, PodcastOpts};

use anyhow::{bail, Result};
use std::path::Path;

/// Absolute path as a string (scenes may be moved around).
pub fn abs(p: &Path) -> String {
    std::fs::canonicalize(p)
        .map(|p| p.to_string_lossy().trim_start_matches(r"\\?\").to_string())
        .unwrap_or_else(|_| p.to_string_lossy().to_string())
}

/// Canvas size for a format name.
pub fn canvas_for(format: &str) -> Result<(u32, u32)> {
    Ok(match format {
        "16:9" | "landscape" | "youtube" | "1080p" => (1920, 1080),
        "9:16" | "portrait" | "reel" | "story" | "tiktok" | "short" => (1080, 1920),
        "1:1" | "square" => (1080, 1080),
        "4:5" | "instagram" => (1080, 1350),
        "4k" => (3840, 2160),
        "720p" => (1280, 720),
        f => bail!("unknown format '{f}' (16:9, 9:16, 1:1, 4:5, 720p, 4k)"),
    })
}
