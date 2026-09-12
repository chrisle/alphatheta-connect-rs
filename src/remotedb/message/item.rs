//! Menu items: the rows a menu render returns.

use crate::remotedb::fields::Field;

/// Item types associated to the MenuItem message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemType {
    Path,
    Folder,
    AlbumTitle,
    Disc,
    TrackTitle,
    Genre,
    Artist,
    Playlist,
    Rating,
    Duration,
    Tempo,
    Label,
    Key,
    BitRate,
    Year,
    Comment,
    HistoryPlaylist,
    OriginalArtist,
    Remixer,
    DateAdded,
    Unknown01,
    Unknown02,
    ColorNone,
    ColorPink,
    ColorRed,
    ColorOrange,
    ColorYellow,
    ColorGreen,
    ColorAqua,
    ColorBlue,
    ColorPurple,
    MenuGenre,
    MenuArtist,
    MenuAlbum,
    MenuTrack,
    MenuPlaylist,
    MenuBPM,
    MenuRating,
    MenuYear,
    MenuRemixer,
    MenuLabel,
    MenuOriginal,
    MenuKey,
    MenuColor,
    MenuFolder,
    MenuSearch,
    MenuTime,
    MenuBit,
    MenuFilename,
    MenuHistory,
    MenuAll,
    TrackTitleAlbum,
    TrackTitleGenre,
    TrackTitleArtist,
    TrackTitleRating,
    TrackTitleTime,
    TrackTitleBPM,
    TrackTitleLabel,
    TrackTitleKey,
    TrackTitleBitRate,
    TrackTitleColor,
    TrackTitleComment,
    TrackTitleOriginalArtist,
    TrackTitleRemixer,
    TrackTitleDJPlayCount,
    MenuTrackTitleDateAdded,
    /// An item type this crate does not know.
    Other(u32),
}

impl ItemType {
    pub const fn from_u32(v: u32) -> Self {
        match v {
            0x0000 => ItemType::Path,
            0x0001 => ItemType::Folder,
            0x0002 => ItemType::AlbumTitle,
            0x0003 => ItemType::Disc,
            0x0004 => ItemType::TrackTitle,
            0x0006 => ItemType::Genre,
            0x0007 => ItemType::Artist,
            0x0008 => ItemType::Playlist,
            0x000a => ItemType::Rating,
            0x000b => ItemType::Duration,
            0x000d => ItemType::Tempo,
            0x000e => ItemType::Label,
            0x000f => ItemType::Key,
            0x0010 => ItemType::BitRate,
            0x0011 => ItemType::Year,
            0x0023 => ItemType::Comment,
            0x0024 => ItemType::HistoryPlaylist,
            0x0028 => ItemType::OriginalArtist,
            0x0029 => ItemType::Remixer,
            0x002e => ItemType::DateAdded,
            0x002f => ItemType::Unknown01,
            0x002a => ItemType::Unknown02,
            0x0013 => ItemType::ColorNone,
            0x0014 => ItemType::ColorPink,
            0x0015 => ItemType::ColorRed,
            0x0016 => ItemType::ColorOrange,
            0x0017 => ItemType::ColorYellow,
            0x0018 => ItemType::ColorGreen,
            0x0019 => ItemType::ColorAqua,
            0x001a => ItemType::ColorBlue,
            0x001b => ItemType::ColorPurple,
            0x0080 => ItemType::MenuGenre,
            0x0081 => ItemType::MenuArtist,
            0x0082 => ItemType::MenuAlbum,
            0x0083 => ItemType::MenuTrack,
            0x0084 => ItemType::MenuPlaylist,
            0x0085 => ItemType::MenuBPM,
            0x0086 => ItemType::MenuRating,
            0x0087 => ItemType::MenuYear,
            0x0088 => ItemType::MenuRemixer,
            0x0089 => ItemType::MenuLabel,
            0x008a => ItemType::MenuOriginal,
            0x008b => ItemType::MenuKey,
            0x008e => ItemType::MenuColor,
            0x0090 => ItemType::MenuFolder,
            0x0091 => ItemType::MenuSearch,
            0x0092 => ItemType::MenuTime,
            0x0093 => ItemType::MenuBit,
            0x0094 => ItemType::MenuFilename,
            0x0095 => ItemType::MenuHistory,
            0x00a0 => ItemType::MenuAll,
            0x0204 => ItemType::TrackTitleAlbum,
            0x0604 => ItemType::TrackTitleGenre,
            0x0704 => ItemType::TrackTitleArtist,
            0x0a04 => ItemType::TrackTitleRating,
            0x0b04 => ItemType::TrackTitleTime,
            0x0d04 => ItemType::TrackTitleBPM,
            0x0e04 => ItemType::TrackTitleLabel,
            0x0f04 => ItemType::TrackTitleKey,
            0x1004 => ItemType::TrackTitleBitRate,
            0x1a04 => ItemType::TrackTitleColor,
            0x2304 => ItemType::TrackTitleComment,
            0x2804 => ItemType::TrackTitleOriginalArtist,
            0x2904 => ItemType::TrackTitleRemixer,
            0x2a04 => ItemType::TrackTitleDJPlayCount,
            0x2e04 => ItemType::MenuTrackTitleDateAdded,
            other => ItemType::Other(other),
        }
    }

