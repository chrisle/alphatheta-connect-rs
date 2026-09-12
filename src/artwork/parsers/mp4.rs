//! Artwork from the MP4 `covr` atom.

use crate::artwork::types::{ExtractedArtwork, FileReader, PictureType};
use crate::metadata::parsers::mp4::{find_atom, find_ilst_atom, find_moov_atom, is_mp4, read_cover_artwork};
use crate::Result;

/// Extract cover art from an MP4/M4A file.
pub async fn extract_from_mp4<R: FileReader>(reader: &R) -> Result<Option<ExtractedArtwork>> {
    if !is_mp4(reader).await? {
        return Ok(None);
    }

    let Some(moov) = find_moov_atom(reader).await? else {
        return Ok(None);
    };
    let Some(ilst) = find_ilst_atom(reader, moov).await? else {
        return Ok(None);
    };
    let Some(covr) = find_atom(reader, ilst.data_offset, ilst.data_offset + ilst.data_size, b"covr").await? else {
        return Ok(None);
    };
    let Some((data, mime_type)) = read_cover_artwork(reader, covr.data_offset, covr.data_size).await? else {
        return Ok(None);
    };

    Ok(Some(ExtractedArtwork { data, mime_type, width: None, height: None, picture_type: Some(PictureType::FrontCover) }))
}
