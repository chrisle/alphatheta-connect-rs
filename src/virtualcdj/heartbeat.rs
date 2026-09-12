//! The Stagehand unicast keep-alive.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::constants::{PROLINK_HEADER, STATUS_PORT};
use crate::devices::DeviceManager;
use crate::logger::{noop_logger, SharedLogger};
use crate::types::{device_snapshot, Device, DeviceType, SharedDevice};
use crate::utils::udp::UdpFeed;

/// Cadence of the Stagehand unicast keep-alive, in milliseconds.
///
/// The real iOS Stagehand app heartbeats every discovered player and the
/// mixer at ~4 Hz (250ms median inter-packet gap in captured traffic). This
/// is the stream that keeps AlphaTheta hardware unicasting live state (`0x39`
/// mixer fader/EQ, `0x58` VU on the mixer; `0x69` slim status, waveform
/// families on the CDJs) to our IP — subnet-broadcast presence alone (the
/// `0x06` keep-alive) is not enough to bootstrap it.
pub const STAGEHAND_HEARTBEAT_INTERVAL_MS: u64 = 250;

/// Builds a Stagehand name field for a unicast frame.
///
/// Unlike the 20-byte `build_name` used by the broadcast announce/claim
/// frames, unicast frames carry a 19-byte name field (offsets 11..29) with
/// the device type / model bytes beginning at offset 30. The name is ASCII,
/// NUL padded.
fn build_unicast_name(name: &str) -> [u8; 19] {
    let mut field = [0u8; 19];
    for (dst, src) in field.iter_mut().zip(name.bytes()) {
        *dst = src;
    }
    field
}

/// Build the Stagehand mixer keep-alive (`0x3a`, 40 bytes).
///
/// Verified byte-for-byte against the real iOS Stagehand app unicasting to a
/// DJM-A9 at 4 Hz (Phase 4/5 captures §4.14.2). The 10-byte trailer
/// `21 02 00 fe 00 04 00 1c 00 00` is constant across every observed sample —
/// byte 33 (`0xfe`) is a VCDJ sentinel, not our runtime device number, so no
/// per-device bytes vary here.
pub fn make_stagehand_mixer_heartbeat(vcdj: &Device) -> Vec<u8> {
    let mut p = Vec::with_capacity(40);
    p.extend_from_slice(&PROLINK_HEADER); // 0-9: magic
    p.push(0x3a); // 10: type 0x3a (mixer keep-alive)
    p.extend_from_slice(&build_unicast_name(&vcdj.name)); // 11-29: name (19 bytes)
    p.extend_from_slice(&[0x21, 0x02, 0x00, 0xfe, 0x00, 0x04, 0x00, 0x1c, 0x00, 0x00]); // 30-39: constant trailer
    p
}

/// Build the Stagehand player keep-alive (`0x68`, 36 bytes).
///
/// Verified byte-for-byte against the real iOS Stagehand app unicasting to a
/// CDJ-3000 at ~2-4 Hz. Body (offsets 30-35) is `03 01 00 3a 00 00`: `0x03`
/// persona/channel, `0x01` device subkind, `0x00` flag, `0x3a` Stagehand
/// model-code stamp, two reserved bytes. Constant across every observed
/// sample.
pub fn make_stagehand_player_heartbeat(vcdj: &Device) -> Vec<u8> {
    let mut p = Vec::with_capacity(36);
    p.extend_from_slice(&PROLINK_HEADER); // 0-9: magic
    p.push(0x68); // 10: type 0x68 (player keep-alive)
    p.extend_from_slice(&build_unicast_name(&vcdj.name)); // 11-29: name (19 bytes)
    p.extend_from_slice(&[0x03, 0x01, 0x00, 0x3a, 0x00, 0x00]); // 30-35: constant body
    p
}

struct Inner {
    status_feed: Arc<UdpFeed>,
    vcdj: SharedDevice,
    device_manager: DeviceManager,
    logger: SharedLogger,
    task: Mutex<Option<JoinHandle<()>>>,
}

