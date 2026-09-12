//! Public data types shared across the crate.
//!
//! Mirrors upstream `src/types.ts`. CDJ status types live in
//! [`crate::status::types`] and are re-exported here as [`CDJStatus`].

use std::net::Ipv4Addr;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use crate::status::types as CDJStatus;

pub use crate::db::get_track_analysis::TrackAnalysis;
pub use crate::localdb::rekordbox::HydrationProgress;
pub use crate::mixstatus::{MixstatusConfig, MixstatusProcessor};
pub use crate::network::NetworkConfig;
pub use crate::nfs::FetchProgress;

/// Known device types on the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceType {
    Cdj,
    Mixer,
    Rekordbox,
    Stagehand,
    /// A type byte this crate does not know.
    Other(u8),
}

impl DeviceType {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x01 => DeviceType::Cdj,
            0x03 => DeviceType::Mixer,
            0x04 => DeviceType::Rekordbox,
            0x05 => DeviceType::Stagehand,
            other => DeviceType::Other(other),
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            DeviceType::Cdj => 0x01,
            DeviceType::Mixer => 0x03,
            DeviceType::Rekordbox => 0x04,
            DeviceType::Stagehand => 0x05,
            DeviceType::Other(v) => v,
        }
    }
}

/// The 8-bit identifier of the device on the network.
pub type DeviceId = u8;

/// Represents a device on the prolink network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub id: DeviceId,
    #[serde(rename = "type")]
    pub device_type: DeviceType,
    pub mac_addr: [u8; 6],
    pub ip: Ipv4Addr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active: Option<DateTime<Utc>>,
}

impl Device {
    /// A device with no `last_active` stamp.
    pub fn new(name: impl Into<String>, id: DeviceId, device_type: DeviceType, mac_addr: [u8; 6], ip: Ipv4Addr) -> Self {
        Self { name: name.into(), id, device_type, mac_addr, ip, last_active: None }
    }
}

/// The virtual device this library announces as, shared between the services
/// that build packets from it.
///
/// Upstream hands one object to every service and mutates its `id` in place
/// when the announcer resolves a device-ID conflict; a shared lock gives the
/// same behaviour here.
pub type SharedDevice = Arc<RwLock<Device>>;

/// Wrap a device for sharing between services.
pub fn shared_device(device: Device) -> SharedDevice {
    Arc::new(RwLock::new(device))
}

/// Snapshot the device behind a [`SharedDevice`].
pub fn device_snapshot(device: &SharedDevice) -> Device {
    device.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Details of a particular media slot on the CDJ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSlotInfo {
    /// The device the slot physically exists on.
    pub device_id: DeviceId,
    /// The slot type.
    pub slot: MediaSlot,
    /// The name of the media connected.
    pub name: String,
    /// The rekordbox configured color of the media connected.
    pub color: MediaColor,
    /// Creation date, as the device reports it (`None` when the string on the
    /// wire is not a date).
    pub created_date: Option<DateTime<Utc>>,
    /// Number of free bytes available on the media.
    pub free_bytes: u64,
    /// Number of bytes used on the media.
    pub total_bytes: u64,
    /// Specifies the available tracks type on the media.
    pub tracks_type: TrackType,
    /// Total number of rekordbox tracks on the media. Will be zero if there is
    /// no rekordbox database on the media.
    pub track_count: u16,
    /// Same as track count, except for playlists.
    pub playlist_count: u16,
    /// True when a rekordbox 'my settings' file has been exported to the media.
    pub has_settings: bool,
}

/// The rekordbox media colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaColor {
    Default,
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    Aqua,
    Blue,
    Purple,
    Other(u8),
}

