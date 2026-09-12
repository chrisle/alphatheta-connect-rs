//! Parsers for the packets that arrive on the status (50002) and beat (50001)
//! sockets.

use std::collections::BTreeMap;

use crate::constants::has_prolink_header;
use crate::status::types::{
    status_flag, ChannelState, CrossfaderAssign, MixerState, OnAirStatus, PlayState, PositionState, State, VUFrame, VUState,
};
use crate::types::{MediaColor, MediaSlot, MediaSlotInfo, TrackType};
use crate::utils::{parse_date, string_from_nul_padded};
use crate::{Error, Result};

const MAX_INT32: u32 = u32::MAX;
const MAX_INT16: u16 = u16::MAX;
const MAX_INT9: u16 = (1 << 9) - 1;

fn u16_be(p: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([p[at], p[at + 1]])
}

fn u32_be(p: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}

fn u64_be(p: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&p[at..at + 8]);
    u64::from_be_bytes(b)
}

/// Parse a CDJ status packet.
///
/// Returns `Ok(None)` for prolink packets that are too short to be a status
/// packet (rekordbox sends some short status packets that we can just
/// ignore), and an error when the packet does not carry the prolink header.
pub fn status_from_packet(packet: &[u8]) -> Result<Option<State>> {
    if !has_prolink_header(packet) {
        return Err(Error::protocol("CDJ status packet does not start with the expected header"));
    }

    // Rekordbox sends some short status packets that we can just ignore.
    if packet.len() < 0xc8 {
        return Ok(None);
    }

    // packetNum is read at 0xc8..0xcc, which the length check does not cover.
    if packet.len() < 0xcc {
        return Ok(None);
    }

    // No track loaded: BPM = MAX_INT16
    let raw_bpm = u16_be(packet, 0x92);
    let track_bpm = if raw_bpm == MAX_INT16 { None } else { Some(f64::from(raw_bpm) / 100.0) };

    // No next cue: beatsUntilCue = MAX_INT9
    let raw_beats_until_cue = u16_be(packet, 0xa4);
    let beats_until_cue = if raw_beats_until_cue == MAX_INT9 { None } else { Some(raw_beats_until_cue) };

    // No track loaded: beat = MAX_INT32
    let raw_beat = u32_be(packet, 0xa0);
    let beat = if raw_beat == MAX_INT32 { None } else { Some(raw_beat) };

    let flags = packet[0x89];

    Ok(Some(State {
        device_id: packet[0x21],
        track_id: u32_be(packet, 0x2c),
        track_device_id: packet[0x28],
        track_slot: MediaSlot::from_u8(packet[0x29]),
        track_type: TrackType::from_u8(packet[0x2a]),
        play_state: PlayState::from_u8(packet[0x7b]),
        is_on_air: flags & status_flag::ON_AIR != 0,
        is_sync: flags & status_flag::SYNC != 0,
        is_bpm_sync: flags & status_flag::BPM_SYNC != 0,
        is_master: flags & status_flag::MASTER != 0,
        is_emergency_mode: packet[0xba] != 0,
        track_bpm,
        // The CDJ status packet carries four pitch values (firmware CDJ-3000
        // FW3.20, sub_d7d610): Pitch1@0x8c and Pitch3@0xc0 are the *effective*
        // pitch (in effect / on the BPM display, from the fader or a synced
        // master), while Pitch2@0x98 and Pitch4@0xc4 track the *local fader*
        // position. We expose Pitch1 as `effective_pitch` and Pitch2 as
        // `slider_pitch`.
        effective_pitch: calc_pitch(&packet[0x8d..0x8d + 3]),
        slider_pitch: calc_pitch(&packet[0x99..0x99 + 3]),
        beat_in_measure: packet[0xa6],
        beats_until_cue,
        beat,
        // Player-type / capability byte (dysentery's "nx"). Guarded because the
        // length check above only guarantees 0xcc; older/short packets may not
        // reach 0xcc, in which case the field reads as 0 (version-gated to 0
        // anyway).
        device_type: if packet.len() > 0xcc { packet[0xcc] } else { 0 },
        packet_num: u32_be(packet, 0xc8),
    }))
}

