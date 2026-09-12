//! MP4 / M4A iTunes metadata atom parsing.

use crate::metadata::parsers::utils::{clean_text, detect_image_type, ID3V1_GENRES};
use crate::metadata::types::{ArtworkMimeType, ExtractedMetadata, FileReader};
use crate::Result;

/// MP4 atom header information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtomHeader {
    pub size: u64,
    pub atom_type: [u8; 4],
    pub header_size: u64,
}

/// MP4 atom location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtomLocation {
    pub data_offset: u64,
    pub data_size: u64,
}

/// iTunes metadata atom types.
pub mod atom_types {
    pub const TITLE: &[u8; 4] = b"\xa9nam";
    pub const ARTIST: &[u8; 4] = b"\xa9ART";
    pub const ALBUM: &[u8; 4] = b"\xa9alb";
    /// Genre (text).
    pub const GENRE: &[u8; 4] = b"\xa9gen";
    /// Genre (ID3v1 numeric).
    pub const GENRE_ID: &[u8; 4] = b"gnre";
    /// Release date/year.
    pub const YEAR: &[u8; 4] = b"\xa9day";
    /// BPM (tempo).
    pub const BPM: &[u8; 4] = b"tmpo";
    /// Cover artwork.
    pub const COVER: &[u8; 4] = b"covr";
    /// Non-standard field, identified by its own name atom.
    pub const FREEFORM: &[u8; 4] = b"----";
    pub const ALBUM_ARTIST: &[u8; 4] = b"aART";
    pub const COMPOSER: &[u8; 4] = b"\xa9wrt";
    pub const COMMENT: &[u8; 4] = b"\xa9cmt";
}

/// Read an MP4 atom header.
pub async fn read_atom_header<R: FileReader>(reader: &R, offset: u64) -> Result<Option<AtomHeader>> {
    if offset + 8 > reader.size() {
        return Ok(None);
    }

    let header = reader.read(offset, 8).await?;
    if header.len() < 8 {
        return Ok(None);
    }
    let size = u64::from(u32::from_be_bytes([header[0], header[1], header[2], header[3]]));
    let atom_type = [header[4], header[5], header[6], header[7]];

    // Extended size (64-bit)
    if size == 1 {
        if offset + 16 > reader.size() {
            return Ok(None);
        }
        let ext = reader.read(offset + 8, 8).await?;
        if ext.len() < 8 {
            return Ok(None);
        }
        let mut b = [0u8; 8];
        b.copy_from_slice(&ext);
        return Ok(Some(AtomHeader { size: u64::from_be_bytes(b), atom_type, header_size: 16 }));
    }

    // Size 0 means atom extends to end of file
    if size == 0 {
        return Ok(Some(AtomHeader { size: reader.size() - offset, atom_type, header_size: 8 }));
    }

    Ok(Some(AtomHeader { size, atom_type, header_size: 8 }))
}

/// Find an atom within a range.
pub async fn find_atom<R: FileReader>(reader: &R, start: u64, end: u64, target: &[u8; 4]) -> Result<Option<AtomLocation>> {
    let mut offset = start;
    while offset < end {
        let Some(header) = read_atom_header(reader, offset).await? else {
            break;
        };
        if header.size == 0 {
            break;
        }
        if &header.atom_type == target {
            return Ok(Some(AtomLocation {
                data_offset: offset + header.header_size,
                data_size: header.size.saturating_sub(header.header_size),
            }));
        }
        offset += header.size;
    }
    Ok(None)
}

/// Find the moov atom (contains all metadata).
pub async fn find_moov_atom<R: FileReader>(reader: &R) -> Result<Option<AtomLocation>> {
    find_atom(reader, 0, reader.size(), b"moov").await
}

/// Navigate to the ilst (iTunes metadata list) atom: moov -> udta -> meta -> ilst.
pub async fn find_ilst_atom<R: FileReader>(reader: &R, moov: AtomLocation) -> Result<Option<AtomLocation>> {
    let Some(udta) = find_atom(reader, moov.data_offset, moov.data_offset + moov.data_size, b"udta").await? else {
        return Ok(None);
    };
    let Some(meta) = find_atom(reader, udta.data_offset, udta.data_offset + udta.data_size, b"meta").await? else {
        return Ok(None);
    };
    // meta atom has 4 bytes of version/flags before child atoms
    let meta_start = meta.data_offset + 4;
    let meta_end = meta.data_offset + meta.data_size;
    find_atom(reader, meta_start, meta_end, b"ilst").await
}

/// Read a text data atom value.
async fn read_text_data_atom<R: FileReader>(reader: &R, offset: u64, size: u64) -> Result<Option<String>> {
    let Some(data) = find_atom(reader, offset, offset + size, b"data").await? else {
        return Ok(None);
    };
    if data.data_size < 8 {
        return Ok(None);
    }
    // data atom: 4 bytes type + 4 bytes locale + actual data
    let content = reader.read(data.data_offset + 8, data.data_size - 8).await?;
    Ok(clean_text(Some(String::from_utf8_lossy(&content).into_owned())))
}