impl MediaColor {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => MediaColor::Default,
            0x01 => MediaColor::Pink,
            0x02 => MediaColor::Red,
            0x03 => MediaColor::Orange,
            0x04 => MediaColor::Yellow,
            0x05 => MediaColor::Green,
            0x06 => MediaColor::Aqua,
            0x07 => MediaColor::Blue,
            0x08 => MediaColor::Purple,
            other => MediaColor::Other(other),
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            MediaColor::Default => 0x00,
            MediaColor::Pink => 0x01,
            MediaColor::Red => 0x02,
            MediaColor::Orange => 0x03,
            MediaColor::Yellow => 0x04,
            MediaColor::Green => 0x05,
            MediaColor::Aqua => 0x06,
            MediaColor::Blue => 0x07,
            MediaColor::Purple => 0x08,
            MediaColor::Other(v) => v,
        }
    }
}

/// A slot where media is present on the CDJ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaSlot {
    Empty,
    Cd,
    Sd,
    Usb,
    Rb,
    /// Possibly TIDAL, Apple Music, or other streaming.
    Unknown05,
    StreamingDirectPlay,
    /// Possibly TIDAL, Apple Music, or other streaming.
    Unknown07,
    /// Possibly TIDAL, Apple Music, or other streaming.
    Unknown08,
    Beatport,
    Other(u8),
}

impl MediaSlot {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => MediaSlot::Empty,
            0x01 => MediaSlot::Cd,
            0x02 => MediaSlot::Sd,
            0x03 => MediaSlot::Usb,
            0x04 => MediaSlot::Rb,
            0x05 => MediaSlot::Unknown05,
            0x06 => MediaSlot::StreamingDirectPlay,
            0x07 => MediaSlot::Unknown07,
            0x08 => MediaSlot::Unknown08,
            0x09 => MediaSlot::Beatport,
            other => MediaSlot::Other(other),
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            MediaSlot::Empty => 0x00,
            MediaSlot::Cd => 0x01,
            MediaSlot::Sd => 0x02,
            MediaSlot::Usb => 0x03,
            MediaSlot::Rb => 0x04,
            MediaSlot::Unknown05 => 0x05,
            MediaSlot::StreamingDirectPlay => 0x06,
            MediaSlot::Unknown07 => 0x07,
            MediaSlot::Unknown08 => 0x08,
            MediaSlot::Beatport => 0x09,
            MediaSlot::Other(v) => v,
        }
    }

    /// True for the two slots a rekordbox database can live in.
    pub const fn is_database_slot(self) -> bool {
        matches!(self, MediaSlot::Usb | MediaSlot::Sd)
    }
}

/// Track type flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackType {
    None,
    Rb,
    Unanalyzed,
    AudioCd,
    Streaming,
    Other(u8),
}

impl TrackType {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => TrackType::None,
            0x01 => TrackType::Rb,
            0x02 => TrackType::Unanalyzed,
            0x05 => TrackType::AudioCd,
            0x06 => TrackType::Streaming,
            other => TrackType::Other(other),
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            TrackType::None => 0x00,
            TrackType::Rb => 0x01,
            TrackType::Unanalyzed => 0x02,
            TrackType::AudioCd => 0x05,
            TrackType::Streaming => 0x06,
            TrackType::Other(v) => v,
        }
    }
}

/// One beat of a [`BeatGrid`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Beat {
    /// Offset from the beginning of track in milliseconds of this beat.
    pub offset: u32,
    /// The count of this particular beat within the measure (1-4).
    pub count: u8,
    /// The BPM at this beat.
    pub bpm: f64,
}

/// A beat grid is a series of offsets from the start of the track. Each offset
/// indicates what count within the measure it is along with the BPM.
pub type BeatGrid = Vec<Beat>;

/// A waveform segment contains a height and 'whiteness' value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WaveformSegment {
    /// The height this segment in the waveform. Ranges from 0 - 31.
    pub height: u8,
    /// The level of "whiteness" of the waveform. 0 being completely blue, and
    /// 1 being completely white.
    pub whiteness: f64,
}

