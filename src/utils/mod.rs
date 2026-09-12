//! Small helpers shared across the crate.

pub mod converters;
pub mod net;
pub mod udp;

use std::net::Ipv4Addr;

use crate::types::{Device, MediaSlot, TrackType};

pub use net::{network_interfaces, InterfaceInfo};

/// Get the byte representation of the device name: 20 bytes, ASCII, NUL
/// padded (and truncated when longer).
pub fn build_name(device: &Device) -> [u8; 20] {
    build_name_str(&device.name)
}

/// [`build_name`] for a bare name.
pub fn build_name_str(name: &str) -> [u8; 20] {
    let mut out = [0u8; 20];
    for (dst, src) in out.iter_mut().zip(name.bytes()) {
        *dst = src;
    }
    out
}

/// Determines the interface that routes the given address by comparing the
/// masked addresses. This type of information is generally determined through
/// the kernel's routing table, but for sake of cross-platform compatibility,
/// we do some rudimentary lookup.
pub fn get_matching_interface(ip_addr: Ipv4Addr) -> Option<InterfaceInfo> {
    let mut matched: Option<InterfaceInfo> = None;
    let mut matched_subnet = 0u32;

    for iface in network_interfaces() {
        if iface.internal {
            continue;
        }
        let prefix = iface.prefix_len();
        if iface.contains(ip_addr) && prefix > matched_subnet {
            matched_subnet = prefix;
            matched = Some(iface);
        }
    }

    matched
}

/// Computes the IPv4 subnet broadcast address for a network interface.
///
/// Builds the address from the interface netmask so the broadcast covers the
/// whole subnet (e.g. x.x.x.255 for a /24).
pub fn get_broadcast_address(iface: &InterfaceInfo) -> Ipv4Addr {
    iface.broadcast()
}

/// Given a BPM and pitch value, compute how many seconds per beat.
pub fn bpm_to_seconds(bpm: f64, pitch: f64) -> f64 {
    let bps = ((pitch / 100.0) * bpm + bpm) / 60.0;
    1.0 / bps
}

/// Returns a string representation of a media slot.
pub fn get_slot_name(slot: MediaSlot) -> String {
    match slot {
        MediaSlot::Empty => "empty".into(),
        MediaSlot::Cd => "cd".into(),
        MediaSlot::Sd => "sd".into(),
        MediaSlot::Usb => "usb".into(),
        MediaSlot::Rb => "rb".into(),
        MediaSlot::Unknown05 => "unknown05".into(),
        MediaSlot::StreamingDirectPlay => "streamingdirectplay".into(),
        MediaSlot::Unknown07 => "unknown07".into(),
        MediaSlot::Unknown08 => "unknown08".into(),
        MediaSlot::Beatport => "beatport".into(),
        MediaSlot::Other(v) => format!("unknown{v:02x}"),
    }
}

/// Returns a string representation of a track type.
pub fn get_track_type_name(track_type: TrackType) -> String {
    match track_type {
        TrackType::None => "none".into(),
        TrackType::Rb => "rb".into(),
        TrackType::Unanalyzed => "unanalyzed".into(),
        TrackType::AudioCd => "audiocd".into(),
        TrackType::Streaming => "streaming".into(),
        TrackType::Other(v) => format!("unknown{v:02x}"),
    }
}

/// Decode a NUL-padded byte field the way upstream does
/// (`buffer.toString().replace(/\0/g, '')`): drop every NUL byte and decode
/// what is left as UTF-8, lossily.
pub fn string_from_nul_padded(bytes: &[u8]) -> String {
    let clean: Vec<u8> = bytes.iter().copied().filter(|b| *b != 0).collect();
    String::from_utf8_lossy(&clean).into_owned()
}

/// Parse a date the way JavaScript's `new Date(string)` accepts the strings
/// rekordbox writes: ISO 8601 with or without time, `YYYY-MM-DD HH:MM:SS`,
/// `YYYY-MM-DD`, `YYYY/MM/DD`. Returns `None` for anything else (JavaScript's
/// "Invalid Date").
pub fn parse_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S", "%Y/%m/%d %H:%M:%S"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc());
        }
    }
    for fmt in ["%Y-%m-%d", "%Y/%m/%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Some(d.and_hms_opt(0, 0, 0)?.and_utc());
        }
    }
    None
}

/// Current time in milliseconds since the Unix epoch (`Date.now()`).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpm_to_seconds_matches_upstream() {
        assert_eq!(bpm_to_seconds(60.0, 0.0), 1.0);
        assert_eq!(bpm_to_seconds(120.0, 0.0), 0.5);
        assert!((bpm_to_seconds(60.0, 25.0) - 0.8).abs() < 1e-12);
    }

    #[test]
    fn build_name_pads_to_20_bytes() {
        let device = Device::new("CDJ-3000", 1, crate::types::DeviceType::Cdj, [0; 6], Ipv4Addr::new(192, 168, 1, 1));
        let name = build_name(&device);
        assert_eq!(name.len(), 20);
        assert!(name.starts_with(b"CDJ-3000"));
        let short = build_name_str("XDJ");
        assert_eq!(short[3], 0);
    }

    #[test]
    fn slot_and_track_type_names() {
        assert_eq!(get_slot_name(MediaSlot::Usb), "usb");
        assert_eq!(get_slot_name(MediaSlot::Sd), "sd");
        assert_eq!(get_slot_name(MediaSlot::Rb), "rb");
        assert_eq!(get_track_type_name(TrackType::Rb), "rb");
        assert_eq!(get_track_type_name(TrackType::Unanalyzed), "unanalyzed");
        assert_eq!(get_track_type_name(TrackType::AudioCd), "audiocd");
    }

    #[test]
    fn parses_rekordbox_dates() {
        assert_eq!(parse_date("2020-10-10").unwrap().to_rfc3339(), "2020-10-10T00:00:00+00:00");
        assert!(parse_date("2024-01-15 10:30:00").is_some());
        assert!(parse_date("").is_none());
        assert!(parse_date("not a date").is_none());
    }
}
