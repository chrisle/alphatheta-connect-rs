//! FLAC metadata block parsing.

use std::collections::HashMap;

use crate::metadata::parsers::utils::{clean_text, normalize_mime_type, parse_bpm, parse_year};
use crate::metadata::types::{ArtworkResult, ExtractedMetadata, FileReader, PictureType};
use crate::Result;

/// FLAC metadata block types.
pub mod block_type {
    pub const STREAMINFO: u8 = 0;
    pub const PADDING: u8 = 1;
    pub const APPLICATION: u8 = 2;
    pub const SEEKTABLE: u8 = 3;
    pub const VORBIS_COMMENT: u8 = 4;
    pub const CUESHEET: u8 = 5;
    pub const PICTURE: u8 = 6;
}

/// Vorbis comment field names (case-insensitive).
mod vorbis_fields {
    pub const TITLE: &[&str] = &["TITLE"];
    pub const ARTIST: &[&str] = &["ARTIST"];
    pub const ALBUM: &[&str] = &["ALBUM"];
    pub const GENRE: &[&str] = &["GENRE"];
    /// Vorbis never standardised a label field, so taggers picked their own:
    /// ORGANIZATION is the one the spec suggests for the producing entity,
    /// and LABEL/PUBLISHER are what everything from Picard to Mixed In Key
    /// writes.
    pub const LABEL: &[&str] = &["LABEL", "PUBLISHER", "ORGANIZATION", "ORGANISATION"];
    pub const DATE: &[&str] = &["DATE", "YEAR"];
    pub const BPM: &[&str] = &["BPM", "TEMPO"];
    pub const KEY: &[&str] = &["KEY", "INITIALKEY"];
}