/// Read a numeric data atom value (for BPM, genre ID).
async fn read_numeric_data_atom<R: FileReader>(reader: &R, offset: u64, size: u64) -> Result<Option<u16>> {
    let Some(data) = find_atom(reader, offset, offset + size, b"data").await? else {
        return Ok(None);
    };
    if data.data_size < 10 {
        return Ok(None);
    }
    let content = reader.read(data.data_offset + 8, data.data_size - 8).await?;
    // BPM is typically stored as 16-bit big-endian
    if content.len() >= 2 {
        return Ok(Some(u16::from_be_bytes([content[0], content[1]])));
    }
    Ok(None)
}

/// Field names taggers use inside a freeform atom for the record label.
const FREEFORM_LABEL_NAMES: &[&str] = &["LABEL", "PUBLISHER", "ORGANIZATION"];

/// Read the field name out of an iTunes freeform ('----') atom.
///
/// iTunes never standardised an atom for the record label, so taggers store
/// it as a freeform atom instead: a 'mean' atom holding the namespace
/// (normally com.apple.iTunes), a 'name' atom holding the field name, and the
/// usual 'data' atom holding the value. Every freeform atom has the same
/// type, so the name is the only thing that says which field this one is.
async fn read_freeform_name<R: FileReader>(reader: &R, offset: u64, size: u64) -> Result<Option<String>> {
    let Some(name) = find_atom(reader, offset, offset + size, b"name").await? else {
        return Ok(None);
    };
    if name.data_size < 4 {
        return Ok(None);
    }
    // name atom: 4 bytes version/flags + the field name
    let content = reader.read(name.data_offset + 4, name.data_size - 4).await?;
    Ok(clean_text(Some(String::from_utf8_lossy(&content).into_owned())))
}

/// Read cover artwork from the covr atom.
pub async fn read_cover_artwork<R: FileReader>(reader: &R, offset: u64, size: u64) -> Result<Option<(Vec<u8>, ArtworkMimeType)>> {
    let Some(data) = find_atom(reader, offset, offset + size, b"data").await? else {
        return Ok(None);
    };
    if data.data_size < 8 {
        return Ok(None);
    }
    let image = reader.read(data.data_offset + 8, data.data_size - 8).await?;
    if image.is_empty() {
        return Ok(None);
    }
    let mime = detect_image_type(&image).unwrap_or(ArtworkMimeType::Jpeg);
    Ok(Some((image, mime)))
}

/// True when the file starts with an `ftyp` atom.
pub async fn is_mp4<R: FileReader>(reader: &R) -> Result<bool> {
    let header = reader.read(0, 8).await?;
    Ok(header.len() >= 8 && &header[4..8] == b"ftyp")
}

/// Extract metadata from an MP4/M4A file.
pub async fn extract_from_mp4<R: FileReader>(reader: &R) -> Result<Option<ExtractedMetadata>> {
    // Verify this is an MP4 file by checking for ftyp atom
    if !is_mp4(reader).await? {
        return Ok(None);
    }

    let Some(moov) = find_moov_atom(reader).await? else {
        return Ok(None);
    };
    let Some(ilst) = find_ilst_atom(reader, moov).await? else {
        return Ok(None);
    };

    let mut metadata = ExtractedMetadata::default();

    // Iterate through ilst child atoms
    let mut offset = ilst.data_offset;
    let end = ilst.data_offset + ilst.data_size;

    while offset < end {
        let Some(header) = read_atom_header(reader, offset).await? else {
            break;
        };
        if header.size == 0 {
            break;
        }

        let data_offset = offset + header.header_size;
        let data_size = header.size.saturating_sub(header.header_size);

        match &header.atom_type {
            t if t == atom_types::TITLE => {
                if metadata.title.is_none() {
                    metadata.title = read_text_data_atom(reader, data_offset, data_size).await?;
                }
            }
            t if t == atom_types::ARTIST => {
                if metadata.artist.is_none() {
                    metadata.artist = read_text_data_atom(reader, data_offset, data_size).await?;
                }
            }
            t if t == atom_types::ALBUM => {
                if metadata.album.is_none() {
                    metadata.album = read_text_data_atom(reader, data_offset, data_size).await?;
                }
            }
            t if t == atom_types::GENRE => {
                let genre = read_text_data_atom(reader, data_offset, data_size).await?;
                if metadata.genre.is_none() {
                    metadata.genre = genre;
                }
            }
            t if t == atom_types::GENRE_ID => {
                let genre_id = read_numeric_data_atom(reader, data_offset, data_size).await?;
                if let Some(id) = genre_id {
                    if metadata.genre.is_none() {
                        // MP4 genre IDs are 1-based
                        metadata.genre = usize::from(id).checked_sub(1).and_then(|i| ID3V1_GENRES.get(i)).map(|g| g.to_string());
                    }
                }
            }
            t if t == atom_types::YEAR => {
                let year = read_text_data_atom(reader, data_offset, data_size).await?;
                if let Some(y) = year {
                    if metadata.year.is_none() {
                        metadata.year = crate::metadata::parsers::utils::parse_year(&y).or_else(|| four_digits(&y));
                    }
                }
            }
            t if t == atom_types::BPM => {
                let bpm = read_numeric_data_atom(reader, data_offset, data_size).await?;
                if let Some(b) = bpm {
                    if b > 0 && b < 500 && metadata.bpm.is_none() {
                        metadata.bpm = Some(f64::from(b));
                    }
                }
            }
            t if t == atom_types::FREEFORM => {
                if metadata.label.is_none() {
                    let name = read_freeform_name(reader, data_offset, data_size).await?;
                    if let Some(name) = name {
                        if FREEFORM_LABEL_NAMES.contains(&name.to_uppercase().as_str()) {
                            metadata.label = read_text_data_atom(reader, data_offset, data_size).await?;
                        }
                    }
                }
            }
            t if t == atom_types::COVER && metadata.artwork.is_none() => {
                if let Some((data, mime)) = read_cover_artwork(reader, data_offset, data_size).await? {
                    metadata.artwork = Some(data);
                    metadata.artwork_mime_type = Some(mime);
                }
            }
            _ => {}
        }

        offset += header.size;
    }

    if metadata.is_empty() {
        return Ok(None);
    }

    Ok(Some(metadata))
}

