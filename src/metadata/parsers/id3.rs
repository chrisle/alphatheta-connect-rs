//! ID3v2 tag parsing for MP3 (and AIFF-embedded) metadata.

use crate::metadata::parsers::utils::{
    clean_text, decode_text, detect_image_type, get_text_encoding, parse_bpm, parse_year, read_null_terminated_string,
    read_syncsafe, ArtworkMimeType, TextEncoding, ID3V1_GENRES,
};
use crate::metadata::types::{ArtworkResult, ExtractedMetadata, FileReader, PictureType};
use crate::Result;

/// ID3v2 frame IDs for metadata extraction.
mod frame_ids {
    pub const TITLE: &[&str] = &["TIT2", "TT2"];
    pub const ARTIST: &[&str] = &["TPE1", "TP1"];
    pub const ALBUM: &[&str] = &["TALB", "TAL"];
    pub const GENRE: &[&str] = &["TCON", "TCO"];
    /// Publisher (the record label).
    pub const LABEL: &[&str] = &["TPUB", "TPB"];
    /// TYER for v2.3, TDRC for v2.4.
    pub const YEAR: &[&str] = &["TYER", "TYE", "TDRC"];
    pub const BPM: &[&str] = &["TBPM", "TBP"];
    pub const KEY: &[&str] = &["TKEY", "TKE"];
    pub const PICTURE: &[&str] = &["APIC", "PIC"];
}

/// The tag header and frame layout of an ID3v2 tag, so the frame walk can
/// be shared between the metadata and artwork extractors.
pub struct Id3Frame<'a> {
    pub id: String,
    pub data: &'a [u8],
}

/// Read the ID3v2 tag from the start of the file and split it into frames.
/// Returns `None` when there is no ID3v2 tag.
pub async fn read_id3_frames<R: FileReader>(reader: &R) -> Result<Option<(u8, Vec<u8>)>> {
    // Read ID3v2 header (10 bytes)
    let header = reader.read(0, 10).await?;
    if header.len() < 10 || &header[0..3] != b"ID3" {
        return Ok(None);
    }

    let major_version = header[3];
    let flags = header[5];
    let tag_size = read_syncsafe(&header, 6) as u64;

    // Handle extended header if present
    let mut extended_header_size = 0u64;
    if flags & 0x40 != 0 {
        let ext = reader.read(10, 4).await?;
        extended_header_size = if major_version == 4 {
            u64::from(read_syncsafe(&ext, 0))
        } else {
            u64::from(u32::from_be_bytes([ext[0], ext[1], ext[2], ext[3]]))
        };
    }

    // Read all tag data
    let tag_data = reader.read(10 + extended_header_size, tag_size.saturating_sub(extended_header_size)).await?;
    Ok(Some((major_version, tag_data)))
}

/// Walk the frames of a tag body.
pub fn frames(major_version: u8, tag_data: &[u8]) -> Vec<Id3Frame<'_>> {
    let mut out = Vec::new();
    let frame_header_size = if major_version >= 3 { 10 } else { 6 };
    let mut offset = 0usize;

    while offset + frame_header_size < tag_data.len() {
        // End of frames (padding starts with zero)
        if tag_data[offset] == 0 {
            break;
        }

        let (frame_id, frame_size) = if major_version >= 3 {
            // ID3v2.3 or ID3v2.4
            let id = String::from_utf8_lossy(&tag_data[offset..offset + 4]).into_owned();
            let size = if major_version == 4 {
                read_syncsafe(tag_data, offset + 4) as usize
            } else {
                u32::from_be_bytes([tag_data[offset + 4], tag_data[offset + 5], tag_data[offset + 6], tag_data[offset + 7]])
                    as usize
            };
            (id, size)
        } else {
            // ID3v2.2
            let id = String::from_utf8_lossy(&tag_data[offset..offset + 3]).into_owned();
            let size = (usize::from(tag_data[offset + 3]) << 16)
                | (usize::from(tag_data[offset + 4]) << 8)
                | usize::from(tag_data[offset + 5]);
            (id, size)
        };

        if frame_size == 0 || frame_size > tag_data.len() - offset {
            break;
        }

        let start = offset + frame_header_size;
        let end = (start + frame_size).min(tag_data.len());
        // Normalize v2.2 frame IDs to v2.3/4 equivalents
        let id = if major_version >= 3 { frame_id } else { normalize_v22_frame_id(&frame_id) };
        out.push(Id3Frame { id, data: &tag_data[start..end] });

        offset += frame_header_size + frame_size;
    }

    out
}

