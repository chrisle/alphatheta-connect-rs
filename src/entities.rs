//! Entities: what is stored in the rekordbox database, plus the analysis data
//! attached to a track.
//!
//! Mirrors upstream `src/entities.ts` together with the entity and cue types
//! it re-exports from `onelibrary-connect`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{BeatGrid, WaveformHD};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artwork {
    pub id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

macro_rules! id_name_entity {
    ($($name:ident),* $(,)?) => {
        $(
            #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
            pub struct $name {
                pub id: u32,
                pub name: String,
            }
        )*
    };
}

id_name_entity!(Key, Label, Color, Genre, Album, Artist);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: u32,
    pub name: String,
    pub is_folder: bool,
    pub parent_id: Option<u32>,
}

/// A track's position in a playlist, by foreign keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistEntry {
    pub id: u32,
    pub sort_index: u32,
    pub playlist_id: u32,
    pub track_id: u32,
}

/// Represents a track with both database fields and ANLZ analysis data.
///
/// Relations are resolved (the upstream `EntityFK.WithRelations` shape). The
/// ANLZ-specific fields `beat_grid` and `waveform_hd` are populated from
/// .DAT/.EXT analysis files on the CDJ.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Track {
    pub id: u32,
    pub title: String,
    /// Seconds.
    pub duration: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u32>,
    pub tempo: f64,
    pub rating: u32,
    pub comment: String,
    pub file_path: String,
    pub file_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disc_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_depth: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mix_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoload_hotcues: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kuvo_public: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    /// The analysis file path with its extension trimmed off; `load_anlz`
    /// appends `.DAT` / `.EXT` / `.2EX`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyze_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyze_date: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_added: Option<DateTime<Utc>>,

    /// Embedded cue and loop information from the database.
    pub cue_and_loops: Option<Vec<CueAndLoop>>,

    pub artwork: Option<Artwork>,
    pub artist: Option<Artist>,
    pub original_artist: Option<Artist>,
    pub remixer: Option<Artist>,
    pub composer: Option<Artist>,
    pub album: Option<Album>,
    pub label: Option<Label>,
    pub genre: Option<Genre>,
    pub color: Option<Color>,
    pub key: Option<Key>,

    /// Embedded beat grid information (from ANLZ files).
    pub beat_grid: Option<BeatGrid>,
    /// Embedded HD Waveform information (from ANLZ files).
    pub waveform_hd: Option<WaveformHD>,
}

/// A hotcue button label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum HotcueButton {
    A = 1,
    B = 2,
    C = 3,
    D = 4,
    E = 5,
    F = 6,
    G = 7,
    H = 8,
}

impl HotcueButton {
    /// The button for a 1-8 value, `None` for anything else (including 0,
    /// which means "not a hot cue").
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => HotcueButton::A,
            2 => HotcueButton::B,
            3 => HotcueButton::C,
            4 => HotcueButton::D,
            5 => HotcueButton::E,
            6 => HotcueButton::F,
            7 => HotcueButton::G,
            8 => HotcueButton::H,
            _ => return None,
        })
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// When a custom color is not configured the cue point will be one of these
/// colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CueColor {
    None,
    Blank,
    Magenta,
    Violet,
    Fuchsia,
    LightSlateBlue,
    Blue,
    SteelBlue,
    Aqua,
    SeaGreen,
    Teal,
    Green,
    Lime,
    Olive,
    Yellow,
    Orange,
    Red,
    Pink,
    Other(u8),
}

impl CueColor {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => CueColor::None,
            0x15 => CueColor::Blank,
            0x31 => CueColor::Magenta,
            0x38 => CueColor::Violet,
            0x3c => CueColor::Fuchsia,
            0x3e => CueColor::LightSlateBlue,
            0x01 => CueColor::Blue,
            0x05 => CueColor::SteelBlue,
            0x09 => CueColor::Aqua,
            0x0e => CueColor::SeaGreen,
            0x12 => CueColor::Teal,
            0x16 => CueColor::Green,
            0x1a => CueColor::Lime,
            0x1e => CueColor::Olive,
            0x20 => CueColor::Yellow,
            0x26 => CueColor::Orange,
            0x2a => CueColor::Red,
            0x2d => CueColor::Pink,
            other => CueColor::Other(other),
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            CueColor::None => 0x00,
            CueColor::Blank => 0x15,
            CueColor::Magenta => 0x31,
            CueColor::Violet => 0x38,
            CueColor::Fuchsia => 0x3c,
            CueColor::LightSlateBlue => 0x3e,
            CueColor::Blue => 0x01,
            CueColor::SteelBlue => 0x05,
            CueColor::Aqua => 0x09,
            CueColor::SeaGreen => 0x0e,
            CueColor::Teal => 0x12,
            CueColor::Green => 0x16,
            CueColor::Lime => 0x1a,
            CueColor::Olive => 0x1e,
            CueColor::Yellow => 0x20,
            CueColor::Orange => 0x26,
            CueColor::Red => 0x2a,
            CueColor::Pink => 0x2d,
            CueColor::Other(v) => v,
        }
    }
}

