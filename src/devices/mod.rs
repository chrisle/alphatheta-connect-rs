//! Device discovery: tracks devices that appear on the prolink network and
//! reports their lifecycle.

pub mod utils;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::constants::VIRTUAL_CDJ_NAME;
use crate::emitter::{Emitter, Listener};
use crate::types::{Device, DeviceId};
use crate::utils::udp::Datagram;

pub use utils::device_from_packet;

/// Device manager configuration.
#[derive(Debug, Clone)]
pub struct DeviceManagerConfig {
    /// Time after which a device is considered to have disconnected if it
    /// has not broadcast an announcement.
    ///
    /// Default: 10 seconds.
    pub device_timeout: Duration,
    /// The name of the virtual CDJ to filter out from device announcements.
    /// This prevents our own device from appearing in the device list.
    ///
    /// Default: [`VIRTUAL_CDJ_NAME`].
    pub vcdj_name: String,
}

impl Default for DeviceManagerConfig {
    fn default() -> Self {
        Self { device_timeout: Duration::from_millis(10_000), vcdj_name: VIRTUAL_CDJ_NAME.to_string() }
    }
}

/// The upper bound to wait when looking for a device to be on the network
/// when using [`DeviceManager::get_device_ensured`].
pub const ENSURED_TIMEOUT: Duration = Duration::from_millis(2000);

struct Inner {
    config: RwLock<DeviceManagerConfig>,
    /// The map of all active devices currently available on the network.
    devices: RwLock<HashMap<DeviceId, Device>>,
    /// Tracks device timeout handlers, as devices announce themselves these
    /// timeouts will be updated.
    timeouts: Mutex<HashMap<DeviceId, JoinHandle<()>>>,
    /// Fired when a new device becomes available on the network.
    connected: Emitter<Device>,
    /// Fired when a device has not announced itself on the network for the
    /// specified timeout.
    disconnected: Emitter<Device>,
    /// Fired every time the device announces itself on the network.
    announced: Emitter<Device>,
}

/// The device manager is responsible for tracking devices that appear on the
/// prolink network, providing an API to react to device lifecycle events as
/// they connect and disconnect from the network.
///
/// Cloning yields another handle on the same manager.
#[derive(Clone)]
pub struct DeviceManager {
    inner: Arc<Inner>,
    /// The task consuming the announce feed. Shared so that clones keep it alive.
    _feed_task: Arc<JoinHandle<()>>,
}

impl std::fmt::Debug for DeviceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceManager").field("devices", &self.devices().len()).finish()
    }
}

impl DeviceManager {
    /// Begin listening for device announcements on the given feed of announce
    /// datagrams (port 50000).
    pub fn new(announce: &Emitter<Datagram>, config: Option<DeviceManagerConfig>) -> Self {
        let inner = Arc::new(Inner {
            config: RwLock::new(config.unwrap_or_default()),
            devices: RwLock::new(HashMap::new()),
            timeouts: Mutex::new(HashMap::new()),
            connected: Emitter::new(),
            disconnected: Emitter::new(),
            announced: Emitter::new(),
        });

        let feed_task = {
            let inner = Arc::clone(&inner);
            let mut rx = announce.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(datagram) => Self::handle_announce(&inner, &datagram.data),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
        };

        Self { inner, _feed_task: Arc::new(feed_task) }
    }

    /// Get active devices on the network.
    pub fn devices(&self) -> HashMap<DeviceId, Device> {
        self.inner.devices.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The device with the given id, if it is currently on the network.
    pub fn device(&self, id: DeviceId) -> Option<Device> {
        self.inner.devices.read().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
    }

    /// Fired when a new device becomes available on the network.
    pub fn connected(&self) -> &Emitter<Device> {
        &self.inner.connected
    }

    /// Fired when a device has not announced itself for the configured timeout.
    pub fn disconnected(&self) -> &Emitter<Device> {
        &self.inner.disconnected
    }

    /// Fired every time a device announces itself on the network.
    pub fn announced(&self) -> &Emitter<Device> {
        &self.inner.announced
    }

    /// Register a callback for [`connected`](Self::connected).
    pub fn on_connected<F: FnMut(Device) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.connected.on(f)
    }

