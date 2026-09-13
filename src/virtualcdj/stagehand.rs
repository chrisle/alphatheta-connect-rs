//! Posing as a Pioneer Stagehand iOS app device.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::RngExt;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::constants::{ANNOUNCE_PORT, PROLINK_HEADER};
use crate::logger::{noop_logger, SharedLogger};
use crate::types::{device_snapshot, Device, DeviceId, DeviceType, SharedDevice};
use crate::utils::udp::UdpFeed;
use crate::utils::{build_name, get_broadcast_address, InterfaceInfo};

pub const STAGEHAND_STARTUP_INTERVAL_MS: u64 = 305;
pub const STAGEHAND_KEEP_ALIVE_INTERVAL_MS: u64 = 2000;

/// Stages of the Stagehand join sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagehandStartupStage {
    InitialAnnounce,
    SecondStageClaim,
    KeepAlive,
}

/// The protocol-layer MAC a Stagehand device embeds in its 0x02 claim and 0x06
/// keep-alive: the AlphaTheta OUI (c8:3d:fc) followed by the low three bytes of
/// the interface's own MAC.
///
/// This must stay stable for the life of the host. A CDJ-3000 records a peer by
/// the identity in its claim, and once it holds a record for our IP it ignores
/// later claims from the same IP that carry a different MAC or device number:
/// the new peer never receives the unicast status/position stream. Observed on
/// emulated CDJ-3000 firmware 3.20: a freshly randomized identity on every
/// `connect()` was served once and then never again until the player rebooted,
/// while the previously registered identity kept being served immediately.
pub fn get_stagehand_mac(iface: &InterfaceInfo) -> [u8; 6] {
    [0xc8, 0x3d, 0xfc, iface.mac[3], iface.mac[4], iface.mac[5]]
}

/// Generates a random Stagehand device ID in the observed range of 141 to 211.
pub fn generate_stagehand_device_id() -> DeviceId {
    rand::rng().random_range(141..=211)
}

/// Constructs a virtual Stagehand Device.
///
/// - `iface`: the network interface to use
/// - `id`: the device ID to use (defaults to a random Stagehand ID)
/// - `name`: the device name (defaults to 'Stagehand')
/// - `mac_addr`: the protocol-layer MAC (defaults to one derived from `iface`)
pub fn get_virtual_stagehand(
    iface: &InterfaceInfo,
    id: Option<DeviceId>,
    name: Option<&str>,
    mac_addr: Option<[u8; 6]>,
) -> Device {
    Device {
        id: id.unwrap_or_else(generate_stagehand_device_id),
        name: name.unwrap_or("Stagehand").to_string(),
        device_type: DeviceType::Stagehand,
        ip: iface.address,
        mac_addr: mac_addr.unwrap_or_else(|| get_stagehand_mac(iface)),
        last_active: None,
    }
}

/// Build Stagehand stage 0x0a packet: Initial announcement. Sent 3 times at
/// 305ms intervals.
pub fn make_stagehand_0a_packet(device: &Device) -> Vec<u8> {
    let mut p = Vec::with_capacity(0x25);
    p.extend_from_slice(&PROLINK_HEADER); // 10 bytes
    p.extend_from_slice(&[0x0a, 0x00]); // 2 bytes packet type (0x0a)
    p.extend_from_slice(&build_name(device)); // 20 bytes device name
    p.extend_from_slice(&[0x01, 0x03]); // 2 bytes protocol / structure bytes
    p.extend_from_slice(&[0x00, 0x25]); // 2 bytes packet length (37)
    p.push(DeviceType::Stagehand.as_u8()); // 1 byte device type (0x05)
    p
}

/// Build Stagehand stage 0x02 packet: Second-stage device number claim. Sent
/// 3 times at 305ms intervals with counter N (1, 2, 3).
pub fn make_stagehand_02_packet(device: &Device, mac: &[u8; 6], counter: u8) -> Vec<u8> {
    let mut p = Vec::with_capacity(0x32);
    p.extend_from_slice(&PROLINK_HEADER); // 10 bytes
    p.extend_from_slice(&[0x02, 0x00]); // 2 bytes packet type (0x02)
    p.extend_from_slice(&build_name(device)); // 20 bytes device name
    p.extend_from_slice(&[0x01, 0x03]); // 2 bytes protocol / structure bytes
    p.extend_from_slice(&[0x00, 0x32]); // 2 bytes packet length (50)
    p.extend_from_slice(&device.ip.octets()); // 4 bytes IP
    p.extend_from_slice(mac); // 6 bytes identifier
    p.push(0x3a); // 1 byte constant
    p.push(counter); // 1 byte counter
    p.push(DeviceType::Stagehand.as_u8()); // 1 byte device type (0x05)
    p.push(0x01); // 1 byte constant
    p
}

/// Build Stagehand stage 0x06 packet: Keep-alive. Sent every 2.0s after
/// startup complete.
pub fn make_stagehand_06_packet(device: &Device, mac: &[u8; 6]) -> Vec<u8> {
    let mut p = Vec::with_capacity(0x36);
    p.extend_from_slice(&PROLINK_HEADER); // 10 bytes
    p.extend_from_slice(&[0x06, 0x00]); // 2 bytes packet type (0x06)
    p.extend_from_slice(&build_name(device)); // 20 bytes device name
    p.extend_from_slice(&[0x01, 0x03]); // 2 bytes protocol / structure bytes
    p.extend_from_slice(&[0x00, 0x36]); // 2 bytes packet length (54)
    p.push(device.id); // 1 byte device number
    p.push(0x01); // 1 byte constant
    p.extend_from_slice(mac); // 6 bytes identifier
    p.extend_from_slice(&device.ip.octets()); // 4 bytes IP
    p.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // 4 bytes constant
    p.push(DeviceType::Stagehand.as_u8()); // 1 byte device-type (0x05)
    p.push(0x20); // 1 byte trailing byte
    p
}

