//! Artwork from ID3v2 APIC frames.

use crate::artwork::types::{ExtractedArtwork, FileReader, PictureType};
use crate::metadata::parsers::id3::{frames, parse_apic_frame, read_id3_frames};
use crate::Result;

/// Extract the front cover (or any other picture) from an MP3's ID3v2 tag.
pub async fn extract_from_mp3<R: FileReader>(reader: &R) -> Result<Option<ExtractedArtwork>> {
    let Some((major_version, tag_data)) = read_id3_frames(reader).await? else {
        return Ok(None);
    };

    let mut front_cover: Option<ExtractedArtwork> = None;
    let mut any_artwork: Option<ExtractedArtwork> = None;

    for frame in frames(major_version, &tag_data) {
        if frame.id != "APIC" {
            continue;
        }
        if let Some(artwork) = parse_apic_frame(frame.data) {
            let artwork: ExtractedArtwork = artwork.into();
            if artwork.picture_type == Some(PictureType::FrontCover) {
                front_cover = Some(artwork);
            } else if any_artwork.is_none() {
                any_artwork = Some(artwork);
            }
        }
    }

    Ok(front_cover.or(any_artwork))
}
