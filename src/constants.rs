//! Protocol constants shared across the crate.

/// The default virtual CDJ ID to use.
///
/// Out of the 1-6 player range, so it can never collide with a real CDJ.
/// Remotedb metadata still works from here: the 1-6 restriction applies to the
/// device-ID byte inside remotedb messages, which `RemoteDatabase` picks
/// per-connection independently of the announced ID.
pub const DEFAULT_VCDJ_ID: u8 = 0x07;

/// The port on which devices on the prolink network announce themselves.
pub const ANNOUNCE_PORT: u16 = 50000;

/// The port on which devices on the prolink network send beat timing information.
pub const BEAT_PORT: u16 = 50001;

/// The port on which devices on the prolink network report their status.
pub const STATUS_PORT: u16 = 50002;

/// The amount of time in ms between sending each announcement packet.
pub const ANNOUNCE_INTERVAL_MS: u64 = 1500;

/// The interval in ms between startup stage packets (0x0a, 0x00, 0x02, 0x04).
pub const STARTUP_STAGE_INTERVAL_MS: u64 = 300;

/// All UDP packets on the PRO DJ LINK network start with this magic header.
pub const PROLINK_HEADER: [u8; 10] = [0x51, 0x73, 0x70, 0x74, 0x31, 0x57, 0x6d, 0x4a, 0x4f, 0x4c];

/// The name given to the Virtual CDJ device.
pub const VIRTUAL_CDJ_NAME: &str = "ProLink-Connect";

/// A string indicating the firmware version reported with status packets.
pub const VIRTUAL_CDJ_FIRMWARE: &str = "3.20";

/// CDJs use device IDs 1-6. Devices outside this range (e.g. Stagehand at 154)
/// should not be queried for media slots or databases as they may crash.
pub const MIN_CDJ_DEVICE_ID: u8 = 1;
/// See [`MIN_CDJ_DEVICE_ID`].
pub const MAX_CDJ_DEVICE_ID: u8 = 6;

/// True when the packet starts with the [`PROLINK_HEADER`] magic.
pub fn has_prolink_header(packet: &[u8]) -> bool {
    packet.len() >= PROLINK_HEADER.len() && packet[..PROLINK_HEADER.len()] == PROLINK_HEADER
}
