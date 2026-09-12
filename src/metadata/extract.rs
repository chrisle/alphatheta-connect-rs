//! Dispatch to the parser for a file's extension.

use crate::logger::{noop_logger, SharedLogger};
use crate::metadata::parsers::{extract_from_aiff, extract_from_flac, extract_from_mp3, extract_from_mp4};
use crate::metadata::types::{ExtractedMetadata, FileReader};
use crate::Result;

/// The parser families this crate knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataParser {
    Mp3,
    Mp4,
    Flac,
    Aiff,
}

/// Get the appropriate parser for a file extension.
pub fn get_parser_for_extension(extension: &str) -> Option<MetadataParser> {
    let normalized = extension.to_lowercase();
    let normalized = normalized.strip_prefix('.').unwrap_or(&normalized);
    Some(match normalized {
        "mp3" => MetadataParser::Mp3,
        "m4a" | "mp4" | "m4p" | "m4b" | "aac" => MetadataParser::Mp4,
        "flac" => MetadataParser::Flac,
        "aiff" | "aif" | "aifc" => MetadataParser::Aiff,
        _ => return None,
    })
}

/// Get the list of supported file extensions.
pub fn get_supported_extensions() -> &'static [&'static str] {
    &["mp3", "m4a", "mp4", "m4p", "m4b", "aac", "flac", "aiff", "aif", "aifc"]
}

/// Check if a file extension is supported.
pub fn is_extension_supported(extension: &str) -> bool {
    get_parser_for_extension(extension).is_some()
}

/// Run a parser against a reader.
pub async fn run_parser<R: FileReader>(parser: MetadataParser, reader: &R) -> Result<Option<ExtractedMetadata>> {
    match parser {
        MetadataParser::Mp3 => extract_from_mp3(reader).await,
        MetadataParser::Mp4 => extract_from_mp4(reader).await,
        MetadataParser::Flac => extract_from_flac(reader).await,
        MetadataParser::Aiff => extract_from_aiff(reader).await,
    }
}

/// Extract metadata from an audio file using the appropriate parser.
///
/// Returns `None` if extraction fails or the format is unsupported; failures
/// are logged, never raised.
pub async fn extract_metadata<R: FileReader>(reader: &R, logger: Option<SharedLogger>) -> Option<ExtractedMetadata> {
    let logger = logger.unwrap_or_else(noop_logger);
    let Some(parser) = get_parser_for_extension(reader.extension()) else {
        logger.debug(&format!("No parser found for extension: {}", reader.extension()));
        return None;
    };

    logger.debug(&format!("Extracting metadata from {} file ({} bytes)", reader.extension(), reader.size()));
    match run_parser(parser, reader).await {
        Ok(Some(result)) => {
            logger.debug(&format!(
                "Extracted metadata: title={}, artist={}",
                result.title.as_deref().unwrap_or("(none)"),
                result.artist.as_deref().unwrap_or("(none)")
            ));
            Some(result)
        }
        Ok(None) => {
            logger.debug(&format!("Parser returned null for {} file", reader.extension()));
            None
        }
        Err(e) => {
            logger.warn(&format!("Failed to extract metadata from {} file: {e}", reader.extension()));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_dispatch() {
        assert_eq!(get_parser_for_extension("MP3"), Some(MetadataParser::Mp3));
        assert_eq!(get_parser_for_extension(".m4a"), Some(MetadataParser::Mp4));
        assert_eq!(get_parser_for_extension("aifc"), Some(MetadataParser::Aiff));
        assert!(!is_extension_supported("wav"));
        assert!(get_supported_extensions().contains(&"flac"));
    }
}