    /// Register a callback for [`disconnected`](Self::disconnected).
    pub fn on_disconnected<F: FnMut(Device) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.disconnected.on(f)
    }

    /// Register a callback for [`announced`](Self::announced).
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

        let timeout = timeout.unwrap_or(ENSURED_TIMEOUT);
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

        tokio::time::timeout(timeout, wait).await.unwrap_or(None)
    }

    /// Change the configuration. Only the fields given change.
    pub fn reconfigure(&self, device_timeout: Option<Duration>, vcdj_name: Option<String>) {
        let mut config = self.inner.config.write().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = device_timeout {
            config.device_timeout = t;
        }
        if let Some(n) = vcdj_name {
            config.vcdj_name = n;
        }
    }

    /// Feed an announce packet by hand (used by tests and by the passive
    /// capture path).
    pub fn handle_packet(&self, packet: &[u8]) {
        Self::handle_announce(&self.inner, packet);
    }

    fn handle_announce(inner: &Arc<Inner>, message: &[u8]) {
        let device = match device_from_packet(message) {
            Ok(Some(device)) => device,
            Ok(None) => return,
            Err(e) => {
                tracing::debug!(target: "alphatheta_connect", "ignoring announce packet: {e}");
                return;
            }
        };

        let (timeout, vcdj_name) = {
            let config = inner.config.read().unwrap_or_else(|e| e.into_inner());
            (config.device_timeout, config.vcdj_name.clone())
        };

        if device.name == vcdj_name {
            return;
        }

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
                Self::handle_disconnect(&inner, device);
            })
        };
        let mut timeouts = inner.timeouts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = timeouts.insert(device.id, handle) {
            old.abort();
        }
    }

    fn handle_disconnect(inner: &Arc<Inner>, removed: Device) {
        inner.devices.write().unwrap_or_else(|e| e.into_inner()).remove(&removed.id);
        inner.timeouts.lock().unwrap_or_else(|e| e.into_inner()).remove(&removed.id);
        inner.disconnected.emit(removed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn announce_packet() -> Vec<u8> {
        include_bytes!("../../tests/data/announce-cdj-2.dat").to_vec()
    }

    #[tokio::test]
    async fn tracks_devices_and_times_them_out() {
        let feed: Emitter<Datagram> = Emitter::new();
        let manager = DeviceManager::new(
            &feed,
            Some(DeviceManagerConfig { device_timeout: Duration::from_millis(50), ..Default::default() }),
        );
        let mut connected = manager.connected().subscribe();
        let mut disconnected = manager.disconnected().subscribe();

        let from = std::net::SocketAddr::new(Ipv4Addr::new(10, 0, 0, 207).into(), 50000);
        feed.emit(Datagram::new(announce_packet(), from));

        let device = tokio::time::timeout(Duration::from_secs(1), connected.recv()).await.unwrap().unwrap();
        assert_eq!(device.id, 2);
        assert_eq!(manager.devices().len(), 1);

        let gone = tokio::time::timeout(Duration::from_secs(1), disconnected.recv()).await.unwrap().unwrap();
        assert_eq!(gone.id, 2);
        assert!(manager.devices().is_empty());
    }

    #[tokio::test]
    async fn get_device_ensured_waits_and_times_out() {
        let feed: Emitter<Datagram> = Emitter::new();
        let manager = DeviceManager::new(&feed, None);
        assert!(manager.get_device_ensured(2, Some(Duration::from_millis(20))).await.is_none());

        let from = std::net::SocketAddr::new(Ipv4Addr::new(10, 0, 0, 207).into(), 50000);
        let m2 = manager.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            m2.handle_packet(&announce_packet());
        });
        let device = manager.get_device_ensured(2, Some(Duration::from_secs(1))).await;
        assert_eq!(device.map(|d| d.ip), Some(Ipv4Addr::new(10, 0, 0, 207)));
        let _ = from;
    }

    #[tokio::test]
    async fn ignores_our_own_name() {
        let feed: Emitter<Datagram> = Emitter::new();
        let manager =
            DeviceManager::new(&feed, Some(DeviceManagerConfig { vcdj_name: "CDJ-2000nexus".into(), ..Default::default() }));
        manager.handle_packet(&announce_packet());
        assert!(manager.devices().is_empty());
    }
}