/// A HD waveform segment contains the height of the waveform, and its color
/// represented as RGB values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WaveformHDSegment {
    /// The height this segment in the waveform. Ranges from 0 - 31.
    pub height: u8,
    /// The RGB value, each channel ranges from 0-1 for the segment.
    pub color: [f64; 3],
}

/// The waveform preview will be 400 segments of data.
pub type WaveformPreview = Vec<WaveformSegment>;

/// Detailed waveforms have 150 segments per second of audio (150 'half frames'
/// per second of audio).
pub type WaveformDetailed = Vec<WaveformSegment>;

/// HD waveforms have 150 segments per second of audio (150 'half frames' per
/// second of audio).
pub type WaveformHD = Vec<WaveformHDSegment>;

/// The result of looking up track waveforms.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Waveforms {
    /// The full-size and full-color waveform.
    pub waveform_hd: WaveformHD,
    /// Color waveform preview (PWV4 tag).
    /// Raw bytes: 1200 columns × 6 bytes per column (3 frequency bands × 2 bytes each).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveform_color_preview: Option<Vec<u8>>,
    /// Standard waveform preview (400 entries). Available for streaming tracks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveform_preview: Option<WaveformPreview>,
    /// Full detailed waveform. Available for streaming tracks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveform_detailed: Option<WaveformDetailed>,
}

/// An RGB triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Extended cue with color and comment support (PCO2 tag from rekordbox).
/// Includes additional metadata like RGB colors, comments, and quantized loop
/// information.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtendedCue {
    /// Hot cue number (0 for memory points, 1-8 for hot cues A-H).
    pub hot_cue: u32,
    /// Type of cue: 1 = simple position/cue, 2 = loop.
    #[serde(rename = "type")]
    pub cue_type: u8,
    /// Position in milliseconds from the start of the track.
    pub time: u32,
    /// For loops, the end position in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_time: Option<u32>,
    /// Color ID referencing the color table (for memory points/loops).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_id: Option<u8>,
    /// Color code for the hot cue palette (0x00 = default green, 0x01-0x3e =
    /// palette colors).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_code: Option<u8>,
    /// RGB color values used to illuminate the player's RGB LEDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_rgb: Option<Rgb>,
    /// User-assigned comment text for the cue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// For quantized loops, the numerator of the loop size fraction (e.g., 4
    /// for a 4-beat loop).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_numerator: Option<u16>,
    /// For quantized loops, the denominator of the loop size fraction (e.g.,
    /// 1 for a 4-beat loop).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_denominator: Option<u16>,
}

/// A phrase within a track's song structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phrase {
    /// Sequential phrase number starting from 1.
    pub index: u16,
    /// Beat number where this phrase begins.
    pub beat: u16,
    /// Raw phrase kind value from rekordbox.
    pub kind: u16,
    /// Human-readable phrase type (e.g., "Intro", "Verse 1", "Chorus").
    pub phrase_type: String,
    /// Whether this phrase has a fill-in section (non-zero if present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<u8>,
    /// Beat number where the fill-in begins (if present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_beat: Option<u16>,
}

/// Overall mood classification of a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    High,
    Mid,
    Low,
}

/// Stylistic bank assigned for lighting control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bank {
    Default,
    Cool,
    Natural,
    Hot,
    Subtle,
    Warm,
    Vivid,
    Club1,
    Club2,
}

/// Song structure / phrase analysis (PSSI tag from rekordbox).
/// Used by CDJ-3000 players for phrase-based navigation and lighting control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SongStructure {
    /// Overall mood classification of the track.
    pub mood: Mood,
    /// Stylistic bank assigned for lighting control.
    pub bank: Bank,
    /// Beat number where the last phrase ends (track may continue after this).
    pub end_beat: u16,
    /// List of identified phrases in the track.
    pub phrases: Vec<Phrase>,
}

