//! Helpers shared by the tag parsers.

pub use crate::metadata::types::ArtworkMimeType;

/// Detect image type from magic bytes.
pub fn detect_image_type(data: &[u8]) -> Option<ArtworkMimeType> {
    if data.len() < 4 {
        return None;
    }
    if data[0] == 0xff && data[1] == 0xd8 && data[2] == 0xff {
        return Some(ArtworkMimeType::Jpeg);
    }
    if data[0] == 0x89 && &data[1..4] == b"PNG" {
        return Some(ArtworkMimeType::Png);
    }
    if &data[0..3] == b"GIF" {
        return Some(ArtworkMimeType::Gif);
    }
    None
}

/// Normalize a MIME type string to a supported artwork type.
pub fn normalize_mime_type(mime_type: &str) -> ArtworkMimeType {
    let lower = mime_type.to_lowercase();
    if lower.contains("png") {
        ArtworkMimeType::Png
    } else if lower.contains("gif") {
        ArtworkMimeType::Gif
    } else {
        ArtworkMimeType::Jpeg
    }
}

/// Read a syncsafe integer (ID3v2 format). Each byte only uses 7 bits.
pub fn read_syncsafe(buffer: &[u8], offset: usize) -> u32 {
    let b = |i: usize| u32::from(buffer.get(offset + i).copied().unwrap_or(0) & 0x7f);
    (b(0) << 21) | (b(1) << 14) | (b(2) << 7) | b(3)
}

/// Text encodings of ID3v2 frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Latin1,
    Utf8,
    Utf16Le,
    Utf16Be,
}

impl TextEncoding {
    pub const fn is_utf16(self) -> bool {
        matches!(self, TextEncoding::Utf16Le | TextEncoding::Utf16Be)
    }
}

/// Get text encoding from ID3v2 encoding byte.
pub fn get_text_encoding(encoding_byte: u8) -> TextEncoding {
    match encoding_byte {
        1 => TextEncoding::Utf16Le,
        2 => TextEncoding::Utf16Be,
        3 => TextEncoding::Utf8,
        _ => TextEncoding::Latin1,
    }
}

/// Decode bytes in the given encoding.
pub fn decode_text(bytes: &[u8], encoding: TextEncoding) -> String {
    match encoding {
        TextEncoding::Latin1 => bytes.iter().map(|&b| b as char).collect(),
        TextEncoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        TextEncoding::Utf16Le => {
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        }
        TextEncoding::Utf16Be => {
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        }
    }
}

/// A null-terminated string read from a buffer.
pub struct NulTerminated {
    pub value: String,
    pub bytes_consumed: usize,
}

/// Read a null-terminated string from a buffer.
pub fn read_null_terminated_string(buffer: &[u8], offset: usize, encoding: TextEncoding) -> NulTerminated {
    let mut end = offset;

    if encoding.is_utf16() {
        while end + 1 < buffer.len() {
            if buffer[end] == 0 && buffer[end + 1] == 0 {
                break;
            }
            end += 2;
        }
    } else {
        while end < buffer.len() && buffer[end] != 0 {
            end += 1;
        }
    }

    let end = end.min(buffer.len());
    let value = decode_text(&buffer[offset.min(end)..end], encoding);
    let terminator = if encoding.is_utf16() { 2 } else { 1 };

    NulTerminated { value, bytes_consumed: end - offset.min(end) + terminator }
}

/// Parse a BPM string to a number. Handles formats like "128", "128.5",
/// "128 BPM".
pub fn parse_bpm(value: &str) -> Option<f64> {
    let start = value.find(|c: char| c.is_ascii_digit() || c == '.')?;
    let end = value[start..].find(|c: char| !(c.is_ascii_digit() || c == '.')).map(|e| start + e).unwrap_or(value.len());
    let num: f64 = value[start..end].parse().ok()?;
    if !num.is_finite() || num <= 0.0 || num > 500.0 {
        return None;
    }
    // Round to 1 decimal place
    Some((num * 10.0).round() / 10.0)
}