/// Unicasts Stagehand keep-alive frames to every discovered player and mixer
/// so that AlphaTheta hardware begins (and keeps) pushing live state to our
/// IP.
///
/// The [`StagehandAnnouncer`](super::StagehandAnnouncer) makes us *visible*
/// on the network via subnet broadcast; this heartbeat is what makes hardware
/// actually *talk back*. It targets the mixer with `0x3a` and each CDJ with
/// `0x68`, mirroring the real iOS Stagehand app. Frames are sent from the
/// status socket (port 50002) to each device's port 50002, which is where
/// their unicast state replies land and where the status emitter listens for
/// them.
#[derive(Clone)]
pub struct StagehandHeartbeat {
    inner: Arc<Inner>,
}

impl StagehandHeartbeat {
    pub fn new(
        vcdj: SharedDevice,
        status_feed: Arc<UdpFeed>,
        device_manager: DeviceManager,
        logger: Option<SharedLogger>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                status_feed,
                vcdj,
                device_manager,
                logger: logger.unwrap_or_else(noop_logger),
                task: Mutex::new(None),
            }),
        }
    }

    pub fn start(&self) {
        let mut task = self.inner.task.lock().unwrap_or_else(|e| e.into_inner());
        if task.is_some() {
            return;
        }

        self.inner.logger.info("Starting Stagehand heartbeat (unicast keep-alive)");
        let inner = Arc::clone(&self.inner);
        *task = Some(tokio::spawn(async move {
            loop {
                Self::send_heartbeats(&inner).await;
                tokio::time::sleep(Duration::from_millis(STAGEHAND_HEARTBEAT_INTERVAL_MS)).await;
            }
        }));
    }

    async fn send_heartbeats(inner: &Inner) {
        let vcdj = device_snapshot(&inner.vcdj);
        for device in inner.device_manager.devices().into_values() {
            // Never heartbeat ourselves.
            if device.id == vcdj.id {
                continue;
            }

            let packet = match device.device_type {
                DeviceType::Mixer => make_stagehand_mixer_heartbeat(&vcdj),
                DeviceType::Cdj => make_stagehand_player_heartbeat(&vcdj),
                // Rekordbox / other Stagehand peers are not heartbeat targets.
                _ => continue,
            };

            if let Err(e) = inner.status_feed.send_to(&packet, device.ip, STATUS_PORT).await {
                inner.logger.debug(&format!("Stagehand heartbeat to {} ({}) failed: {e}", device.name, device.ip));
            }
        }
    }

    pub fn stop(&self) {
        self.inner.logger.info("Stopping Stagehand heartbeat");
        if let Some(t) = self.inner.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            t.abort();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(t) = self.task.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn stagehand() -> Device {
        Device::new("Stagehand", 154, DeviceType::Stagehand, [0xc8, 0x3d, 0xfc, 1, 2, 3], Ipv4Addr::new(10, 0, 0, 9))
    }

    #[test]
    fn mixer_heartbeat_is_40_bytes_with_constant_trailer() {
        let p = make_stagehand_mixer_heartbeat(&stagehand());
        assert_eq!(p.len(), 40);
        assert_eq!(p[10], 0x3a);
        assert_eq!(&p[11..20], b"Stagehand");
        assert_eq!(&p[30..40], &[0x21, 0x02, 0x00, 0xfe, 0x00, 0x04, 0x00, 0x1c, 0x00, 0x00]);
    }

    #[test]
    fn player_heartbeat_is_36_bytes() {
        let p = make_stagehand_player_heartbeat(&stagehand());
        assert_eq!(p.len(), 36);
        assert_eq!(p[10], 0x68);
        assert_eq!(&p[30..36], &[0x03, 0x01, 0x00, 0x3a, 0x00, 0x00]);
    }

    #[test]
    fn unicast_name_truncates_to_19_bytes() {
        let n = build_unicast_name("A-very-long-device-name-here");
        assert_eq!(&n[..], b"A-very-long-device-");
    }
}
