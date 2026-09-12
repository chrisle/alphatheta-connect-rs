//! Artwork from FLAC PICTURE blocks.

use crate::artwork::types::{ExtractedArtwork, FileReader, PictureType};
use crate::metadata::parsers::flac::{block_type, flac_blocks, parse_picture_block};
use crate::Result;

/// Extract the front cover (or any other picture) from a FLAC file.
pub async fn extract_from_flac<R: FileReader>(reader: &R) -> Result<Option<ExtractedArtwork>> {
    let Some(blocks) = flac_blocks(reader).await? else {
        return Ok(None);
    };

    let mut front_cover: Option<ExtractedArtwork> = None;
    let mut any_artwork: Option<ExtractedArtwork> = None;

    for (bt, offset, length) in blocks {
        if bt != block_type::PICTURE {
            continue;
        }
        let data = reader.read(offset, length).await?;
        if let Some(artwork) = parse_picture_block(&data) {
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