    /// True for the nine colour item types.
    pub const fn is_color(self) -> bool {
        matches!(
            self,
            ItemType::ColorNone
                | ItemType::ColorPink
                | ItemType::ColorRed
                | ItemType::ColorOrange
                | ItemType::ColorYellow
                | ItemType::ColorGreen
                | ItemType::ColorAqua
                | ItemType::ColorBlue
                | ItemType::ColorPurple
        )
    }
}

/// A menu item, the structured intermediate object upstream builds from the
/// 12 item arguments. Typed accessors give the per-type meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub item_type: ItemType,
    /// Parent ID, such as an artist for a track item.
    pub parent_id: u32,
    /// Main ID, such as rekordbox for a track item.
    pub main_id: u32,
    /// Label 1 main text.
    pub label1: String,
    /// Label 2 (secondary text, e.g. artist name for playlist entries).
    pub label2: String,
    /// Only holds artwork ID?
    pub artwork_id: u32,
}

impl Item {
    /// The generic `{id, name}` shape (albums, artists, genres, labels, keys,
    /// colors, folders, playlists, ...).
    pub fn id(&self) -> u32 {
        self.main_id
    }

    pub fn name(&self) -> &str {
        &self.label1
    }

    /// Track title items.
    pub fn title(&self) -> &str {
        &self.label1
    }

    /// Path items.
    pub fn path(&self) -> &str {
        &self.label1
    }

    /// Comment items.
    pub fn comment(&self) -> &str {
        &self.label1
    }

    /// BitRate items.
    pub fn bitrate(&self) -> u32 {
        self.main_id
    }

    /// Year items.
    pub fn year(&self) -> Option<u32> {
        self.label1.trim().parse().ok()
    }

    /// Rating items.
    pub fn rating(&self) -> u32 {
        self.main_id
    }

    /// Tempo items.
    pub fn bpm(&self) -> f64 {
        f64::from(self.main_id) / 100.0
    }

    /// Duration items (seconds).
    pub fn duration(&self) -> u32 {
        self.main_id
    }

    /// A `{id, name}` entity of any of the id/name kinds.
    pub fn id_name(&self) -> (u32, String) {
        (self.main_id, self.label1.clone())
    }
}

/// Translate a list of fields for an item response into a structured item.
pub fn fields_to_item(args: &[Field]) -> Item {
    let num = |i: usize| args.get(i).and_then(Field::as_number).unwrap_or(0);
    let s = |i: usize| args.get(i).and_then(Field::as_str).unwrap_or("").to_string();

    let raw_type = num(6);
    let item_type = ItemType::from_u32(raw_type);
    if let ItemType::Other(t) = item_type {
        tracing::warn!(target: "alphatheta_connect", "No item transformer registered for item type {t}");
    }

    Item { item_type, parent_id: num(0), main_id: num(1), label1: s(3), label2: s(5), artwork_id: num(8) }
}
