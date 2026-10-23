//! Everything that touches media files through ffmpeg/ffprobe.

pub mod audio;
pub mod beats;
pub mod colorspace;
pub mod encode;
pub mod image;
pub mod probe;
pub mod process;
pub mod video;

pub use encode::Encoder;
pub use probe::{probe, MediaInfo};
pub use process::{ffmpeg, run};