/// Parse a year string to a number. Handles formats like "2024",
/// "2024-01-15", "2024/01/15".
pub fn parse_year(value: &str) -> Option<u32> {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..i + 4].iter().all(u8::is_ascii_digit) {
            let year: u32 = value[i..i + 4].parse().ok()?;
            if !(1900..=2100).contains(&year) {
                return None;
            }
            return Some(year);
        }
        i += 1;
    }
    None
}

/// Clean up text by removing null characters and trimming.
pub fn clean_text(value: Option<String>) -> Option<String> {
    let cleaned: String = value?.chars().filter(|c| *c != '\0').collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

/// ID3v1 genre list.
pub const ID3V1_GENRES: &[&str] = &[
    "Blues",
    "Classic Rock",
    "Country",
    "Dance",
    "Disco",
    "Funk",
    "Grunge",
    "Hip-Hop",
    "Jazz",
    "Metal",
    "New Age",
    "Oldies",
    "Other",
    "Pop",
    "R&B",
    "Rap",
    "Reggae",
    "Rock",
    "Techno",
    "Industrial",
    "Alternative",
    "Ska",
    "Death Metal",
    "Pranks",
    "Soundtrack",
    "Euro-Techno",
    "Ambient",
    "Trip-Hop",
    "Vocal",
    "Jazz+Funk",
    "Fusion",
    "Trance",
    "Classical",
    "Instrumental",
    "Acid",
    "House",
    "Game",
    "Sound Clip",
    "Gospel",
    "Noise",
    "Alternative Rock",
    "Bass",
    "Soul",
    "Punk",
    "Space",
    "Meditative",
    "Instrumental Pop",
    "Instrumental Rock",
    "Ethnic",
    "Gothic",
    "Darkwave",
    "Techno-Industrial",
    "Electronic",
    "Pop-Folk",
    "Eurodance",
    "Dream",
    "Southern Rock",
    "Comedy",
    "Cult",
    "Gangsta",
    "Top 40",
    "Christian Rap",
    "Pop/Funk",
    "Jungle",
    "Native US",
    "Cabaret",
    "New Wave",
    "Psychedelic",
    "Rave",
    "Showtunes",
    "Trailer",
    "Lo-Fi",
    "Tribal",
    "Acid Punk",
    "Acid Jazz",
    "Polka",
    "Retro",
    "Musical",
    "Rock & Roll",
    "Hard Rock",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpm_and_year_parsing() {
        assert_eq!(parse_bpm("128"), Some(128.0));
        assert_eq!(parse_bpm("128.55"), Some(128.6));
        assert_eq!(parse_bpm("128 BPM"), Some(128.0));
        assert_eq!(parse_bpm("0"), None);
        assert_eq!(parse_bpm("nope"), None);
        assert_eq!(parse_year("2024"), Some(2024));
        assert_eq!(parse_year("2024-01-15"), Some(2024));
        assert_eq!(parse_year("x"), None);
        assert_eq!(parse_year("1800"), None);
    }

    #[test]
    fn null_terminated_strings() {
        let r = read_null_terminated_string(b"abc\0def", 0, TextEncoding::Latin1);
        assert_eq!(r.value, "abc");
        assert_eq!(r.bytes_consumed, 4);
        let r = read_null_terminated_string(&[b'h', 0, b'i', 0, 0, 0, 1], 0, TextEncoding::Utf16Le);
        assert_eq!(r.value, "hi");
        assert_eq!(r.bytes_consumed, 6);
    }

    #[test]
    fn image_detection() {
        assert_eq!(detect_image_type(&[0xff, 0xd8, 0xff, 0xe0]), Some(ArtworkMimeType::Jpeg));
        assert_eq!(detect_image_type(b"\x89PNG"), Some(ArtworkMimeType::Png));
        assert_eq!(detect_image_type(b"GIF89a"), Some(ArtworkMimeType::Gif));
        assert_eq!(detect_image_type(b"abcd"), None);
        assert_eq!(normalize_mime_type("image/PNG"), ArtworkMimeType::Png);
        assert_eq!(normalize_mime_type("image/jpg"), ArtworkMimeType::Jpeg);
    }
}