/// Parse a media slot info packet (type 0x06 on the status socket).
///
/// Returns `Ok(None)` for other packet types and an error when the packet does
/// not carry the prolink header.
pub fn media_slot_from_packet(packet: &[u8]) -> Result<Option<MediaSlotInfo>> {
    if !has_prolink_header(packet) {
        return Err(Error::protocol("CDJ media slot packet does not start with the expected header"));
    }

    if packet.get(0x0a) != Some(&0x06) {
        return Ok(None);
    }

    if packet.len() < 0xc0 {
        return Err(Error::protocol(format!("CDJ media slot packet too short: {} bytes", packet.len())));
    }

    let name = string_from_nul_padded(&packet[0x2c..0x2c + 40]);
    let created_date = parse_date(&string_from_nul_padded(&packet[0x6c..0x6c + 24]));

    Ok(Some(MediaSlotInfo {
        device_id: packet[0x27],
        slot: MediaSlot::from_u8(packet[0x2b]),
        name,
        color: MediaColor::from_u8(packet[0xa8]),
        created_date,
        free_bytes: u64_be(packet, 0xb8),
        total_bytes: u64_be(packet, 0xb0),
        tracks_type: TrackType::from_u8(packet[0xaa]),
        track_count: u16_be(packet, 0xa6),
        playlist_count: u16_be(packet, 0xae),
        has_settings: packet[0xab] != 0,
    }))
}

/// Converts a uint24 byte value into a pitch percentage.
///
/// The pitch information ranges from 0x000000 (meaning -100%, complete stop)
/// to 0x200000 (+100%).
fn calc_pitch(pitch: &[u8]) -> f64 {
    let value = u32::from_be_bytes([0x00, pitch[0], pitch[1], pitch[2]]);
    let relative_zero = 0x100000_i64;

    let computed = ((i64::from(value) - relative_zero) as f64 / relative_zero as f64) * 100.0;

    // `+computed.toFixed(2)`
    (computed * 100.0).round() / 100.0
}

/// Parse absolute position packet from CDJ-3000+ devices.
/// These packets are sent every 30ms on port 50001 while a track is loaded.
/// Packet structure: subtype 0x00, lenr varies based on device.
pub fn position_from_packet(packet: &[u8]) -> Option<PositionState> {
    if !has_prolink_header(packet) {
        return None;
    }

    // Check if this is a position packet (subtype 0x00)
    if packet.get(0x20) != Some(&0x00) {
        return None;
    }

    // Check minimum length for position packet
    if packet.len() < 0x34 {
        return None;
    }
    let lenr = u16_be(packet, 0x22);
    if lenr < 0x0c {
        return None;
    }

    let device_id = packet[0x21];
    let track_length = u32_be(packet, 0x24);
    let playhead = u32_be(packet, 0x28);

    // Parse pitch: 32-bit signed integer representing pitch × 64 × 100.
    // To get percentage: divide by 6400.
    let raw_pitch = u32_be(packet, 0x2c) as i32;
    let pitch = f64::from(raw_pitch) / 6400.0;

    // Parse BPM: multiply by 10, or null if 0xffffffff
    let raw_bpm = u32_be(packet, 0x30);
    let bpm = if raw_bpm == 0xffff_ffff { None } else { Some(f64::from(raw_bpm) / 10.0) };

    Some(PositionState { device_id, track_length, playhead, pitch, bpm })
}

