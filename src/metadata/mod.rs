//! Full metadata extraction (title, artist, album, BPM, key, genre, artwork)
//! from audio files, using partial file reads — only file headers are
//! downloaded, not entire files.
//!
//! The parsers are inlined from the `metadata-connect` sibling library;
//! upstream's `src/metadata.ts` glue lives at the bottom of this module.

pub mod extract;
pub mod parsers;
pub mod reader;
pub mod types;

use crate::artwork::reader::NfsFileReader;
use crate::nfs::get_file_info;
use crate::types::{Device, MediaSlot};

pub use extract::{extract_metadata, get_parser_for_extension, get_supported_extensions, is_extension_supported, MetadataParser};
pub use reader::{create_buffer_reader, BufferReader};
pub use types::{ArtworkMimeType, ExtractedMetadata, FileReader, PictureType};

/// Extract full metadata from an audio file using a [`FileReader`].
///
/// This is the low-level API that works with any reader implementation. For
/// extracting from a Pro DJ Link device, use [`extract_metadata_from_device`].
pub async fn extract_full_metadata<R: FileReader>(reader: &R) -> Option<ExtractedMetadata> {
    extract_metadata(reader, None).await
}

/// Check if metadata extraction is supported for a file extension (with or
/// without leading dot).
pub fn is_metadata_extraction_supported(extension: &str) -> bool {
    is_extension_supported(extension)
}

/// Extract full metadata from an audio file on a Pro DJ Link device.
///
/// This function reads only the necessary bytes from the file header,
/// avoiding the need to transfer entire audio files over the network.
/// Returns `None` on any error (file not found, network issues, ...).
pub async fn extract_metadata_from_device(device: &Device, slot: MediaSlot, file_path: &str) -> Option<ExtractedMetadata> {
    let extension = file_path.rsplit('.').next().unwrap_or("").to_lowercase();
    if !is_extension_supported(&extension) {
        return None;
    }

    let file_info = get_file_info(device, slot, file_path).await.ok()?;
    if file_info.size == 0 {
        return None;
    }

    let reader = NfsFileReader::new(device.clone(), slot, file_path.to_string(), u64::from(file_info.size));
    extract_metadata(&reader, None).await
}
