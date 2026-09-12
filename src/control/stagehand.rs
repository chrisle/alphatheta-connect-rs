//! Stagehand remote-control packets.

use crate::constants::PROLINK_HEADER;
use crate::types::Device;
use crate::utils::build_name;

/// Generates a Stagehand transport control packet (0x07, 56 bytes).
///
/// - `host_device`: the Stagehand device posing as sender
/// - `op`: the command opcode (e.g. 0x0f, 0x14, 0x18, 0x1a, 0x1b)
/// - `press`: whether the action is press (true) or release (false)
/// - `correlation_byte`: the randomized per-session correlation byte
pub fn make_stagehand_transport_packet(host_device: &Device, op: u8, press: bool, correlation_byte: u8) -> Vec<u8> {
    let mut packet = vec![0u8; 56];

    // 0-9: magic header
    packet[..10].copy_from_slice(&PROLINK_HEADER);
    // 10: opcode 0x07
    packet[10] = 0x07;
    // 11-30: device name
    packet[11..31].copy_from_slice(&build_name(host_device));
    // 31: 0x01
    packet[31] = 0x01;
    // 32: 0x03
    packet[32] = 0x03;
    // 33: per-session correlation byte
    packet[33] = correlation_byte;
    // 34-35: remaining length 0x0030 (48 bytes)
    packet[34] = 0x00;
    packet[35] = 0x30;
    // 40: Stagehand sub-id 0x3a
    packet[40] = 0x3a;
    // 44: command opcode
    packet[44] = op;
    // 46: press/release flag
    packet[46] = u8::from(press);

    packet
}

/// On-air preference value for [`make_stagehand_pref_write_packet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnAirPref {
    On,
    Off,
}

/// Preferences a Stagehand write packet can carry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StagehandPreferences {
    pub on_air: Option<OnAirPref>,
    pub quantize: Option<u8>,
}

/// Generates a Stagehand preference write packet (0x6b, 124 bytes).
pub fn make_stagehand_pref_write_packet(host_device: &Device, options: StagehandPreferences) -> Vec<u8> {
    let mut packet = vec![0u8; 124];

    // 0-9: magic header
    packet[..10].copy_from_slice(&PROLINK_HEADER);
    // 10: opcode 0x6b
    packet[10] = 0x6b;
    // 11-30: device name. Trailing byte 30 (index 19 of the name) is set to 0x03
    let mut name = build_name(host_device);
    name[19] = 0x03;
    packet[11..31].copy_from_slice(&name);
    // 31: 0x01 (subscription-id-a constant)
    packet[31] = 0x01;
    // 32: 0x03 (constant)
    packet[32] = 0x03;
    // 33: Stagehand sub-id constant 0x3a
    packet[33] = 0x3a;
    // 34-35: body length 0x0050 (80 bytes)
    packet[34] = 0x00;
    packet[35] = 0x50;
    // 36: transaction flag (0x01 = write)
    packet[36] = 0x01;
    // 44: on_air slot (0x80 = OFF, 0x81 = ON, 0x00 = untouched)
    match options.on_air {
        Some(OnAirPref::On) => packet[44] = 0x81,
        Some(OnAirPref::Off) => packet[44] = 0x80,
        None => {}
    }
    // 60: quantize slot (0x80 | enum_index, e.g. 0x81, 0x82 etc.)
    if let Some(q) = options.quantize {
        packet[60] = 0x80 | q;
    }

    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DeviceType;
    use std::net::Ipv4Addr;

    fn host() -> Device {
        Device::new("Stagehand", 154, DeviceType::Stagehand, [0; 6], Ipv4Addr::new(10, 0, 0, 9))
    }

    #[test]
    fn transport_packet_layout() {
        let p = make_stagehand_transport_packet(&host(), 0x0f, true, 0xab);
        assert_eq!(p.len(), 56);
        assert_eq!(p[10], 0x07);
        assert_eq!(&p[11..20], b"Stagehand");
        assert_eq!(p[31], 0x01);
        assert_eq!(p[32], 0x03);
        assert_eq!(p[33], 0xab);
        assert_eq!(&p[34..36], &[0x00, 0x30]);
        assert_eq!(p[40], 0x3a);
        assert_eq!(p[44], 0x0f);
        assert_eq!(p[46], 0x01);
        assert_eq!(make_stagehand_transport_packet(&host(), 0x14, false, 1)[46], 0x00);
    }

    #[test]
    fn pref_write_packet_layout() {
        let p =
            make_stagehand_pref_write_packet(&host(), StagehandPreferences { on_air: Some(OnAirPref::On), quantize: Some(2) });
        assert_eq!(p.len(), 124);
        assert_eq!(p[10], 0x6b);
        assert_eq!(p[30], 0x03);
        assert_eq!(&p[31..37], &[0x01, 0x03, 0x3a, 0x00, 0x50, 0x01]);
        assert_eq!(p[44], 0x81);
        assert_eq!(p[60], 0x82);
        let off =
            make_stagehand_pref_write_packet(&host(), StagehandPreferences { on_air: Some(OnAirPref::Off), quantize: None });
        assert_eq!(off[44], 0x80);
        assert_eq!(off[60], 0x00);
        let none = make_stagehand_pref_write_packet(&host(), StagehandPreferences::default());
        assert_eq!(none[44], 0x00);
    }
}