/// Parse on-air status packet from DJM mixer.
/// The mixer broadcasts which channels are currently audible.
/// Supports both 4-channel (subtype 0x00) and 6-channel (subtype 0x03) variants.
///
/// Packet structure:
/// - 4-channel: subtype 0x00, length 0x0009 (9 data bytes: F1 F2 F3 F4 00 00 00 00 00)
/// - 6-channel: subtype 0x03, length 0x0011 (17 data bytes: F1 F2 F3 F4 00 00 00 00 00 F5 F6 00 30 00 00 00 00 00)
pub fn on_air_from_packet(packet: &[u8]) -> Option<OnAirStatus> {
    if !has_prolink_header(packet) || packet.len() < 0x24 {
        return None;
    }

    let subtype = packet[0x20];
    let lenr = u16_be(packet, 0x22);

    // Check for 4-channel variant (subtype 0x00, length 0x0009)
    if subtype == 0x00 && lenr == 0x0009 && packet.len() >= 0x2e {
        let mut channels = BTreeMap::new();
        channels.insert(1, packet[0x24] != 0);
        channels.insert(2, packet[0x25] != 0);
        channels.insert(3, packet[0x26] != 0);
        channels.insert(4, packet[0x27] != 0);
        return Some(OnAirStatus { device_id: packet[0x21], channels, is_six_channel: false });
    }

    // Check for 6-channel variant (subtype 0x03, length 0x0011)
    if subtype == 0x03 && lenr == 0x0011 && packet.len() >= 0x36 {
        let mut channels = BTreeMap::new();
        channels.insert(1, packet[0x24] != 0);
        channels.insert(2, packet[0x25] != 0);
        channels.insert(3, packet[0x26] != 0);
        channels.insert(4, packet[0x27] != 0);
        channels.insert(5, packet[0x2e] != 0);
        channels.insert(6, packet[0x2f] != 0);
        return Some(OnAirStatus { device_id: packet[0x21], channels, is_six_channel: true });
    }

    None
}

/// Parse unicast mixer state packet from DJM-A9 (packet type 0x39 on port 50002).
pub fn mixer_state_from_packet(packet: &[u8]) -> Option<MixerState> {
    if !has_prolink_header(packet) {
        return None;
    }

    // Verify packet type 0x39
    if packet.get(10) != Some(&0x39) {
        return None;
    }

    if packet.len() < 266 {
        return None;
    }

    let device_name = string_from_nul_padded(&packet[11..30]).trim().to_string();
    let crossfader = packet[180];

    let mut channels = BTreeMap::new();
    for ch in 1u8..=4 {
        let offset = 36 + usize::from(ch - 1) * 24;
        let crossfader_assign = match packet[offset + 12] {
            0x01 => CrossfaderAssign::A,
            0x02 => CrossfaderAssign::B,
            _ => CrossfaderAssign::Thru,
        };
        channels.insert(
            ch,
            ChannelState {
                trim: packet[offset + 1],
                eq_hi: packet[offset + 3],
                eq_mid: packet[offset + 4],
                eq_low: packet[offset + 6],
                color_fx: packet[offset + 7],
                fader: packet[offset + 11],
                crossfader_assign,
            },
        );
    }

    Some(MixerState {
        // Mixers are typically device 33 on ProDJLink
        device_id: 33,
        device_name,
        channels,
        crossfader,
    })
}