/// A cue point, loop, hot cue or hot loop. On older exports the label and
/// color are `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CueAndLoop {
    /// A single cue point.
    CuePoint {
        /// Number of milliseconds from the start of the track.
        offset: f64,
        /// The comment associated to the cue point.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// The hotcue color.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<CueColor>,
    },
    /// A loop, similar to a cue point, but includes a length.
    Loop {
        offset: f64,
        /// The length in milliseconds of the loop.
        length: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<CueColor>,
    },
    /// A hotcue is like a cue point, but also includes the button it is
    /// assigned to.
    HotCue {
        offset: f64,
        /// Which hotcue button this hotcue is assigned to.
        button: HotcueButton,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<CueColor>,
    },
    /// A hot loop, the union of a hotcue and a loop.
    HotLoop {
        offset: f64,
        length: f64,
        button: HotcueButton,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<CueColor>,
    },
}

impl CueAndLoop {
    /// Milliseconds from the start of the track.
    pub fn offset(&self) -> f64 {
        match self {
            CueAndLoop::CuePoint { offset, .. }
            | CueAndLoop::Loop { offset, .. }
            | CueAndLoop::HotCue { offset, .. }
            | CueAndLoop::HotLoop { offset, .. } => *offset,
        }
    }

    /// Loop length in milliseconds, for the two loop kinds.
    pub fn length(&self) -> Option<f64> {
        match self {
            CueAndLoop::Loop { length, .. } | CueAndLoop::HotLoop { length, .. } => Some(*length),
            _ => None,
        }
    }

    /// The hot cue button, for the two hot kinds.
    pub fn button(&self) -> Option<HotcueButton> {
        match self {
            CueAndLoop::HotCue { button, .. } | CueAndLoop::HotLoop { button, .. } => Some(*button),
            _ => None,
        }
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            CueAndLoop::CuePoint { label, .. }
            | CueAndLoop::Loop { label, .. }
            | CueAndLoop::HotCue { label, .. }
            | CueAndLoop::HotLoop { label, .. } => label.as_deref(),
        }
    }

    pub fn color(&self) -> Option<CueColor> {
        match self {
            CueAndLoop::CuePoint { color, .. }
            | CueAndLoop::Loop { color, .. }
            | CueAndLoop::HotCue { color, .. }
            | CueAndLoop::HotLoop { color, .. } => *color,
        }
    }

    /// Attach a label and colour to any kind of entry.
    pub fn with_label_color(self, label: Option<String>, color: Option<CueColor>) -> Self {
        match self {
            CueAndLoop::CuePoint { offset, .. } => CueAndLoop::CuePoint { offset, label, color },
            CueAndLoop::Loop { offset, length, .. } => CueAndLoop::Loop { offset, length, label, color },
            CueAndLoop::HotCue { offset, button, .. } => CueAndLoop::HotCue { offset, button, label, color },
            CueAndLoop::HotLoop { offset, length, button, .. } => CueAndLoop::HotLoop { offset, length, button, label, color },
        }
    }
}

// ==========================================================================
// OneLibrary-specific entity types
// ==========================================================================

/// User-created tag (MyTag).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyTag {
    pub id: u32,
    pub name: String,
    pub is_folder: bool,
    pub parent_id: Option<u32>,
}

/// History session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySession {
    pub id: u32,
    pub name: String,
    pub parent_id: Option<u32>,
}

/// Hot cue bank list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotCueBankList {
    pub id: u32,
    pub name: String,
    pub parent_id: Option<u32>,
}

/// Menu item for browsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuItem {
    pub id: u32,
    pub kind: u32,
    pub name: String,
}

/// Browse category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: u32,
    pub menu_item_id: u32,
    pub name: String,
    pub kind: u32,
    pub is_visible: bool,
}

/// Sort option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortOption {
    pub id: u32,
    pub menu_item_id: u32,
    pub name: String,
    pub kind: u32,
    pub is_visible: bool,
    pub is_selected_as_sub_column: bool,
}

/// Device property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceProperty {
    pub device_name: String,
    pub db_version: String,
    pub number_of_contents: u32,
    pub created_date: String,
    pub background_color_type: u32,
}