fn u32_be(d: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

fn u32_le(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

/// Parse a FLAC PICTURE metadata block.
pub fn parse_picture_block(data: &[u8]) -> Option<ArtworkResult> {
    if data.len() < 32 {
        return None;
    }

    let mut offset = 0usize;
    let picture_type = PictureType::from_u32(u32_be(data, offset));
    offset += 4;

    let mime_length = u32_be(data, offset) as usize;
    offset += 4;
    if offset + mime_length > data.len() {
        return None;
    }
    let mime_type = String::from_utf8_lossy(&data[offset..offset + mime_length]).into_owned();
    offset += mime_length;

    if offset + 4 > data.len() {
        return None;
    }
    let desc_length = u32_be(data, offset) as usize;
    offset += 4 + desc_length;

    if offset + 16 > data.len() {
        return None;
    }
    let width = u32_be(data, offset);
    offset += 4;
    let height = u32_be(data, offset);
    // Skip depth and colors
    offset += 4 + 8;

    if offset + 4 > data.len() {
        return None;
    }
    let image_length = u32_be(data, offset) as usize;
    offset += 4;

    if offset + image_length > data.len() {
        return None;
    }
    let image_data = &data[offset..offset + image_length];
    if image_data.is_empty() {
        return None;
    }

    Some(ArtworkResult {
        data: image_data.to_vec(),
        mime_type: normalize_mime_type(&mime_type),
        width: (width > 0).then_some(width),
        height: (height > 0).then_some(height),
        picture_type,
    })
}

/// Parse a Vorbis comment block. Format: vendor string length (32-bit LE) +
/// vendor string + comment count (32-bit LE) + comments; each comment:
/// length (32-bit LE) + "FIELD=value".
pub fn parse_vorbis_comment_block(data: &[u8]) -> HashMap<String, String> {
    let mut comments = HashMap::new();
    if data.len() < 8 {
        return comments;
    }

    let mut offset = 0usize;
    // Skip vendor string
    let vendor_length = u32_le(data, offset) as usize;
    offset += 4 + vendor_length;
    if offset + 4 > data.len() {
        return comments;
    }

    let comment_count = u32_le(data, offset);
    offset += 4;

    for _ in 0..comment_count {
        if offset + 4 > data.len() {
            break;
        }
        let comment_length = u32_le(data, offset) as usize;
        offset += 4;
        if offset + comment_length > data.len() {
            break;
        }
        let comment = String::from_utf8_lossy(&data[offset..offset + comment_length]).into_owned();
        offset += comment_length;

        // Split on first '='
        if let Some(eq) = comment.find('=') {
            if eq > 0 {
                let field = comment[..eq].to_uppercase();
                let value = comment[eq + 1..].to_string();
                // Only store first value for each field
                comments.entry(field).or_insert(value);
            }
        }
    }

    comments
}

fn get_field_value(comments: &HashMap<String, String>, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(v) = comments.get(*name) {
            if !v.is_empty() {
                return clean_text(Some(v.clone()));
            }
        }
    }
    None
}

/// The metadata blocks of a FLAC file: (is_last, block_type, offset of the
/// block body, length).
pub async fn flac_blocks<R: FileReader>(reader: &R) -> Result<Option<Vec<(u8, u64, u64)>>> {
    let signature = reader.read(0, 4).await?;
    if signature != b"fLaC" {
        return Ok(None);
    }

    let mut blocks = Vec::new();
    let mut offset = 4u64;
    let mut is_last = false;
    while !is_last && offset < reader.size() {
        let header = reader.read(offset, 4).await?;
        if header.len() < 4 {
            break;
        }
        is_last = header[0] & 0x80 != 0;
        let block_type = header[0] & 0x7f;
        let block_length = (u64::from(header[1]) << 16) | (u64::from(header[2]) << 8) | u64::from(header[3]);
        if block_length == 0 || offset + 4 + block_length > reader.size() {
            break;
        }
        blocks.push((block_type, offset + 4, block_length));
        offset += 4 + block_length;
    }
    Ok(Some(blocks))
}

/// Extract metadata from a FLAC file.
pub async fn extract_from_flac<R: FileReader>(reader: &R) -> Result<Option<ExtractedMetadata>> {
    let Some(blocks) = flac_blocks(reader).await? else {
        return Ok(None);
    };

    let mut metadata = ExtractedMetadata::default();
    let mut front_cover: Option<ArtworkResult> = None;
    let mut any_artwork: Option<ArtworkResult> = None;

    for (block_type, body_offset, length) in blocks {
        if block_type == block_type::VORBIS_COMMENT {
            let data = reader.read(body_offset, length).await?;
            let comments = parse_vorbis_comment_block(&data);

            metadata.title = metadata.title.take().or_else(|| get_field_value(&comments, vorbis_fields::TITLE));
            metadata.artist = metadata.artist.take().or_else(|| get_field_value(&comments, vorbis_fields::ARTIST));
            metadata.album = metadata.album.take().or_else(|| get_field_value(&comments, vorbis_fields::ALBUM));
            metadata.genre = metadata.genre.take().or_else(|| get_field_value(&comments, vorbis_fields::GENRE));
            metadata.label = metadata.label.take().or_else(|| get_field_value(&comments, vorbis_fields::LABEL));

            if metadata.year.is_none() {
                if let Some(date) = get_field_value(&comments, vorbis_fields::DATE) {
                    metadata.year = parse_year(&date);
                }
            }
            if metadata.bpm.is_none() {
                if let Some(bpm) = get_field_value(&comments, vorbis_fields::BPM) {
                    metadata.bpm = parse_bpm(&bpm);
                }
            }
            metadata.key = metadata.key.take().or_else(|| get_field_value(&comments, vorbis_fields::KEY));
        } else if block_type == block_type::PICTURE {
            let data = reader.read(body_offset, length).await?;
            if let Some(artwork) = parse_picture_block(&data) {
                if artwork.picture_type == PictureType::FrontCover {
                    front_cover = Some(artwork);
                } else if any_artwork.is_none() {
                    any_artwork = Some(artwork);
                }
            }
        }
    }

    if let Some(artwork) = front_cover.or(any_artwork) {
        metadata.artwork = Some(artwork.data);
        metadata.artwork_mime_type = Some(artwork.mime_type);
    }

    if metadata.is_empty() {
        return Ok(None);
    }

    Ok(Some(metadata))
}

#[cfg(test)]
pub(crate) mod fixtures {
    pub fn block(block_type: u8, is_last: bool, body: &[u8]) -> Vec<u8> {
        let mut b = vec![block_type | if is_last { 0x80 } else { 0 }];
        let len = body.len() as u32;
        b.push(((len >> 16) & 0xff) as u8);
        b.push(((len >> 8) & 0xff) as u8);
        b.push((len & 0xff) as u8);
        b.extend_from_slice(body);
        b
    }

    pub fn vorbis_comments(fields: &[(&str, &str)]) -> Vec<u8> {
        let mut b = Vec::new();
        let vendor = b"test";
        b.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        b.extend_from_slice(vendor);
        b.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        for (k, v) in fields {
            let c = format!("{k}={v}");
            b.extend_from_slice(&(c.len() as u32).to_le_bytes());
            b.extend_from_slice(c.as_bytes());
        }
        b
    }

    pub fn picture(picture_type: u32, mime: &str, image: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&picture_type.to_be_bytes());
        b.extend_from_slice(&(mime.len() as u32).to_be_bytes());
        b.extend_from_slice(mime.as_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&500u32.to_be_bytes());
        b.extend_from_slice(&500u32.to_be_bytes());
        b.extend_from_slice(&24u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&(image.len() as u32).to_be_bytes());
        b.extend_from_slice(image);
        b
    }

    pub fn file(blocks: &[Vec<u8>]) -> Vec<u8> {
        let mut f = b"fLaC".to_vec();
        for b in blocks {
            f.extend_from_slice(b);
        }
        f.extend_from_slice(&[0xff, 0xf8, 0, 0]);
        f
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::metadata::parsers::id3::fixtures::JPEG;
    use crate::metadata::reader::BufferReader;
    use crate::metadata::types::ArtworkMimeType;

    #[tokio::test]
    async fn extracts_vorbis_comments_and_picture() {
        let f = file(&[
            block(block_type::STREAMINFO, false, &[0u8; 34]),
            block(
                block_type::VORBIS_COMMENT,
                false,
                &vorbis_comments(&[
                    ("TITLE", "T"),
                    ("artist", "A"),
                    ("DATE", "2020-05-01"),
                    ("BPM", "124.5"),
                    ("INITIALKEY", "8A"),
                    ("PUBLISHER", "L"),
                ]),
            ),
            block(block_type::PICTURE, true, &picture(3, "image/jpeg", JPEG)),
        ]);
        let reader = BufferReader::new(f, "flac");
        let m = extract_from_flac(&reader).await.unwrap().unwrap();
        assert_eq!(m.title.as_deref(), Some("T"));
        assert_eq!(m.artist.as_deref(), Some("A"));
        assert_eq!(m.year, Some(2020));
        assert_eq!(m.bpm, Some(124.5));
        assert_eq!(m.key.as_deref(), Some("8A"));
        assert_eq!(m.label.as_deref(), Some("L"));
        assert_eq!(m.artwork_mime_type, Some(ArtworkMimeType::Jpeg));
        assert_eq!(m.artwork.as_deref(), Some(JPEG));
    }

    #[tokio::test]
    async fn not_flac_is_none() {
        let reader = BufferReader::new(b"RIFF....".to_vec(), "flac");
        assert!(extract_from_flac(&reader).await.unwrap().is_none());
    }
}
