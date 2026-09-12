//! CDJ status types, upstream's `CDJStatus` namespace.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::{DeviceId, MediaSlot, TrackType};

/// Status flag bitmasks (byte 0x89 of the CDJ status packet).
///
/// Verified against CDJ-3000 firmware (EP122 FW3.20): the deck assembles this
/// byte in `sub_d7d3c8` (0xd7d3c8) from the BeatSyncMaster state struct, one
/// source boolean per bit. The bits not named here:
///
/// - bit 0 (0x01): structurally unused — no code path ever sets it (always 0).
/// - bit 2 (0x04): a real, dedicated beat-sync boolean (struct +5), but its
///   name did not survive in the stripped `usecase::sync` async-task code. It
///   is normally 0, which is why it has never been observed on the wire.
/// - bit 7 (0x80): a real bit (struct +0xa) that is default-set at init and
///   whose de-assertion itself triggers a state-change notification — best
///   read as a "sync-master active / handoff-in-progress" toggle. Also
///   normally 0 in steady state.
pub mod status_flag {
    /// Degraded to BPM Sync: the player is still tracking the master's tempo,
    /// but beat alignment was dropped after a pitch-bend / jog nudge. Firmware
    /// sources this from BeatSyncMaster struct +4.
    pub const BPM_SYNC: u8 = 1 << 1;
    pub const ON_AIR: u8 = 1 << 3;
    pub const SYNC: u8 = 1 << 4;
    pub const MASTER: u8 = 1 << 5;
    pub const PLAYING: u8 = 1 << 6;
}

/// Play state flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlayState {
    Empty,
    Loading,
    Playing,
    Looping,
    Paused,
    Cued,
    Cuing,
    PlatterHeld,
    Searching,
    SpunDown,
    Ended,
    Other(u8),
}

impl PlayState {
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x00 => PlayState::Empty,
            0x02 => PlayState::Loading,
            0x03 => PlayState::Playing,
            0x04 => PlayState::Looping,
            0x05 => PlayState::Paused,
            0x06 => PlayState::Cued,
            0x07 => PlayState::Cuing,
            0x08 => PlayState::PlatterHeld,
            0x09 => PlayState::Searching,
            0x0e => PlayState::SpunDown,
            0x11 => PlayState::Ended,
            other => PlayState::Other(other),
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            PlayState::Empty => 0x00,
            PlayState::Loading => 0x02,
            PlayState::Playing => 0x03,
            PlayState::Looping => 0x04,
            PlayState::Paused => 0x05,
            PlayState::Cued => 0x06,
            PlayState::Cuing => 0x07,
            PlayState::PlatterHeld => 0x08,
            PlayState::Searching => 0x09,
            PlayState::SpunDown => 0x0e,
            PlayState::Ended => 0x11,
            PlayState::Other(v) => v,
        }
    }
}

/// Represents various details about the current state of the CDJ.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// The device reporting this status.
    pub device_id: DeviceId,
    /// The ID of the track loaded on the device. 0 when no track is loaded.
    pub track_id: u32,
    /// The device ID the track is loaded from.
    ///
    /// For example if you have two CDJs and you've loaded a track over the
    /// 'LINK', this will be the ID of the player with the USB media device
    /// connected to it.
    pub track_device_id: DeviceId,
    /// The MediaSlot the track is loaded from. For example a SD card or USB device.
    pub track_slot: MediaSlot,
    /// The TrackType of the track, for example a CD or Rekordbox analyzed track.
    pub track_type: TrackType,
    /// The current play state of the CDJ.
    pub play_state: PlayState,
    /// Whether the CDJ is currently reporting itself as 'on-air'.
    ///
    /// This is indicated by the red ring around the platter on the CDJ Nexus
    /// models. A DJM mixer must be on the network for the CDJ to report this
    /// as true.
    pub is_on_air: bool,
    /// Whether the CDJ is synced.
    pub is_sync: bool,
    /// Whether the CDJ has degraded into BPM Sync — still in Sync mode and
    /// tracking the master's tempo, but no longer beat-aligned because the DJ
    /// used pitch bend (e.g. nudged the jog wheel). Corresponds to
    /// [`status_flag::BPM_SYNC`] (bit 1 of byte 0x89), which older pre-nexus
    /// players never set.
    pub is_bpm_sync: bool,
    /// Whether the CDJ is the master player.
    pub is_master: bool,
    /// Whether the CDJ is in an emergency state (emergency loop / emergency
    /// mode on newer players).
    pub is_emergency_mode: bool,
    /// The BPM of the loaded track. `None` if no track is loaded or the BPM is
    /// unknown.
    pub track_bpm: Option<f64>,
    /// The pitch actually *in effect* — the value shown on the BPM display,
    /// whether it comes from the local pitch fader or a synced tempo master
    /// (packet Pitch1 @ 0x8c). This is the value to combine with `track_bpm`
    /// to get the playing BPM. It is also what is reported when the jog wheel
    /// is nudged, the platter is held, or the deck spins down on the vinyl
    /// stop knob.
    pub effective_pitch: f64,
    /// The *local pitch-fader* position (packet Pitch2 @ 0x98) — always tied
    /// to the physical fader, following the player's brake/release ramp as
    /// playback stops or starts, regardless of any sync master.
    pub slider_pitch: f64,
    /// The current beat within the measure. 1-4. 0 when no track is loaded.
    pub beat_in_measure: u8,
    /// Number of beats remaining until the next cue point is reached. `None`
    /// if there is no next cue point.
    pub beats_until_cue: Option<u16>,
    /// The beat 'timestamp' of the track. Can be used to compute absolute
    /// track time given the slider pitch.
    pub beat: Option<u32>,
    /// The player-type / capability byte (packet byte 0xcc, dysentery's "nx").
    ///
    /// This is a capability *bitfield*, not a model id. Known values: `0x05`
    /// for older (pre-nexus) players, `0x0f` for nexus, and `0x1f` for the
    /// CDJ-3000 and XDJ-XZ (the nexus value plus bit 4). Firmware (CDJ-3000
    /// FW3.20, one 16-bit store in `sub_d858b8`) hardcodes `0x1f`. The byte
    /// is version-gated: a player zeroes it toward peers advertising a Pro DJ
    /// Link protocol version below 3, so a much older device on the link may
    /// report `0`.
    pub device_type: u8,
    /// A counter that increments for every status packet sent (packet byte 0xc8).
    ///
    /// Caveat: on the CDJ-3000 this field is hardwired to 0 — the firmware
    /// never writes packet offset 0xc8 (verified: no store to the status
    /// body's +0xa4). The CDJ-3000's live per-packet counter moved to its
    /// high-resolution stream packet on UDP 50004 (a big-endian u32 at that
    /// packet's offset 0x28). Do not rely on `packet_num` to detect liveness
    /// or drops on CDJ-3000 hardware.
    pub packet_num: u32,
}

