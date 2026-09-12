//! The media slot query request packet.

use crate::constants::PROLINK_HEADER;
use crate::types::{Device, MediaSlot};
use crate::utils::build_name;

/// Get information about the media connected to the specified slot on the
/// device.
///
/// - `host_device`: the device asking for media info.
/// - `device`: the target device we'll be querying for details of its media slot.
/// - `slot`: the specific slot.
pub fn make_media_slot_request(host_device: &Device, device: &Device, slot: MediaSlot) -> Vec<u8> {
    let mut p = Vec::with_capacity(0x30);
    p.extend_from_slice(&PROLINK_HEADER);
    p.push(0x05);
    p.extend_from_slice(&build_name(host_device));
    p.extend_from_slice(&[0x01, 0x00]);
    p.push(host_device.id);
    p.extend_from_slice(&[0x00, 0x0c]);
    p.extend_from_slice(&host_device.ip.octets());
    p.extend_from_slice(&[0x00, 0x00, 0x00, device.id]);
    p.extend_from_slice(&[0x00, 0x00, 0x00, slot.as_u8()]);
    p
}
