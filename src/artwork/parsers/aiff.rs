//! Artwork from the ID3 chunk of an AIFF file.

use crate::artwork::parsers::id3::extract_from_mp3;
use crate::artwork::reader::BufferReader;
use crate::artwork::types::{ExtractedArtwork, FileReader};
use crate::metadata::parsers::aiff::find_aiff_id3_chunk;
use crate::Result;

/// Extract artwork from an AIFF file by delegating its ID3 chunk to the ID3 parser.
pub async fn extract_from_aiff<R: FileReader>(reader: &R) -> Result<Option<ExtractedArtwork>> {
    let Some((offset, size)) = find_aiff_id3_chunk(reader).await? else {
        return Ok(None);
    };
    let id3_data = reader.read(offset, size).await?;
    let id3_reader = BufferReader::new(id3_data, "mp3");
    extract_from_mp3(&id3_reader).await
}
