//! The virtual CDJ: the device this library announces as, and the announcer
//! that keeps it on the network.

pub mod device_id;
pub mod heartbeat;
pub mod stagehand;

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::constants::{
    has_prolink_header, ANNOUNCE_INTERVAL_MS, ANNOUNCE_PORT, PROLINK_HEADER, STARTUP_STAGE_INTERVAL_MS, VIRTUAL_CDJ_FIRMWARE,
    VIRTUAL_CDJ_NAME,
};
use crate::devices::DeviceManager;
use crate::logger::{noop_logger, SharedLogger};
use crate::types::{device_snapshot, Device, DeviceId, DeviceType, SharedDevice};
use crate::utils::udp::UdpFeed;
use crate::utils::{build_name, get_broadcast_address, InterfaceInfo};

pub use device_id::*;
pub use heartbeat::*;
pub use stagehand::*;

/// Constructs a virtual CDJ Device.
///
/// - `iface`: the network interface to use
/// - `id`: the device ID to use
/// - `name`: optional custom name (defaults to [`VIRTUAL_CDJ_NAME`])
pub fn get_virtual_cdj(iface: &InterfaceInfo, id: DeviceId, name: Option<&str>) -> Device {
    Device {
        id,
        name: name.unwrap_or(VIRTUAL_CDJ_NAME).to_string(),
        device_type: DeviceType::Cdj,
        ip: iface.address,
        mac_addr: iface.mac,
        last_active: None,
    }
}

/// Returns a mostly empty-state status packet. This is currently used to
/// report the virtual CDJs status, which *seems* to be required for the CDJ
/// to send metadata about some unanalyzed mp3 files.
pub fn make_status_packet(device: &Device) -> Vec<u8> {
    // NOTE: It seems that byte 0x68 and 0x75 MUST be 1 in order for the CDJ to
    //       correctly report mp3 metadata (again, only for some files).
    //       See https://github.com/brunchboy/dysentery/issues/15
    // NOTE: Byte 0xb6 MUST be 1 in order for the CDJ to not think that our
    //       device is "running an older firmware"
    #[rustfmt::skip]
    let mut b: Vec<u8> = vec![
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x03, 0x00, 0x00, 0xf8, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x04, 0x04, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9c, 0xff, 0xfe, 0x00, 0x10, 0x00, 0x00,
        0x7f, 0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff,
        0xff, 0xff, 0xff, 0xff, 0x01, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0f, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    // The following items get replaced in this format:
    //
    //  - 0x00: 10 byte header
    //  - 0x0B: 20 byte device name
    //  - 0x21: 01 byte device ID
    //  - 0x24: 01 byte device ID
    //  - 0x7C: 04 byte firmware string
    //
    // Upstream writes the header at 0x0b (where the name then overwrites it),
    // leaving the magic bytes out of the packet. The packet is not sent by
    // any code path, so this port follows the documented layout instead.
    b[..10].copy_from_slice(&PROLINK_HEADER);
    b[0x0b..0x0b + 20].copy_from_slice(&build_name(device));
    b[0x21] = device.id;
    b[0x24] = device.id;
    let fw = VIRTUAL_CDJ_FIRMWARE.as_bytes();
    b[0x7c..0x7c + fw.len()].copy_from_slice(fw);

    b
}

/// Constructs the announce packet that is sent on the prolink network to
/// announce a device's existence.
pub fn make_announce_packet(device: &Device) -> Vec<u8> {
    // The packet below is constructed in the following format:
    //
    //  - 0x00: 10 byte header
    //  - 0x0A: 02 byte announce packet type
    //  - 0x0c: 20 byte device name
    //  - 0x20: 02 byte unknown
    //  - 0x22: 02 byte packet length
    //  - 0x24: 01 byte for the player ID
    //  - 0x25: 01 byte for the player type
    //  - 0x26: 06 byte mac address
    //  - 0x2C: 04 byte IP address
    //  - 0x30: 04 byte unknown
    //  - 0x34: 01 byte for the player type
    //  - 0x35: 01 byte final padding
    let mut p = Vec::with_capacity(0x36);
    p.extend_from_slice(&PROLINK_HEADER);
    p.extend_from_slice(&[0x06, 0x00]);
    p.extend_from_slice(&build_name(device));
    // unknown padding bytes
    p.extend_from_slice(&[0x01, 0x02]);
    p.extend_from_slice(&[0x00, 0x36]);
    p.push(device.id);
    p.push(device.device_type.as_u8());
    p.extend_from_slice(&device.mac_addr);
    p.extend_from_slice(&device.ip.octets());
    // Updated on 2024-04-27 to be compatible with CDJ-3000 players that use
    // player number 5 or 6.
    p.extend_from_slice(&[0x02, 0x00, 0x00, 0x00]);
    p.push(device.device_type.as_u8());
    p.push(0x64);
    p
}

/// Startup stages for the full startup protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupStage {
    /// Initial announcement packets (0x0a).
    InitialAnnounce,
    /// First-stage device number claim (0x00).
    FirstStageClaim,
    /// Second-stage device number claim (0x02).
    SecondStageClaim,
    /// Final-stage device number claim (0x04).
    FinalStageClaim,
    /// Keep-alive packets (0x06).
    KeepAlive,
}