struct Inner {
    announce_feed: Arc<UdpFeed>,
    vcdj: SharedDevice,
    iface: InterfaceInfo,
    logger: SharedLogger,
    mac: [u8; 6],
    task: Mutex<Option<JoinHandle<()>>>,
    ready: watch::Sender<bool>,
}

/// Broadcasts the Stagehand join sequence, then keep-alives.
#[derive(Clone)]
pub struct StagehandAnnouncer {
    inner: Arc<Inner>,
}

impl StagehandAnnouncer {
    pub fn new(vcdj: SharedDevice, announce_feed: Arc<UdpFeed>, iface: InterfaceInfo, logger: Option<SharedLogger>) -> Self {
        let mac = device_snapshot(&vcdj).mac_addr;
        let (ready, _) = watch::channel(false);
        Self {
            inner: Arc::new(Inner {
                announce_feed,
                vcdj,
                iface,
                logger: logger.unwrap_or_else(noop_logger),
                mac,
                task: Mutex::new(None),
                ready,
            }),
        }
    }

    /// Resolves when the join sequence completes.
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
        let vcdj = device_snapshot(&self.inner.vcdj);
        self.inner.logger.info(&format!("Starting Stagehand announcer: device name \"{}\", ID {}", vcdj.name, vcdj.id));

        let mut task = self.inner.task.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = task.take() {
            t.abort();
        }
        let inner = Arc::clone(&self.inner);
        *task = Some(tokio::spawn(async move {
            let _ = inner.ready.send(false);
            for stage in [StagehandStartupStage::InitialAnnounce, StagehandStartupStage::SecondStageClaim] {
                for counter in 1..=3u8 {
                    let device = device_snapshot(&inner.vcdj);
                    let packet = match stage {
                        StagehandStartupStage::InitialAnnounce => {
                            inner.logger.debug(&format!("Stagehand sending stage 0x0a (announce) packet {counter}/3"));
                            make_stagehand_0a_packet(&device)
                        }
                        StagehandStartupStage::SecondStageClaim => {
                            inner.logger.debug(&format!("Stagehand sending stage 0x02 (claim) packet {counter}/3"));
                            make_stagehand_02_packet(&device, &inner.mac, counter)
                        }
                        StagehandStartupStage::KeepAlive => unreachable!(),
                    };
                    Self::send_packet(&inner, &packet).await;
                    tokio::time::sleep(Duration::from_millis(STAGEHAND_STARTUP_INTERVAL_MS)).await;
                }
            }

            inner.logger.info("Stagehand join sequence complete, transitioning to keep-alive");
            let _ = inner.ready.send(true);

            loop {
                let device = device_snapshot(&inner.vcdj);
                inner.logger.debug("Stagehand sending keep-alive packet");
                Self::send_packet(&inner, &make_stagehand_06_packet(&device, &inner.mac)).await;
                tokio::time::sleep(Duration::from_millis(STAGEHAND_KEEP_ALIVE_INTERVAL_MS)).await;
            }
        }));
    }

    async fn send_packet(inner: &Inner, packet: &[u8]) {
        let broadcast = get_broadcast_address(&inner.iface);
        if let Err(e) = inner.announce_feed.send_to(packet, broadcast, ANNOUNCE_PORT).await {
            inner.logger.debug(&format!("Stagehand broadcast to {broadcast} failed: {e}"));
        }
    }

    pub fn stop(&self) {
        self.inner.logger.info("Stopping Stagehand announcer");
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

    fn iface() -> InterfaceInfo {
        InterfaceInfo {
            name: "en0".into(),
            address: Ipv4Addr::new(10, 0, 0, 9),
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            mac: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
            internal: false,
        }
    }

    #[test]
    fn random_ids_are_in_range() {
        for _ in 0..50 {
            let id = generate_stagehand_device_id();
            assert!((141..=211).contains(&id));
        }
    }

    #[test]
    fn mac_is_derived_from_the_interface() {
        assert_eq!(get_stagehand_mac(&iface()), [0xc8, 0x3d, 0xfc, 0x33, 0x44, 0x55]);
    }

    #[test]
    fn mac_is_stable_for_the_same_interface() {
        assert_eq!(get_stagehand_mac(&iface()), get_stagehand_mac(&iface().clone()));
        assert_eq!(get_virtual_stagehand(&iface(), Some(150), None, None).mac_addr, get_stagehand_mac(&iface()));
    }

    #[test]
    fn stagehand_packet_layouts() {
        let d = stagehand();
        let p0a = make_stagehand_0a_packet(&d);
        assert_eq!(p0a.len(), 0x25);
        assert_eq!(&p0a[0x20..0x25], &[0x01, 0x03, 0x00, 0x25, 0x05]);

        let p02 = make_stagehand_02_packet(&d, &d.mac_addr, 2);
        assert_eq!(p02.len(), 0x32);
        assert_eq!(&p02[0x24..0x28], &[10, 0, 0, 9]);
        assert_eq!(&p02[0x2e..0x32], &[0x3a, 2, 0x05, 0x01]);

        let p06 = make_stagehand_06_packet(&d, &d.mac_addr);
        assert_eq!(p06.len(), 0x36);
        assert_eq!(p06[0x24], 154);
        assert_eq!(&p06[0x26..0x2c], &d.mac_addr);
        assert_eq!(&p06[0x30..0x36], &[0x01, 0x00, 0x00, 0x00, 0x05, 0x20]);
    }
}