/// Parse an APIC (attached picture) frame.
pub fn parse_apic_frame(data: &[u8]) -> Option<ArtworkResult> {
    if data.len() < 4 {
        return None;
    }

    let mut offset = 0usize;
    let encoding_byte = data[offset];
    offset += 1;
    let mut encoding = get_text_encoding(encoding_byte);

    // Check for BOM in UTF-16
    if encoding_byte == 1 && data.len() > offset + 2 {
        let bom = u16::from_be_bytes([data[offset], data[offset + 1]]);
        if bom == 0xfeff {
            encoding = TextEncoding::Utf16Be;
        } else if bom == 0xfffe {
            encoding = TextEncoding::Utf16Le;
        }
    }

    let mime = read_null_terminated_string(data, offset, TextEncoding::Latin1);
    let mime_type = mime.value;
    offset += mime.bytes_consumed;

    if offset >= data.len() {
        return None;
    }

    let picture_type = PictureType::from_u32(u32::from(data[offset]));
    offset += 1;
    if offset >= data.len() {
        return None;
    }

    let desc = read_null_terminated_string(data, offset, encoding);
    offset += desc.bytes_consumed;

    if offset >= data.len() {
        return None;
    }

    let image_data = &data[offset..];
    if image_data.is_empty() {
        return None;
    }

    let final_mime = detect_image_type(image_data).unwrap_or(if mime_type.contains("png") {
        ArtworkMimeType::Png
    } else {
        ArtworkMimeType::Jpeg
    });

    Some(ArtworkResult { data: image_data.to_vec(), mime_type: final_mime, picture_type, width: None, height: None })
}

/// Parse a text frame (TIT2, TPE1, etc.).
pub fn parse_text_frame(data: &[u8]) -> Option<String> {
    if data.len() < 2 {
        return None;
    }

    let encoding_byte = data[0];
    let mut encoding = get_text_encoding(encoding_byte);
    let mut offset = 1usize;

    // Check for BOM in UTF-16
    if encoding_byte == 1 && data.len() > offset + 2 {
        let bom = u16::from_be_bytes([data[offset], data[offset + 1]]);
        if bom == 0xfeff {
            encoding = TextEncoding::Utf16Be;
            offset += 2;
        } else if bom == 0xfffe {
            encoding = TextEncoding::Utf16Le;
            offset += 2;
        }
    }

    // Read until null terminator or end of data
    let text_data = &data[offset..];

    if encoding.is_utf16() {
        // Find null terminator (two zeros)
        let mut end = text_data.len();
        let mut i = 0;
        while i + 1 < text_data.len() {
            if text_data[i] == 0 && text_data[i + 1] == 0 {
                end = i;
                break;
            }
            i += 2;
        }
        return clean_text(Some(decode_text(&text_data[..end], encoding)));
    }

    // Latin1 or UTF-8
    let end = text_data.iter().position(|b| *b == 0).unwrap_or(text_data.len());
    clean_text(Some(decode_text(&text_data[..end], encoding)))
}

/// Check if a frame ID matches any of the target IDs.
fn matches_frame_id(frame_id: &str, targets: &[&str]) -> bool {
    targets.iter().any(|t| frame_id == *t || frame_id.starts_with(t))
}

/// Extract metadata from an MP3 file with ID3v2 tags.
pub async fn extract_from_mp3<R: FileReader>(reader: &R) -> Result<Option<ExtractedMetadata>> {
    let Some((major_version, tag_data)) = read_id3_frames(reader).await? else {
        return Ok(None);
    };

    let mut metadata = ExtractedMetadata::default();
    let mut front_cover: Option<ArtworkResult> = None;
    let mut any_artwork: Option<ArtworkResult> = None;

    for frame in frames(major_version, &tag_data) {
        let id = frame.id.as_str();
        let data = frame.data;

        if matches_frame_id(id, frame_ids::TITLE) {
            metadata.title = metadata.title.take().or_else(|| parse_text_frame(data));
        } else if matches_frame_id(id, frame_ids::ARTIST) {
            metadata.artist = metadata.artist.take().or_else(|| parse_text_frame(data));
        } else if matches_frame_id(id, frame_ids::ALBUM) {
            metadata.album = metadata.album.take().or_else(|| parse_text_frame(data));
        } else if matches_frame_id(id, frame_ids::GENRE) {
            metadata.genre = metadata.genre.take().or_else(|| parse_genre(parse_text_frame(data)));
        } else if matches_frame_id(id, frame_ids::LABEL) {
            metadata.label = metadata.label.take().or_else(|| parse_text_frame(data));
        } else if matches_frame_id(id, frame_ids::YEAR) {
            metadata.year = metadata.year.or_else(|| parse_year(&parse_text_frame(data).unwrap_or_default()));
        } else if matches_frame_id(id, frame_ids::BPM) {
            metadata.bpm = metadata.bpm.or_else(|| parse_bpm(&parse_text_frame(data).unwrap_or_default()));
        } else if matches_frame_id(id, frame_ids::KEY) {
            metadata.key = metadata.key.take().or_else(|| parse_text_frame(data));
        } else if matches_frame_id(id, frame_ids::PICTURE) {
            if let Some(artwork) = parse_apic_frame(data) {
                if artwork.picture_type == PictureType::FrontCover {
                    front_cover = Some(artwork);
                } else if any_artwork.is_none() {
                    any_artwork = Some(artwork);
                }
            }
        }
    }

    // Use front cover if available, otherwise any artwork
    if let Some(artwork) = front_cover.or(any_artwork) {
        metadata.artwork = Some(artwork.data);
        metadata.artwork_mime_type = Some(artwork.mime_type);
    }

    // Return None if no metadata was found
    if metadata.is_empty() {
        return Ok(None);
    }

    Ok(Some(metadata))
}