/// Build stage 0x0a packet: Initial announcement (CDJ-3000 compatible).
/// Sent 3 times at 300ms intervals.
pub fn make_stage_0a_packet(device: &Device) -> Vec<u8> {
    let high = device.id >= 5;
    let mut p = Vec::with_capacity(0x27);
    p.extend_from_slice(&PROLINK_HEADER);
    p.extend_from_slice(&[0x0a, 0x00]);
    p.extend_from_slice(&build_name(device));
    p.push(0x01);
    p.push(if high { 0x04 } else { 0x02 });
    p.extend_from_slice(&[0x00, if high { 0x26 } else { 0x25 }]);
    p.push(0x01);
    if high {
        p.push(0x40);
    }
    p
}

/// Build stage 0x00 packet: First-stage device number claim (CDJ-3000
/// compatible). Sent 3 times at 300ms intervals with counter N (1, 2, 3).
pub fn make_stage_00_packet(device: &Device, counter: u8) -> Vec<u8> {
    let high = device.id >= 5;
    let mut p = Vec::with_capacity(0x2c);
    p.extend_from_slice(&PROLINK_HEADER);
    p.extend_from_slice(&[0x00, 0x00]);
    p.extend_from_slice(&build_name(device));
    p.push(0x01);
    p.push(if high { 0x03 } else { 0x02 });
    p.extend_from_slice(&[0x00, 0x2c]);
    p.push(counter);
    p.push(0x01);
    p.extend_from_slice(&device.mac_addr);
    p
}

/// Build stage 0x02 packet: Second-stage device number claim (CDJ-3000
/// compatible). Sent 3 times at 300ms intervals with counter N (1, 2, 3).
pub fn make_stage_02_packet(device: &Device, counter: u8) -> Vec<u8> {
    let high = device.id >= 5;
    let mut p = Vec::with_capacity(0x32);
    p.extend_from_slice(&PROLINK_HEADER);
    p.extend_from_slice(&[0x02, 0x00]);
    p.extend_from_slice(&build_name(device));
    p.push(0x01);
    p.push(if high { 0x03 } else { 0x02 });
    p.extend_from_slice(&[0x00, 0x32]);
    p.extend_from_slice(&device.ip.octets());
    p.extend_from_slice(&device.mac_addr);
    p.push(device.id);
    p.push(counter);
    // auto-assign mode
    p.extend_from_slice(&[0x30, 0x01, 0x01]);
    p
}