/// Absolute position information from CDJ-3000+ devices.
/// Sent every 30ms on port 50001 while a track is loaded.
/// Provides precise playhead position independent of beat grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionState {
    /// The device ID sending this position update.
    pub device_id: DeviceId,
    /// Track length in seconds (rounded down to nearest second).
    pub track_length: u32,
    /// Absolute playhead position in milliseconds.
    pub playhead: u32,
    /// Pitch slider value as shown on screen. For example, 3.26% is
    /// represented as 3.26.
    pub pitch: f64,
    /// Effective BPM (track BPM adjusted by pitch) as shown on screen. `None`
    /// if BPM is unknown.
    pub bpm: Option<f64>,
}

/// On-Air status from DJM mixer.
/// Broadcast by the mixer to indicate which channels are currently audible.
/// Supports both 4-channel (DJM-900/1000) and 6-channel (DJM-V10) mixers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnAirStatus {
    /// The mixer device ID (typically 33 / 0x21).
    pub device_id: DeviceId,
    /// On-air flags for channels 1-4 (always present), 5-6 on six-channel
    /// mixers. `true` = channel is on-air (audible).
    pub channels: BTreeMap<u8, bool>,
    /// Whether this is a 6-channel variant (CDJ-3000 + DJM-V10).
    /// Determined by packet subtype (0x00 = 4-channel, 0x03 = 6-channel).
    pub is_six_channel: bool,
}

impl OnAirStatus {
    /// Whether `channel` is on air; `false` for channels the packet did not carry.
    pub fn channel(&self, channel: u8) -> bool {
        self.channels.get(&channel).copied().unwrap_or(false)
    }
}

/// Crossfader assignment of a mixer channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CrossfaderAssign {
    #[serde(rename = "thru")]
    Thru,
    A,
    B,
}

/// State of a single mixer channel fader, EQ, trim and routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelState {
    /// Input Trim level (0-255). Unity gain is typically 128 (0x80).
    pub trim: u8,
    /// EQ High level (0-255). Fully cut at 0, unity at 128 (0x80), boosted to max at 255.
    pub eq_hi: u8,
    /// EQ Mid level (0-255).
    pub eq_mid: u8,
    /// EQ Low level (0-255).
    pub eq_low: u8,
    /// Color FX knob position (0-255). Centered at 128 (0x80).
    pub color_fx: u8,
    /// Channel fader position (0-255). 0 is completely closed, 255 is maximum level.
    pub fader: u8,
    /// Crossfader assignment.
    pub crossfader_assign: CrossfaderAssign,
}

/// Full mixer control state parsed from Stagehand unicast status (0x39 packets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixerState {
    /// The reporting device ID (typically 33).
    pub device_id: DeviceId,
    /// Device name reported by the mixer (e.g. "DJM-A9").
    pub device_name: String,
    /// State of mixer channels (1-4).
    pub channels: BTreeMap<u8, ChannelState>,
    /// Crossfader position (0-255). 0 is full-left (A), 255 is full-right (B).
    pub crossfader: u8,
}

/// A single audio VU level frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VUFrame {
    /// Left channel level (0-65535).
    pub left: u16,
    /// Right channel level (0-65535).
    pub right: u16,
}

/// Real-time sliding window audio level VU data parsed from Stagehand unicast
/// packets (0x58).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VUState {
    /// The reporting device ID (typically 33).
    pub device_id: DeviceId,
    /// Array of 15 sliding window stereo VU level frames per channel (1-4).
    pub channels: BTreeMap<u8, Vec<VUFrame>>,
}
