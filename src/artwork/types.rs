//! Artwork extraction types.

use serde::{Deserialize, Serialize};

pub use crate::metadata::types::{ArtworkMimeType, FileReader, PictureType};

/// Extracted artwork from an audio file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedArtwork {
    pub data: Vec<u8>,
    pub mime_type: ArtworkMimeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_type: Option<PictureType>,
}

impl From<crate::metadata::types::ArtworkResult> for ExtractedArtwork {
    fn from(r: crate::metadata::types::ArtworkResult) -> Self {
        Self { data: r.data, mime_type: r.mime_type, width: r.width, height: r.height, picture_type: Some(r.picture_type) }
    }
}
