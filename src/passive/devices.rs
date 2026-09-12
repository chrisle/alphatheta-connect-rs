//! Device tracking from captured announce packets.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::constants::has_prolink_header;
use crate::emitter::{Emitter, Listener};
use crate::passive::pcap_adapter::{PacketInfo, PcapAdapter};
use crate::types::{Device, DeviceId, DeviceType};
use crate::utils::string_from_nul_padded;

/// Parse a device from an announce packet, using the source IP from the
/// packet info when the packet payload doesn't contain the full IP address.
///
/// This handles both standard and short-format announce packets (46 bytes)
/// that some devices like XDJ-XZ send.
pub fn device_from_packet_with_info(packet: &[u8], info: &PacketInfo) -> Option<Device> {
    if !has_prolink_header(packet) {
        return None;
    }

    // Check for stage 3 announce (type 0x06 at offset 0x0a)
    if packet.get(0x0a) != Some(&0x06) {
        return None;
    }

    if packet.len() < 0x2c {
        return None;
    }

    // Extract device name (20 bytes starting at offset 0x0c)
    let name = string_from_nul_padded(&packet[0x0c..0x0c + 20]);

    // Short-format packets (46 bytes) have different offsets
    let is_short_format = packet.len() < 0x35;

    let device_id = packet[0x24];
    let (device_type, mac_addr, ip) = if is_short_format {
        // Short format (46 bytes) - XDJ-XZ, etc. Short-format packets don't
        // have device type at the usual offset. Use the device ID heuristic:
        // 1-6 are CDJ slots, 17 is Rekordbox, 33+ are mixer
        let device_type = match device_id {
            1..=6 => DeviceType::Cdj,
            17 => DeviceType::Rekordbox,
            _ => DeviceType::Mixer,
        };
        let mut mac = [0u8; 6];
        for (i, b) in packet[0x26..packet.len().min(0x2c)].iter().enumerate() {
            mac[i] = *b;
        }
        // Use source IP from packet info (payload may be truncated)
        (device_type, mac, info.src_ipv4())
    } else {
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&packet[0x26..0x2c]);
        let ip = Ipv4Addr::from(u32::from_be_bytes([packet[0x2c], packet[0x2d], packet[0x2e], packet[0x2f]]));
        (DeviceType::from_u8(packet[0x34]), mac, ip)
    };

    Some(Device { name, id: device_id, device_type, mac_addr, ip, last_active: None })
}

/// Configuration for [`PassiveDeviceManager`].
#[derive(Debug, Clone)]
pub struct PassiveDeviceManagerConfig {
    /// Time after which a device is considered to have disconnected if it
    /// has not broadcast an announcement. Default: 10 seconds.
    pub device_timeout: Duration,
}

impl Default for PassiveDeviceManagerConfig {
    fn default() -> Self {
        Self { device_timeout: Duration::from_millis(10_000) }
    }
}

/// The upper bound to wait when looking for a device to be on the network
/// when using [`PassiveDeviceManager::get_device_ensured`].
pub const ENSURED_TIMEOUT: Duration = Duration::from_millis(2000);

struct Inner {
    config: RwLock<PassiveDeviceManagerConfig>,
    devices: RwLock<HashMap<DeviceId, Device>>,
    timeouts: Mutex<HashMap<DeviceId, JoinHandle<()>>>,
    connected: Emitter<Device>,
    disconnected: Emitter<Device>,
    announced: Emitter<Device>,
    feed_task: Mutex<Option<JoinHandle<()>>>,
}

/// Tracks devices on the Pro DJ Link network using passive packet capture
/// instead of UDP sockets. It provides the same API as the active
/// [`DeviceManager`](crate::devices::DeviceManager), making it easy to swap
/// between active and passive modes.
#[derive(Clone)]
pub struct PassiveDeviceManager {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PassiveDeviceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassiveDeviceManager").field("devices", &self.devices().len()).finish()
    }
}

impl PassiveDeviceManager {
    pub fn new(adapter: &PcapAdapter, config: Option<PassiveDeviceManagerConfig>) -> Self {
        let inner = Arc::new(Inner {
            config: RwLock::new(config.unwrap_or_default()),
            devices: RwLock::new(HashMap::new()),
            timeouts: Mutex::new(HashMap::new()),
            connected: Emitter::new(),
            disconnected: Emitter::new(),
            announced: Emitter::new(),
            feed_task: Mutex::new(None),
        });

        // Listen for announce packets from the pcap adapter
        let task = {
            let inner = Arc::clone(&inner);
            let mut rx = adapter.announce().subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(datagram) => Self::handle_announce(&inner, &datagram),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
        };
        *inner.feed_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);