/// Normalize ID3v2.2 frame IDs to v2.3/4 equivalents.
pub fn normalize_v22_frame_id(frame_id: &str) -> String {
    match frame_id {
        "TT2" => "TIT2",
        "TP1" => "TPE1",
        "TAL" => "TALB",
        "TCO" => "TCON",
        "TPB" => "TPUB",
        "TYE" => "TYER",
        "TBP" => "TBPM",
        "TKE" => "TKEY",
        "PIC" => "APIC",
        other => other,
    }
    .to_string()
}

/// Parse genre string, handling ID3v1 numeric references. Format: "(17)Rock"
/// or "(17)" or "Rock".
fn parse_genre(genre: Option<String>) -> Option<String> {
    let genre = genre?;

    // Check for ID3v1 numeric reference
    if let Some(rest) = genre.strip_prefix('(') {
        if let Some(close) = rest.find(')') {
            let (num, text) = rest.split_at(close);
            if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
                let text_genre = text[1..].trim();
                // If there's text after the number, use that
                if !text_genre.is_empty() {
                    return Some(text_genre.to_string());
                }
                // Otherwise look up the ID3v1 genre
                let id: usize = num.parse().ok()?;
                return Some(ID3V1_GENRES.get(id).map(|g| g.to_string()).unwrap_or(genre));
            }
        }
    }

    Some(genre)
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! ID3v2.3 tag builders for tests.

    pub fn text_frame(id: &str, text: &str) -> Vec<u8> {
        let mut body = vec![0u8]; // latin1
        body.extend_from_slice(text.as_bytes());
        frame(id, &body)
    }

    pub fn frame(id: &str, body: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(id.as_bytes());
        f.extend_from_slice(&(body.len() as u32).to_be_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(body);
        f
    }

    pub fn apic(mime: &str, picture_type: u8, image: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8];
        body.extend_from_slice(mime.as_bytes());
        body.push(0);
        body.push(picture_type);
        body.push(0); // empty description
        body.extend_from_slice(image);
        frame("APIC", &body)
    }

    pub fn tag(frames: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = frames.concat();
        let size = body.len() as u32;
        let mut t = Vec::new();
        t.extend_from_slice(b"ID3");
        t.push(3);
        t.push(0);
        t.push(0);
        t.push(((size >> 21) & 0x7f) as u8);
        t.push(((size >> 14) & 0x7f) as u8);
        t.push(((size >> 7) & 0x7f) as u8);
        t.push((size & 0x7f) as u8);
        t.extend_from_slice(&body);
        // some audio bytes
        t.extend_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
        t
    }

    pub const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
    pub const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::metadata::reader::BufferReader;

    #[tokio::test]
    async fn extracts_text_frames_and_front_cover() {
        let file = tag(&[
            text_frame("TIT2", "Title"),
            text_frame("TPE1", "Artist"),
            text_frame("TALB", "Album"),
            text_frame("TCON", "(17)"),
            text_frame("TPUB", "Label"),
            text_frame("TYER", "2021"),
            text_frame("TBPM", "128"),
            text_frame("TKEY", "Am"),
            apic("image/jpeg", 0, PNG),
            apic("image/jpeg", 3, JPEG),
        ]);
        let reader = BufferReader::new(file, "mp3");
        let m = extract_from_mp3(&reader).await.unwrap().unwrap();
        assert_eq!(m.title.as_deref(), Some("Title"));
        assert_eq!(m.artist.as_deref(), Some("Artist"));
        assert_eq!(m.album.as_deref(), Some("Album"));
        assert_eq!(m.genre.as_deref(), Some("Rock"));
        assert_eq!(m.label.as_deref(), Some("Label"));
        assert_eq!(m.year, Some(2021));
        assert_eq!(m.bpm, Some(128.0));
        assert_eq!(m.key.as_deref(), Some("Am"));
        assert_eq!(m.artwork.as_deref(), Some(JPEG));
        assert_eq!(m.artwork_mime_type, Some(ArtworkMimeType::Jpeg));
    }

    #[tokio::test]
    async fn no_tag_is_none() {
        let reader = BufferReader::new(vec![0xff, 0xfb, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "mp3");
        assert!(extract_from_mp3(&reader).await.unwrap().is_none());
    }

    #[test]
    fn utf16_text_frames() {
        let mut body = vec![1u8, 0xff, 0xfe];
        for u in "Héllo".encode_utf16() {
            body.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_text_frame(&body).as_deref(), Some("Héllo"));
        assert_eq!(parse_genre(Some("(3)Dance Pop".into())).as_deref(), Some("Dance Pop"));
        assert_eq!(parse_genre(Some("House".into())).as_deref(), Some("House"));
    }
}
