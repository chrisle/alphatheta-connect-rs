//! Artwork extraction directly from audio files on connected media via NFS.

pub mod parsers;
pub mod reader;
pub mod types;

use crate::logger::{noop_logger, SharedLogger};
use crate::nfs::get_file_info;
use crate::types::{Device, MediaSlot};
use crate::Result;

use parsers::{extract_from_aiff, extract_from_flac, extract_from_mp3, extract_from_mp4};

pub use reader::{create_buffer_reader, create_nfs_file_reader, create_nfs_file_reader_with_info, BufferReader, NfsFileReader};
pub use types::{ArtworkMimeType, ExtractedArtwork, FileReader, PictureType};

const SUPPORTED_EXTENSIONS: &[&str] = &["mp3", "m4a", "mp4", "aac", "flac", "aiff", "aif"];

/// True for the extensions artwork can be extracted from.
pub fn is_artwork_extraction_supported(extension: &str) -> bool {
    SUPPORTED_EXTENSIONS.contains(&extension.to_lowercase().as_str())
}

/// Extract artwork from a file, choosing the parser by the reader's extension.
pub async fn extract_artwork<R: FileReader>(reader: &R) -> Result<Option<ExtractedArtwork>> {
    let ext = reader.extension().to_lowercase();
    match ext.as_str() {
        "mp3" => extract_from_mp3(reader).await,
        "m4a" | "mp4" | "aac" => extract_from_mp4(reader).await,
        "flac" => extract_from_flac(reader).await,
        "aiff" | "aif" => extract_from_aiff(reader).await,
        _ => {
            if let Some(mp3) = extract_from_mp3(reader).await? {
                return Ok(Some(mp3));
            }
            extract_from_mp4(reader).await
        }
    }
}

/// Extract artwork from a file on a device's media slot, over NFS.
pub async fn extract_artwork_from_device(
    device: &Device,
    slot: MediaSlot,
    file_path: &str,
    logger: Option<SharedLogger>,
) -> Result<Option<ExtractedArtwork>> {
    let logger = logger.unwrap_or_else(noop_logger);
    logger.debug(&format!("[artwork-nfs] getFileInfo: device={}, slot={:?}, path={file_path}", device.ip, slot));
    let file_info = get_file_info(device, slot, file_path).await?;
    logger.debug(&format!("[artwork-nfs] File found: {} bytes", file_info.size));
    let reader = create_nfs_file_reader(device, slot, file_path, u64::from(file_info.size));
    let result = extract_artwork(&reader).await?;
    logger.debug(&format!(
        "[artwork-nfs] extractArtwork result: {}",
        result.as_ref().map(|r| format!("{} ({}b)", r.mime_type.as_str(), r.data.len())).unwrap_or_else(|| "null".into())
    ));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::parsers::id3::fixtures::{apic, tag, text_frame, JPEG, PNG};

    #[tokio::test]
    async fn prefers_front_cover() {
        let file = tag(&[text_frame("TIT2", "x"), apic("image/png", 0, PNG), apic("image/jpeg", 3, JPEG)]);
        let art = extract_artwork(&BufferReader::new(file, "mp3")).await.unwrap().unwrap();
        assert_eq!(art.data, JPEG);
        assert_eq!(art.picture_type, Some(PictureType::FrontCover));
    }

    #[tokio::test]
    async fn falls_back_to_any_artwork_and_sniffs_unknown_extensions() {
        let file = tag(&[apic("image/png", 4, PNG)]);
        let art = extract_artwork(&BufferReader::new(file.clone(), "mp3")).await.unwrap().unwrap();
        assert_eq!(art.mime_type, ArtworkMimeType::Png);
        let sniffed = extract_artwork(&BufferReader::new(file, "bin")).await.unwrap().unwrap();
        assert_eq!(sniffed.mime_type, ArtworkMimeType::Png);
        assert!(extract_artwork(&BufferReader::new(vec![0; 32], "wav")).await.unwrap().is_none());
    }

    #[test]
    fn supported_extensions() {
        assert!(is_artwork_extraction_supported("MP3"));
        assert!(is_artwork_extraction_supported("aif"));
        assert!(!is_artwork_extraction_supported("wav"));
    }
}