        Self { inner }
    }

    /// Get active devices on the network.
    pub fn devices(&self) -> HashMap<DeviceId, Device> {
        self.inner.devices.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The device with the given id, if it is currently on the network.
    pub fn device(&self, id: DeviceId) -> Option<Device> {
        self.inner.devices.read().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
    }

    pub fn connected(&self) -> &Emitter<Device> {
        &self.inner.connected
    }

    pub fn disconnected(&self) -> &Emitter<Device> {
        &self.inner.disconnected
    }

    pub fn announced(&self) -> &Emitter<Device> {
        &self.inner.announced
    }

    pub fn on_connected<F: FnMut(Device) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.connected.on(f)
    }

    pub fn on_disconnected<F: FnMut(Device) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.disconnected.on(f)
    }

    pub fn on_announced<F: FnMut(Device) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.announced.on(f)
    }

    /// Waits for a specific device ID to appear on the network, with a
    /// configurable timeout, in which case it resolves with `None`.
    pub async fn get_device_ensured(&self, id: DeviceId, timeout: Option<Duration>) -> Option<Device> {
        let mut rx = self.inner.connected.subscribe();
        if let Some(existing) = self.device(id) {
            return Some(existing);
        }
        let wait = async {
            loop {
                match rx.recv().await {
                    Ok(device) if device.id == id => return Some(device),
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        };
        tokio::time::timeout(timeout.unwrap_or(ENSURED_TIMEOUT), wait).await.unwrap_or(None)
    }

    /// Reconfigure the device manager.
    pub fn reconfigure(&self, config: PassiveDeviceManagerConfig) {
        *self.inner.config.write().unwrap_or_else(|e| e.into_inner()) = config;
    }

    /// Stop listening to the pcap adapter and clean up timeouts.
    pub fn stop(&self) {
        if let Some(t) = self.inner.feed_task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            t.abort();
        }
        for (_, t) in self.inner.timeouts.lock().unwrap_or_else(|e| e.into_inner()).drain() {
            t.abort();
        }
    }

    fn handle_announce(inner: &Arc<Inner>, datagram: &PacketInfo) {
        // Ignore malformed packets
        let Some(device) = device_from_packet_with_info(&datagram.data, datagram) else {
            return;
        };

        let timeout = inner.config.read().unwrap_or_else(|e| e.into_inner()).device_timeout;

        // Device has not checked in before
        let is_new = {
            let mut devices = inner.devices.write().unwrap_or_else(|e| e.into_inner());
            match devices.entry(device.id) {
                std::collections::hash_map::Entry::Occupied(_) => false,
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(device.clone());
                    true
                }
            }
        };
        if is_new {
            inner.connected.emit(device.clone());
        }

        inner.announced.emit(device.clone());

        // Reset the device timeout handler
        let handle = {
            let inner = Arc::clone(inner);
            let device = device.clone();
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                inner.devices.write().unwrap_or_else(|e| e.into_inner()).remove(&device.id);
                inner.timeouts.lock().unwrap_or_else(|e| e.into_inner()).remove(&device.id);
                inner.disconnected.emit(device);
            })
        };
        if let Some(old) = inner.timeouts.lock().unwrap_or_else(|e| e.into_inner()).insert(device.id, handle) {
            old.abort();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(t) = self.feed_task.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            t.abort();
        }
        for (_, t) in self.timeouts.get_mut().unwrap_or_else(|e| e.into_inner()).drain() {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::PROLINK_HEADER;
    use std::net::{SocketAddr, SocketAddrV4};

    fn info(src: Ipv4Addr) -> PacketInfo {
        PacketInfo::new(Vec::new(), SocketAddr::V4(SocketAddrV4::new(src, 50000)))
    }

    #[test]
    fn parses_short_format_announce_with_source_ip() {
        let mut p = vec![0u8; 46];
        p[..10].copy_from_slice(&PROLINK_HEADER);
        p[0x0a] = 0x06;
        p[0x0c..0x12].copy_from_slice(b"XDJ-XZ");
        p[0x24] = 1;
        p[0x26..0x2c].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        let d = device_from_packet_with_info(&p, &info(Ipv4Addr::new(169, 254, 88, 83))).unwrap();
        assert_eq!(d.name, "XDJ-XZ");
        assert_eq!(d.id, 1);
        assert_eq!(d.device_type, DeviceType::Cdj);
        assert_eq!(d.ip, Ipv4Addr::new(169, 254, 88, 83));
        assert_eq!(d.mac_addr, [1, 2, 3, 4, 5, 6]);

        p[0x24] = 17;
        assert_eq!(device_from_packet_with_info(&p, &info(Ipv4Addr::LOCALHOST)).unwrap().device_type, DeviceType::Rekordbox);
        p[0x24] = 33;
        assert_eq!(device_from_packet_with_info(&p, &info(Ipv4Addr::LOCALHOST)).unwrap().device_type, DeviceType::Mixer);
    }

    #[test]
    fn parses_standard_announce_and_rejects_others() {
        let packet = include_bytes!("../../tests/data/announce-cdj-2.dat");
        let d = device_from_packet_with_info(packet, &info(Ipv4Addr::LOCALHOST)).unwrap();
        assert_eq!(d.id, 2);
        assert_eq!(d.ip, Ipv4Addr::new(10, 0, 0, 207));
        assert!(device_from_packet_with_info(b"garbage", &info(Ipv4Addr::LOCALHOST)).is_none());
        let mut p = PROLINK_HEADER.to_vec();
        p.push(0x05);
        assert!(device_from_packet_with_info(&p, &info(Ipv4Addr::LOCALHOST)).is_none());
    }
}