/// The first run of four digits in a string, as upstream's `/\d{4}/` match
/// for the year atom (which is not range-checked).
fn four_digits(s: &str) -> Option<u32> {
    let bytes = s.as_bytes();
    (0..bytes.len().saturating_sub(3))
        .find(|&i| bytes[i..i + 4].iter().all(u8::is_ascii_digit))
        .and_then(|i| s[i..i + 4].parse().ok())
}

#[cfg(test)]
pub(crate) mod fixtures {
    pub fn atom(atom_type: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut a = Vec::new();
        a.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        a.extend_from_slice(atom_type);
        a.extend_from_slice(body);
        a
    }

    pub fn data_atom(type_flag: u32, payload: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&type_flag.to_be_bytes());
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(payload);
        atom(b"data", &body)
    }

    pub fn text_item(atom_type: &[u8; 4], text: &str) -> Vec<u8> {
        atom(atom_type, &data_atom(1, text.as_bytes()))
    }

    pub fn number_item(atom_type: &[u8; 4], value: u16) -> Vec<u8> {
        atom(atom_type, &data_atom(21, &value.to_be_bytes()))
    }

    pub fn freeform_item(name: &str, text: &str) -> Vec<u8> {
        let mut mean = 0u32.to_be_bytes().to_vec();
        mean.extend_from_slice(b"com.apple.iTunes");
        let mut nm = 0u32.to_be_bytes().to_vec();
        nm.extend_from_slice(name.as_bytes());
        let body = [atom(b"mean", &mean), atom(b"name", &nm), data_atom(1, text.as_bytes())].concat();
        atom(b"----", &body)
    }

    pub fn file(items: &[Vec<u8>]) -> Vec<u8> {
        let ilst = atom(b"ilst", &items.concat());
        let mut meta_body = 0u32.to_be_bytes().to_vec();
        meta_body.extend_from_slice(&ilst);
        let meta = atom(b"meta", &meta_body);
        let udta = atom(b"udta", &meta);
        let moov = atom(b"moov", &udta);
        let ftyp = atom(b"ftyp", b"M4A \0\0\0\0");
        [ftyp, moov, atom(b"mdat", &[0u8; 16])].concat()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::metadata::parsers::id3::fixtures::PNG;
    use crate::metadata::reader::BufferReader;

    #[tokio::test]
    async fn extracts_itunes_atoms() {
        let f = file(&[
            text_item(atom_types::TITLE, "Title"),
            text_item(atom_types::ARTIST, "Artist"),
            text_item(atom_types::ALBUM, "Album"),
            number_item(atom_types::GENRE_ID, 18),
            text_item(atom_types::YEAR, "2019-03-04"),
            number_item(atom_types::BPM, 126),
            freeform_item("LABEL", "Label"),
            atom(atom_types::COVER, &data_atom(14, PNG)),
        ]);
        let reader = BufferReader::new(f, "m4a");
        let m = extract_from_mp4(&reader).await.unwrap().unwrap();
        assert_eq!(m.title.as_deref(), Some("Title"));
        assert_eq!(m.artist.as_deref(), Some("Artist"));
        assert_eq!(m.album.as_deref(), Some("Album"));
        assert_eq!(m.genre.as_deref(), Some("Rock"));
        assert_eq!(m.year, Some(2019));
        assert_eq!(m.bpm, Some(126.0));
        assert_eq!(m.label.as_deref(), Some("Label"));
        assert_eq!(m.artwork_mime_type, Some(ArtworkMimeType::Png));
        assert_eq!(m.artwork.as_deref(), Some(PNG));
    }

    #[tokio::test]
    async fn not_mp4_is_none() {
        let reader = BufferReader::new(b"ID3\x03\0\0\0\0\0\0".to_vec(), "m4a");
        assert!(extract_from_mp4(&reader).await.unwrap().is_none());
    }
}
