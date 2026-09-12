//! AIFF metadata: AIFF files can contain ID3v2 tags in an 'ID3 ' chunk.

use crate::metadata::parsers::id3::extract_from_mp3;
use crate::metadata::reader::BufferReader;
use crate::metadata::types::{ExtractedMetadata, FileReader};
use crate::Result;

/// Locate the ID3 chunk of an AIFF file: (offset, size) of its body.
pub async fn find_aiff_id3_chunk<R: FileReader>(reader: &R) -> Result<Option<(u64, u64)>> {
    // Read AIFF header (12 bytes minimum)
    let header = reader.read(0, 12).await?;
    if header.len() < 12 || &header[0..4] != b"FORM" {
        return Ok(None);
    }

    let form_type = &header[8..12];
    if form_type != b"AIFF" && form_type != b"AIFC" {
        return Ok(None);
    }

    let form_size = u64::from(u32::from_be_bytes([header[4], header[5], header[6], header[7]]));
    let file_end = (8 + form_size).min(reader.size());

    let mut offset = 12u64;

    // Iterate through chunks looking for the ID3 tag
    while offset + 8 < file_end {
        let chunk_header = reader.read(offset, 8).await?;
        if chunk_header.len() < 8 {
            break;
        }
        let chunk_id = &chunk_header[0..4];
        let chunk_size = u64::from(u32::from_be_bytes([chunk_header[4], chunk_header[5], chunk_header[6], chunk_header[7]]));

        if chunk_size == 0 || offset + 8 + chunk_size > file_end {
            break;
        }

        // Check for ID3 chunk (can be 'ID3 ' or 'id3 ')
        if chunk_id == b"ID3 " || chunk_id == b"id3 " {
            return Ok(Some((offset + 8, chunk_size)));
        }

        // AIFF chunks are padded to even byte boundaries
        offset += 8 + chunk_size + (chunk_size % 2);
    }

    Ok(None)
}

/// Extract metadata from an AIFF file by delegating its ID3 chunk to the ID3 parser.
pub async fn extract_from_aiff<R: FileReader>(reader: &R) -> Result<Option<ExtractedMetadata>> {
    let Some((offset, size)) = find_aiff_id3_chunk(reader).await? else {
        return Ok(None);
    };
    let id3_data = reader.read(offset, size).await?;
    let id3_reader = BufferReader::new(id3_data, "mp3");
    extract_from_mp3(&id3_reader).await
}

#[cfg(test)]
pub(crate) mod fixtures {
    pub fn file(id3: &[u8]) -> Vec<u8> {
        let mut comm = b"COMM".to_vec();
        comm.extend_from_slice(&18u32.to_be_bytes());
        comm.extend_from_slice(&[0u8; 18]);
        let mut id3_chunk = b"ID3 ".to_vec();
        id3_chunk.extend_from_slice(&(id3.len() as u32).to_be_bytes());
        id3_chunk.extend_from_slice(id3);
        if id3.len() % 2 == 1 {
            id3_chunk.push(0);
        }
        let body = [b"AIFF".to_vec(), comm, id3_chunk].concat();
        let mut f = b"FORM".to_vec();
        f.extend_from_slice(&(body.len() as u32).to_be_bytes());
        f.extend_from_slice(&body);
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::parsers::id3::fixtures::{tag, text_frame};

    #[tokio::test]
    async fn delegates_to_id3() {
        let f = fixtures::file(&tag(&[text_frame("TIT2", "Aiff Title")]));
        let reader = BufferReader::new(f, "aiff");
        let m = extract_from_aiff(&reader).await.unwrap().unwrap();
        assert_eq!(m.title.as_deref(), Some("Aiff Title"));
        let reader = BufferReader::new(b"FORM\0\0\0\x04WAVE".to_vec(), "aiff");
        assert!(extract_from_aiff(&reader).await.unwrap().is_none());
    }
}