/// Build stage 0x04 packet: Final-stage device number claim (CDJ-3000
/// compatible). Sent 1-3 times at 300ms intervals with counter N.
pub fn make_stage_04_packet(device: &Device, counter: u8) -> Vec<u8> {
    let high = device.id >= 5;
    let mut p = Vec::with_capacity(0x26);
    p.extend_from_slice(&PROLINK_HEADER);
    p.extend_from_slice(&[0x04, 0x00]);
    p.extend_from_slice(&build_name(device));
    p.push(0x01);
    p.push(if high { 0x03 } else { 0x02 });
    p.extend_from_slice(&[0x00, 0x26]);
    p.push(device.id);
    p.push(counter);
    p
}

/// Build stage 0x06 packet: Keep-alive (CDJ-3000 compatible). Sent every
/// 1.5s after startup complete.
pub fn make_stage_06_packet(device: &Device, peer_count: u8) -> Vec<u8> {
    let high = device.id >= 5;
    let mut p = Vec::with_capacity(0x36);
    p.extend_from_slice(&PROLINK_HEADER);
    p.extend_from_slice(&[0x06, 0x00]);
    p.extend_from_slice(&build_name(device));
    p.extend_from_slice(&[0x01, 0x02]);
    p.extend_from_slice(&[0x00, 0x36]);
    p.push(device.id);
    p.push(0x01);
    p.extend_from_slice(&device.mac_addr);
    p.extend_from_slice(&device.ip.octets());
    p.push(0x30);
    p.push(peer_count);
    p.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    p.push(if high { 0x64 } else { 0x00 });
    p
}

/// Check if a packet is a channel conflict (0x08) packet.
/// Returns the conflicting device ID if this is a conflict packet.
pub fn parse_conflict_packet(packet: &[u8]) -> Option<DeviceId> {
    // Conflict packets are 0x29 (41) bytes long with packet type 0x08 at byte 0x0a
    if packet.len() != 0x29 {
        return None;
    }
    if !has_prolink_header(packet) {
        return None;
    }
    if packet[0x0a] != 0x08 {
        return None;
    }
    // Extract the device ID being defended (byte 0x24)
    Some(packet[0x24])
}

struct AnnouncerState {
    /// The task driving startup / keep-alive packets.
    timer: Option<JoinHandle<()>>,
    /// Listener for incoming conflict packets.
    conflict_listener: Option<JoinHandle<()>>,
}

struct AnnouncerInner {
    /// The announce socket to use to make the announcements.
    feed: Arc<UdpFeed>,
    /// The device manager service used to determine which devices to announce
    /// ourselves to.
    device_manager: DeviceManager,
    /// The virtual CDJ device to announce.
    vcdj: SharedDevice,
    /// Network interface for the virtual CDJ.
    iface: InterfaceInfo,
    /// Whether to use full startup protocol.
    full_startup: bool,
    /// Whether to send announcer packets to Pioneer Stagehand devices, which
    /// are normally excluded because they crash on our packets.
    announce_to_stagehand: bool,
    logger: SharedLogger,
    state: Mutex<AnnouncerState>,
    /// Becomes true once the (full) startup protocol has completed.
    ready: watch::Sender<bool>,
}

/// The announcer service is used to report our fake CDJ to the prolink
/// network, as if it was a real CDJ.
#[derive(Clone)]
pub struct Announcer {
    inner: Arc<AnnouncerInner>,
}

impl Announcer {
    pub fn new(
        vcdj: SharedDevice,
        announce_feed: Arc<UdpFeed>,
        device_manager: DeviceManager,
        iface: InterfaceInfo,
        full_startup: bool,
        announce_to_stagehand: bool,
        logger: Option<SharedLogger>,
    ) -> Self {
        let (ready, _) = watch::channel(!full_startup);
        Self {
            inner: Arc::new(AnnouncerInner {
                feed: announce_feed,
                device_manager,
                vcdj,
                iface,
                full_startup,
                announce_to_stagehand,
                logger: logger.unwrap_or_else(noop_logger),
                state: Mutex::new(AnnouncerState { timer: None, conflict_listener: None }),
                ready,
            }),
        }
    }

