//! Constants for the OneLibrary (exportLibrary.db) SQLite schema.
//!
//! Column names use camelCase in the database (e.g., bpmx100, titleForSearch).

/// Cue point types based on the 'kind' field in the cue table.
/// Note: These values may vary. Verify with actual data.
pub mod cue_kind {
    pub const MEMORY_CUE: i64 = 0;
    pub const HOT_CUE: i64 = 1;
}

/// Playlist attribute types based on the 'attribute' field.
pub mod playlist_attribute {
    pub const PLAYLIST: i64 = 0;
    pub const FOLDER: i64 = 1;
}

/// MyTag attribute types based on the 'attribute' field.
pub mod my_tag_attribute {
    pub const TAG: i64 = 0;
    pub const FOLDER: i64 = 1;
}

/// Menu item kinds for browsing categories. These match the 'kind' field in
/// the menuItem table.
pub mod menu_item_kind {
    pub const GENRE: i64 = 128;
    pub const ARTIST: i64 = 129;
    pub const ALBUM: i64 = 130;
    pub const TRACK: i64 = 131;
    pub const PLAYLIST: i64 = 132;
    pub const BPM: i64 = 133;
    pub const RATING: i64 = 134;
    pub const YEAR: i64 = 135;
    pub const REMIXER: i64 = 136;
    pub const LABEL: i64 = 137;
    pub const ORIGINAL_ARTIST: i64 = 138;
    pub const KEY: i64 = 139;
    pub const DATE_ADDED: i64 = 140;
    pub const CUE: i64 = 141;
    pub const COLOR: i64 = 142;
    pub const FOLDER: i64 = 144;
    pub const SEARCH: i64 = 145;
    pub const TIME: i64 = 146;
    pub const BITRATE: i64 = 147;
    pub const FILE_NAME: i64 = 148;
    pub const HISTORY: i64 = 149;
    pub const COMMENTS: i64 = 150;
    pub const DJ_PLAY_COUNT: i64 = 151;
    pub const HOT_CUE_BANK: i64 = 152;
    pub const DEFAULT: i64 = 161;
    pub const ALPHABET: i64 = 162;
    pub const MATCHING: i64 = 170;
}
