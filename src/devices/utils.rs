//! Announce packet parsing.

use std::net::Ipv4Addr;

use crate::constants::has_prolink_header;
use crate::types::{Device, DeviceType};
use crate::utils::string_from_nul_padded;
use crate::{Error, Result};

/// Converts an announce packet to a device object.
///
/// Returns `Ok(None)` for prolink packets that are not keep-alive (0x06)
/// announcements, and an error when the packet does not carry the prolink
/// header at all.
pub fn device_from_packet(packet: &[u8]) -> Result<Option<Device>> {
    if !has_prolink_header(packet) {
        return Err(Error::protocol("Announce packet does not start with expected header"));
    }

    if packet.get(0x0a) != Some(&0x06) {
        return Ok(None);
    }

    if packet.len() < 0x35 {
        return Err(Error::protocol(format!("Announce packet too short: {} bytes", packet.len())));
    }

    let name = string_from_nul_padded(&packet[0x0c..0x0c + 20]);

    let mut mac_addr = [0u8; 6];
    mac_addr.copy_from_slice(&packet[0x26..0x26 + 6]);

    let ip = Ipv4Addr::from(u32::from_be_bytes([packet[0x2c], packet[0x2d], packet[0x2e], packet[0x2f]]));

    Ok(Some(Device { name, id: packet[0x24], device_type: DeviceType::from_u8(packet[0x34]), mac_addr, ip, last_active: None }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::PROLINK_HEADER;

    #[test]
    fn fails_for_non_prolink_packet() {
        assert!(device_from_packet(&[]).is_err());
    }

    #[test]
    fn only_handles_announce_packets() {
        let mut packet = PROLINK_HEADER.to_vec();
        packet.push(0x05);
        assert!(device_from_packet(&packet).unwrap().is_none());
    }

    #[test]
    fn handles_a_real_announce_packet() {
        let packet = include_bytes!("../../tests/data/announce-cdj-2.dat");
        let device = device_from_packet(packet).unwrap().unwrap();
        assert_eq!(device.id, 2);
        assert_eq!(device.device_type, DeviceType::Cdj);
        assert_eq!(device.name, "CDJ-2000nexus");
        assert_eq!(device.ip, Ipv4Addr::new(10, 0, 0, 207));
        assert_eq!(device.mac_addr, [116, 94, 28, 87, 130, 216]);
    }
}
