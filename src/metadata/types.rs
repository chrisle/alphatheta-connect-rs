//! Types for audio metadata extraction.

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::Result;

/// Interface for reading file data at arbitrary offsets. This allows
/// extracting metadata from remote files by reading only the necessary bytes.
pub trait FileReader: Send + Sync {
    /// Total file size in bytes.
    fn size(&self) -> u64;
    /// File extension (without dot), e.g. 'mp3', 'flac', 'm4a'.
    fn extension(&self) -> &str;
    /// Read bytes from the file at a given offset. May return fewer bytes
    /// than asked for at the end of the file.
    fn read(&self, offset: u64, length: u64) -> impl Future<Output = Result<Vec<u8>>> + Send;
}

/// Standard picture types from ID3v2 / FLAC specs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PictureType {
    Other,
    FileIcon32x32,
    OtherFileIcon,
    FrontCover,
    BackCover,
    LeafletPage,
    Media,
    LeadArtist,
    Artist,
    Conductor,
    Band,
    Composer,
    Lyricist,
    RecordingLocation,
    DuringRecording,
    DuringPerformance,
    MovieScreenCapture,
    BrightColoredFish,
    Illustration,
    BandLogotype,
    PublisherLogotype,
    Unknown(u32),
}

impl PictureType {
    pub const fn from_u32(v: u32) -> Self {
        match v {
            0 => PictureType::Other,
            1 => PictureType::FileIcon32x32,
            2 => PictureType::OtherFileIcon,
            3 => PictureType::FrontCover,
            4 => PictureType::BackCover,
            5 => PictureType::LeafletPage,
            6 => PictureType::Media,
            7 => PictureType::LeadArtist,
            8 => PictureType::Artist,
            9 => PictureType::Conductor,
            10 => PictureType::Band,
            11 => PictureType::Composer,
            12 => PictureType::Lyricist,
            13 => PictureType::RecordingLocation,
            14 => PictureType::DuringRecording,
            15 => PictureType::DuringPerformance,
            16 => PictureType::MovieScreenCapture,
            17 => PictureType::BrightColoredFish,
            18 => PictureType::Illustration,
            19 => PictureType::BandLogotype,
            20 => PictureType::PublisherLogotype,
            other => PictureType::Unknown(other),
        }
    }
}

/// MIME types supported for artwork.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArtworkMimeType {
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/gif")]
    Gif,
}

impl ArtworkMimeType {
    pub const fn as_str(self) -> &'static str {
        match self {
            ArtworkMimeType::Jpeg => "image/jpeg",
            ArtworkMimeType::Png => "image/png",
            ArtworkMimeType::Gif => "image/gif",
        }
    }
}

/// Extracted metadata from an audio file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractedMetadata {
    /// Track title.
    pub title: Option<String>,
    /// Artist name.
    pub artist: Option<String>,
    /// Album name.
    pub album: Option<String>,
    /// Genre.
    pub genre: Option<String>,
    /// Record label / publisher.
    pub label: Option<String>,
    /// Release year.
    pub year: Option<u32>,
    /// Beats per minute.
    pub bpm: Option<f64>,
    /// Musical key (e.g., "Am", "C#m", "5A").
    pub key: Option<String>,
    /// Artwork image data.
    pub artwork: Option<Vec<u8>>,
    /// MIME type of the artwork.
    pub artwork_mime_type: Option<ArtworkMimeType>,
}

impl ExtractedMetadata {
    /// True when no field was found.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.album.is_none()
            && self.genre.is_none()
            && self.label.is_none()
            && self.year.is_none()
            && self.bpm.is_none()
            && self.key.is_none()
            && self.artwork.is_none()
    }
}

/// An artwork parse result shared by the parsers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkResult {
    pub data: Vec<u8>,
    pub mime_type: ArtworkMimeType,
    pub picture_type: PictureType,
    pub width: Option<u32>,
    pub height: Option<u32>,
}