/// 3-band color waveform preview (PWV6 tag from .2EX files).
/// Same resolution as PWV4 (typically 1200 entries) but with separate low,
/// mid, and high frequency band amplitudes.
///
/// See <https://djl-analysis.deepsymmetry.org/djl-analysis/track-metadata.html#color-3band-preview-waveform>
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waveform3BandPreview {
    pub num_entries: u32,
    /// Raw interleaved bytes: num_entries × 3 (low, mid, high per entry).
    pub data: Vec<u8>,
}

/// 3-band color detail waveform (PWV7 tag from .2EX files).
/// Higher resolution than PWV6, approximately 150 entries per second.
///
/// See <https://djl-analysis.deepsymmetry.org/djl-analysis/track-metadata.html#color-3band-detail-waveform>
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waveform3BandDetail {
    pub num_entries: u32,
    /// Raw interleaved bytes: num_entries × 3 (low, mid, high per entry).
    pub data: Vec<u8>,
}

/// Vocal detection configuration (PWVC tag from .2EX files).
/// Threshold values used to classify frequency content as vocal or non-vocal.
///
/// Values are u16 but observed range across 192 real files is 80-159,
/// matching the 0-255 byte scale used by waveform band values.
/// Observed ranges: low 80-114, mid 80-146, high 98-159.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocalConfig {
    pub threshold_low: u16,
    pub threshold_mid: u16,
    pub threshold_high: u16,
}

/// Monochrome waveform preview data (PWAV/PWV2 tags).
/// PWAV contains 400 bytes, PWV2 contains 100 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaveformPreviewData {
    /// Raw waveform data - each byte encodes height and whiteness.
    pub data: Vec<u8>,
}

/// Represents the contents of a playlist.
///
/// Upstream exposes the tracks as an async iterator because looking up track
/// metadata may be slow when connected to the remote database. Here the
/// entries are listed eagerly and each track is fetched on demand through
/// [`crate::db::Database::playlist_track`] / the remote query interface.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistContents {
    /// The playlists in this playlist.
    pub playlists: Vec<crate::entities::Playlist>,
    /// The folders in this playlist.
    pub folders: Vec<crate::entities::Playlist>,
    /// The IDs of the tracks in this playlist, in playlist order.
    pub track_ids: Vec<u32>,
    /// The total number of tracks in this playlist.
    pub total_tracks: usize,
}

/// The lifecycle state of a [`crate::network::ProlinkNetwork`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkState {
    /// The network is offline when we don't have an open connection to the
    /// network (no connection to the announcement and or status UDP socket is
    /// present).
    Offline,
    /// The network is online when we have opened sockets to the network, but
    /// have not yet started announcing ourselves as a virtual CDJ.
    Online,
    /// The network is connected once we have heard from another device on the
    /// network.
    Connected,
    /// The network may have failed to connect if we aren't able to open the
    /// announcement and or status UDP socket.
    Failed,
}

/// Mixstatus reporting modes specify how the mixstatus processor will
/// determine when a new track is 'now playing'.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MixstatusMode {
    /// Tracks will be smartly marked as playing following rules:
    ///
    /// - The track that has been in the play state with the CDJ in the "on
    ///   air" state for the longest period of time (allowing for a
    ///   configurable length of interruption with allowed_interrupt_beats) is
    ///   considered to be the active track that incoming tracks will be
    ///   compared against.
    /// - A incoming track will immediately be reported as now playing if it
    ///   is on air, playing, and the last active track has been cued.
    /// - A incoming track will be reported as now playing if the active track
    ///   has not been on air or has not been playing for the configured
    ///   allowed_interrupt_beats.
    /// - A incoming track will be reported as now playing if it has played
    ///   consecutively (with allowed_interrupt_beats honored for the incoming
    ///   track) for the configured beats_until_reported.
    SmartTiming,
    /// Tracks will not be reported after the beats_until_reported AND will
    /// ONLY be reported if the other track has gone into a non-playing play
    /// state, or taken off air (when use_on_air_status is enabled).
    WaitsForSilence,
    /// The track will simply be reported only after the player becomes master.
    FollowsMaster,
}
