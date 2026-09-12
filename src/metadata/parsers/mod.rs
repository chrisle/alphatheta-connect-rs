//! Tag parsers for the supported audio formats.

pub mod aiff;
pub mod flac;
pub mod id3;
pub mod mp4;
pub mod utils;

pub use aiff::extract_from_aiff;
pub use flac::extract_from_flac;
pub use id3::extract_from_mp3;
pub use mp4::extract_from_mp4;
pub use utils::{detect_image_type, normalize_mime_type};