    /// Resolves when the startup protocol completes. Resolves immediately if
    /// full startup is disabled.
    pub async fn ready(&self) {
        let mut rx = self.inner.ready.subscribe();
        if *rx.borrow() {
            return;
        }
        while rx.changed().await.is_ok() {
            if *rx.borrow() {
                return;
            }
        }
    }

    /// True once the announcer is in keep-alive mode.
    pub fn is_ready(&self) -> bool {
        *self.inner.ready.borrow()
    }

    pub fn start(&self) {
        self.arm_conflict_listener();
        Self::spawn_timer(&self.inner);
    }

    /// Listen for another device defending the ID we are announcing as.
    ///
    /// The listener stays armed for the announcer's whole life, not just
    /// during startup. A player that powers on after us can claim our ID at
    /// any point, and until it is answered both devices fight over the slot —
    /// which is what knocks a live CDJ off the network.
    fn arm_conflict_listener(&self) {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.conflict_listener.is_some() {
            return;
        }

        let inner = Arc::clone(&self.inner);
        let mut rx = self.inner.feed.packets().subscribe();
        state.conflict_listener = Some(tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(datagram) => {
                        let conflict_id = parse_conflict_packet(&datagram.data);
                        let our_id = device_snapshot(&inner.vcdj).id;
                        if conflict_id == Some(our_id) {
                            inner.logger.warn(&format!("Device ID {our_id} is already in use. Finding alternative..."));
                            Self::handle_conflict(&inner);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
    }

    /// Handle a device ID conflict by finding an available ID and restarting.
    fn handle_conflict(inner: &Arc<AnnouncerInner>) {
        // Stop announcing under the contested ID
        Self::clear_timer(inner);

        // Stay in whichever range we were already in, so a conflict on player
        // 5 looks for another free player number first rather than jumping
        // the whole device out of the range the DJ configured it into.
        //
        // The mixer decides which of those numbers are actually safe: a player
        // can only be assigned a number its mixer has a channel for, so
        // anything above that is out of reach of the rig that just contested
        // us.
        let devices: Vec<Device> = inner.device_manager.devices().into_values().collect();
        let likes: Vec<DeviceLike> = devices.iter().map(DeviceLike::from).collect();
        let current_id = device_snapshot(&inner.vcdj).id;
        let new_id = pick_available_device_id(
            devices.iter().map(|d| d.id),
            PickDeviceIdOptions {
                prefer_player_range: current_id <= REMOTEDB_MAX_DEVICE_ID,
                player_ceiling: player_number_ceiling(&likes),
            },
        );

        let Some(new_id) = new_id else {
            inner.logger.error("No available device IDs. All 32 slots are occupied.");
            Self::stop_inner(inner);
            return;
        };

        inner.logger.info(&format!("Switching to device ID {new_id}"));

        // Mutate in place rather than building a replacement device. The
        // remotedb, localdb and control services were all handed this same
        // shared device when the network connected and read `.id` when they
        // build packets, so swapping the reference here would leave us
        // announcing as one ID and querying as another.
        inner.vcdj.write().unwrap_or_else(|e| e.into_inner()).id = new_id;

        // Re-announce under the new ID
        if inner.full_startup {
            let _ = inner.ready.send(false);
        }
        Self::spawn_timer(inner);
    }

    /// Clear whichever task is currently driving announcements.
    fn clear_timer(inner: &AnnouncerInner) {
        let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = state.timer.take() {
            t.abort();
        }
    }

    /// Sends a packet by unicast to every discovered device.
    ///
    /// Pioneer Stagehand is excluded by default (it crashes on our packets)
    /// unless announce_to_stagehand is enabled.
    ///
    /// When nothing has been discovered yet, falls back to a subnet broadcast
    /// — but only when announce_to_stagehand is enabled, because a broadcast
    /// reaches every host including Stagehand. In normal operation the
    /// virtual CDJ is found via unicast replies to peers' own announce
    /// broadcasts, so skipping this cold-start broadcast costs nothing.
    async fn send_packet(inner: &AnnouncerInner, packet: &[u8]) {
        let devices: Vec<Device> = inner
            .device_manager
            .devices()
            .into_values()
            .filter(|d| inner.announce_to_stagehand || !d.name.to_lowercase().contains("stagehand"))
            .collect();

        for device in &devices {
            if let Err(e) = inner.feed.send_to(packet, device.ip, ANNOUNCE_PORT).await {
                inner.logger.debug(&format!("Announce to {} ({}) failed: {e}", device.name, device.ip));
            }
        }

        // Cold-start discovery broadcast — gated; see the method doc above.
        if devices.is_empty() && inner.announce_to_stagehand {
            let broadcast: Ipv4Addr = get_broadcast_address(&inner.iface);
            if let Err(e) = inner.feed.send_to(packet, broadcast, ANNOUNCE_PORT).await {
                inner.logger.debug(&format!("Announce broadcast to {broadcast} failed: {e}"));
            }
        }
    }

    /// Spawn the task that walks the startup stages (when enabled) and then
    /// sends keep-alives forever.
    fn spawn_timer(inner: &Arc<AnnouncerInner>) {
        Self::clear_timer(inner);
        let task_inner = Arc::clone(inner);
        let handle = tokio::spawn(async move {
            let inner = task_inner;
            if inner.full_startup {
                let stages = [
                    StartupStage::InitialAnnounce,
                    StartupStage::FirstStageClaim,
                    StartupStage::SecondStageClaim,
                    StartupStage::FinalStageClaim,
                ];
                for stage in stages {
                    for counter in 1..=3u8 {
                        let device = device_snapshot(&inner.vcdj);
                        let packet = match stage {
                            StartupStage::InitialAnnounce => make_stage_0a_packet(&device),
                            StartupStage::FirstStageClaim => make_stage_00_packet(&device, counter),
                            StartupStage::SecondStageClaim => make_stage_02_packet(&device, counter),
                            StartupStage::FinalStageClaim => make_stage_04_packet(&device, counter),
                            StartupStage::KeepAlive => unreachable!(),
                        };
                        Self::send_packet(&inner, &packet).await;
                        tokio::time::sleep(Duration::from_millis(STARTUP_STAGE_INTERVAL_MS)).await;
                    }
                }
            }

            let _ = inner.ready.send(true);

            loop {
                let device = device_snapshot(&inner.vcdj);
                // +1 for ourselves
                let peer_count = (inner.device_manager.devices().len() + 1).min(255) as u8;
                let packet =
                    if inner.full_startup { make_stage_06_packet(&device, peer_count) } else { make_announce_packet(&device) };
                Self::send_packet(&inner, &packet).await;
                tokio::time::sleep(Duration::from_millis(ANNOUNCE_INTERVAL_MS)).await;
            }
        });

        inner.state.lock().unwrap_or_else(|e| e.into_inner()).timer = Some(handle);
    }

    fn stop_inner(inner: &AnnouncerInner) {
        let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = state.timer.take() {
            t.abort();
        }
        if let Some(l) = state.conflict_listener.take() {
            l.abort();
        }
    }

    pub fn stop(&self) {
        Self::stop_inner(&self.inner);
    }
}

impl Drop for AnnouncerInner {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = state.timer.take() {
            t.abort();
        }
        if let Some(l) = state.conflict_listener.take() {
            l.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: u8) -> Device {
        Device::new("CDJ-test", id, DeviceType::Cdj, [1, 2, 3, 4, 5, 6], Ipv4Addr::new(10, 0, 0, 1))
    }

    #[test]
    fn announce_packet_layout() {
        let p = make_announce_packet(&device(7));
        assert_eq!(p.len(), 0x36);
        assert_eq!(&p[..10], &PROLINK_HEADER);
        assert_eq!(p[0x0a], 0x06);
        assert_eq!(&p[0x0c..0x14], b"CDJ-test");
        assert_eq!(&p[0x22..0x24], &[0x00, 0x36]);
        assert_eq!(p[0x24], 7);
        assert_eq!(p[0x25], 0x01);
        assert_eq!(&p[0x26..0x2c], &[1, 2, 3, 4, 5, 6]);
        assert_eq!(&p[0x2c..0x30], &[10, 0, 0, 1]);
        assert_eq!(p[0x34], 0x01);
        assert_eq!(p[0x35], 0x64);
        // It parses back as the same device.
        let parsed = crate::devices::device_from_packet(&p).unwrap().unwrap();
        assert_eq!(parsed.id, 7);
        assert_eq!(parsed.ip, Ipv4Addr::new(10, 0, 0, 1));
    }

    #[test]
    fn stage_packets_follow_player_number() {
        let low = make_stage_0a_packet(&device(2));
        assert_eq!(low.len(), 0x25);
        assert_eq!(low[0x21], 0x02);
        let high = make_stage_0a_packet(&device(5));
        assert_eq!(high.len(), 0x26);
        assert_eq!(high[0x21], 0x04);
        assert_eq!(high[0x25], 0x40);

        let s00 = make_stage_00_packet(&device(5), 2);
        assert_eq!(s00.len(), 0x2c);
        assert_eq!(s00[0x24], 2);
        assert_eq!(&s00[0x26..0x2c], &[1, 2, 3, 4, 5, 6]);

        let s02 = make_stage_02_packet(&device(2), 3);
        // Upstream builds 51 bytes while the length field says 0x32; kept as is.
        assert_eq!(s02.len(), 51);
        assert_eq!(&s02[0x24..0x28], &[10, 0, 0, 1]);
        assert_eq!(s02[0x2e], 2);
        assert_eq!(s02[0x2f], 3);
        assert_eq!(&s02[0x30..0x33], &[0x30, 0x01, 0x01]);

        let s04 = make_stage_04_packet(&device(2), 1);
        assert_eq!(s04.len(), 0x26);
        assert_eq!(s04[0x24], 2);
        assert_eq!(s04[0x25], 1);

        let s06 = make_stage_06_packet(&device(5), 3);
        // 55 bytes on the wire (the length field says 0x36), as upstream.
        assert_eq!(s06.len(), 0x37);
        assert_eq!(s06[0x24], 5);
        assert_eq!(s06[0x31], 3);
        assert_eq!(s06[0x36], 0x64);
        assert_eq!(make_stage_06_packet(&device(2), 3)[0x36], 0x00);
    }

    #[test]
    fn conflict_packet_detection() {
        let mut p = vec![0u8; 0x29];
        p[..10].copy_from_slice(&PROLINK_HEADER);
        p[0x0a] = 0x08;
        p[0x24] = 5;
        assert_eq!(parse_conflict_packet(&p), Some(5));
        p[0x0a] = 0x06;
        assert_eq!(parse_conflict_packet(&p), None);
        assert_eq!(parse_conflict_packet(&p[..0x28]), None);
    }

    #[test]
    fn status_packet_layout() {
        let p = make_status_packet(&device(7));
        assert_eq!(&p[..10], &PROLINK_HEADER);
        assert_eq!(p[0x0a], 0x0a);
        assert_eq!(&p[0x0b..0x13], b"CDJ-test");
        assert_eq!(p[0x21], 7);
        assert_eq!(p[0x24], 7);
        assert_eq!(&p[0x7c..0x80], b"3.20");
        assert_eq!(p[0x68], 1);
        assert_eq!(p[0x75], 1);
        assert_eq!(p[0xb6], 1);
    }
}