/// Parse unicast VU meter packet from DJM-A9 (packet type 0x58 on port 50001).
/// Contains 15 sample-tuples (16-bit BE left/right levels) per channel.
pub fn vu_from_packet(packet: &[u8]) -> Option<VUState> {
    if !has_prolink_header(packet) {
        return None;
    }

    // Verify packet type 0x58
    if packet.get(10) != Some(&0x58) {
        return None;
    }

    if packet.len() < 584 {
        return None;
    }

    let mut channels = BTreeMap::new();
    for ch in 1u8..=4 {
        // 15 frames * 4 bytes per frame = 60 bytes
        let ch_offset = 44 + usize::from(ch - 1) * 60;
        let frames = (0..15)
            .map(|i| {
                let at = ch_offset + i * 4;
                VUFrame { left: u16_be(packet, at), right: u16_be(packet, at + 2) }
            })
            .collect();
        channels.insert(ch, frames);
    }

    Some(VUState { device_id: 33, channels })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::PROLINK_HEADER;

    fn with_header(len: usize) -> Vec<u8> {
        let mut p = vec![0u8; len];
        p[..10].copy_from_slice(&PROLINK_HEADER);
        p
    }

    #[test]
    fn status_rejects_non_prolink() {
        assert!(status_from_packet(&[]).is_err());
    }

    #[test]
    fn status_ignores_short_packets() {
        let mut p = PROLINK_HEADER.to_vec();
        p.extend_from_slice(&[0, 0]);
        assert!(status_from_packet(&p).unwrap().is_none());
    }

    #[test]
    fn status_parses_a_real_packet() {
        let packet = include_bytes!("../../tests/data/status-simple.dat");
        let s = status_from_packet(packet).unwrap().unwrap();
        assert_eq!(s.packet_num, 74108);
        assert_eq!(s.device_id, 3);
        assert_eq!(s.beat, None);
        assert_eq!(s.beat_in_measure, 0);
        assert_eq!(s.beats_until_cue, None);
        assert_eq!(s.device_type, 0x0f);
        assert_eq!(s.effective_pitch, 0.0);
        assert!(!s.is_master && !s.is_on_air && !s.is_sync && !s.is_bpm_sync && !s.is_emergency_mode);
        assert_eq!(s.play_state, PlayState::Empty);
        assert_eq!(s.slider_pitch, 0.0);
        assert_eq!(s.track_bpm, None);
        assert_eq!(s.track_device_id, 0);
        assert_eq!(s.track_id, 0);
        assert_eq!(s.track_slot, MediaSlot::Empty);
        assert_eq!(s.track_type, TrackType::None);
    }

    #[test]
    fn media_slot_parses_a_real_packet() {
        let packet = include_bytes!("../../tests/data/media-slot-usb.dat");
        let m = media_slot_from_packet(packet).unwrap().unwrap();
        assert_eq!(m.color, MediaColor::Default);
        assert_eq!(m.slot, MediaSlot::Usb);
        assert_eq!(m.name, "");
        assert_eq!(m.device_id, 2);
        assert_eq!(m.created_date.unwrap().to_rfc3339(), "2020-10-10T00:00:00+00:00");
        assert_eq!(m.playlist_count, 1);
        assert_eq!(m.track_count, 76);
        assert_eq!(m.tracks_type, TrackType::Rb);
        assert!(m.has_settings);
        assert_eq!(m.total_bytes, 62_714_675_200);
        assert_eq!(m.free_bytes, 61_048_520_704);
    }

    #[test]
    fn media_slot_only_handles_type_06() {
        let mut p = PROLINK_HEADER.to_vec();
        p.push(0x05);
        assert!(media_slot_from_packet(&p).unwrap().is_none());
        assert!(media_slot_from_packet(&[]).is_err());
    }

    #[test]
    fn calc_pitch_ranges() {
        assert_eq!(calc_pitch(&[0x10, 0x00, 0x00]), 0.0);
        assert_eq!(calc_pitch(&[0x00, 0x00, 0x00]), -100.0);
        assert_eq!(calc_pitch(&[0x20, 0x00, 0x00]), 100.0);
    }

    #[test]
    fn mixer_state_parses() {
        assert!(mixer_state_from_packet(&[]).is_none());
        let mut p = with_header(266);
        p[10] = 0x0a;
        assert!(mixer_state_from_packet(&p).is_none());
        let mut short = with_header(100);
        short[10] = 0x39;
        assert!(mixer_state_from_packet(&short).is_none());

        let mut p = with_header(266);
        p[10] = 0x39;
        p[11..17].copy_from_slice(b"DJM-A9");
        p[180] = 120;
        for ch in 1u8..=4 {
            let o = 36 + usize::from(ch - 1) * 24;
            p[o + 1] = 100 + ch;
            p[o + 3] = 110 + ch;
            p[o + 4] = 120 + ch;
            p[o + 6] = 130 + ch;
            p[o + 7] = 140 + ch;
            p[o + 11] = 150 + ch;
            p[o + 12] = match ch {
                1 => 0x01,
                2 => 0x02,
                _ => 0x00,
            };
        }
        let s = mixer_state_from_packet(&p).unwrap();
        assert_eq!(s.device_id, 33);
        assert_eq!(s.device_name, "DJM-A9");
        assert_eq!(s.crossfader, 120);
        assert_eq!(
            s.channels[&1],
            ChannelState {
                trim: 101,
                eq_hi: 111,
                eq_mid: 121,
                eq_low: 131,
                color_fx: 141,
                fader: 151,
                crossfader_assign: CrossfaderAssign::A
            }
        );
        assert_eq!(s.channels[&2].crossfader_assign, CrossfaderAssign::B);
        assert_eq!(s.channels[&3].crossfader_assign, CrossfaderAssign::Thru);
        assert_eq!(s.channels[&4].fader, 154);
    }

    #[test]
    fn vu_parses() {
        assert!(vu_from_packet(&[]).is_none());
        let mut wrong = with_header(584);
        wrong[10] = 0x0a;
        assert!(vu_from_packet(&wrong).is_none());
        let mut short = with_header(300);
        short[10] = 0x58;
        assert!(vu_from_packet(&short).is_none());

        let mut p = with_header(584);
        p[10] = 0x58;
        for ch in 1u16..=4 {
            let ch_offset = 44 + usize::from(ch - 1) * 60;
            for i in 0..15u16 {
                let at = ch_offset + usize::from(i) * 4;
                p[at..at + 2].copy_from_slice(&(1000 * ch + i).to_be_bytes());
                p[at + 2..at + 4].copy_from_slice(&(2000 * ch + i).to_be_bytes());
            }
        }
        let s = vu_from_packet(&p).unwrap();
        assert_eq!(s.device_id, 33);
        assert_eq!(s.channels[&1][0], VUFrame { left: 1000, right: 2000 });
        assert_eq!(s.channels[&1][14], VUFrame { left: 1014, right: 2014 });
        assert_eq!(s.channels[&4][0], VUFrame { left: 4000, right: 8000 });
        assert_eq!(s.channels[&4][14], VUFrame { left: 4014, right: 8014 });
    }

    #[test]
    fn on_air_four_and_six_channel() {
        let mut p = with_header(0x2e);
        p[0x20] = 0x00;
        p[0x21] = 33;
        p[0x22..0x24].copy_from_slice(&0x0009u16.to_be_bytes());
        p[0x24] = 1;
        p[0x26] = 1;
        let s = on_air_from_packet(&p).unwrap();
        assert!(!s.is_six_channel);
        assert!(s.channel(1) && !s.channel(2) && s.channel(3) && !s.channel(4));
        assert!(!s.channel(5));

        let mut p = with_header(0x36);
        p[0x20] = 0x03;
        p[0x21] = 33;
        p[0x22..0x24].copy_from_slice(&0x0011u16.to_be_bytes());
        p[0x2e] = 1;
        let s = on_air_from_packet(&p).unwrap();
        assert!(s.is_six_channel);
        assert!(s.channel(5) && !s.channel(6));
    }

    #[test]
    fn position_parses() {
        let mut p = with_header(0x34);
        p[0x20] = 0x00;
        p[0x21] = 2;
        p[0x22..0x24].copy_from_slice(&0x0c_u16.to_be_bytes());
        p[0x24..0x28].copy_from_slice(&300u32.to_be_bytes());
        p[0x28..0x2c].copy_from_slice(&12345u32.to_be_bytes());
        p[0x2c..0x30].copy_from_slice(&(-6400i32).to_be_bytes());
        p[0x30..0x34].copy_from_slice(&1280u32.to_be_bytes());
        let s = position_from_packet(&p).unwrap();
        assert_eq!(s.device_id, 2);
        assert_eq!(s.track_length, 300);
        assert_eq!(s.playhead, 12345);
        assert_eq!(s.pitch, -1.0);
        assert_eq!(s.bpm, Some(128.0));

        p[0x30..0x34].copy_from_slice(&0xffff_ffffu32.to_be_bytes());
        assert_eq!(position_from_packet(&p).unwrap().bpm, None);
    }
}
