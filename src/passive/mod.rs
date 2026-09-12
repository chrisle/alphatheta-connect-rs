//! Passive mode: monitoring the Pro DJ Link network through packet capture,
//! without binding UDP ports or announcing a virtual CDJ.
//!
//! Unlike the active [`ProlinkNetwork`](crate::network::ProlinkNetwork),
//! passive mode:
//! - Does not bind to UDP ports (no conflicts with rekordbox)
//! - Does not announce a virtual CDJ (devices don't know we exist)
//! - Cannot send packets (no CDJ control, no media slot queries)
//! - Works with USB-connected devices (XDJ-AZ, XDJ-XZ)
//!
//! Requirements: root/sudo privileges for packet capture, libpcap (Linux)
//! or Npcap (Windows), and the `passive` cargo feature.

pub mod alphatheta;
pub mod devices;
pub mod localdb;
pub mod pcap_adapter;
pub mod position;
pub mod remotedb;
pub mod status;

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::localdb::DatabasePreference;
use crate::mixstatus::MixstatusProcessor;
use crate::utils::net::network_interfaces;
use crate::Result;

pub use alphatheta::{
    find_all_alphatheta_interfaces, find_alphatheta_interface, get_arp_cache_for_interface, AlphaThetaInterface, ConnectionType,
};
pub use devices::{PassiveDeviceManager, PassiveDeviceManagerConfig};
pub use localdb::PassiveLocalDatabase;
pub use pcap_adapter::{PacketInfo, PcapAdapter, PcapAdapterConfig};
pub use position::PassivePositionEmitter;
pub use remotedb::PassiveRemoteDatabase;
pub use status::PassiveStatusEmitter;

/// Configuration for [`PassiveProlinkNetwork`].
#[derive(Debug, Clone)]
pub struct PassiveNetworkConfig {
    /// Network interface name (e.g., 'en0', 'eth0', 'en15' for USB-connected devices).
    pub iface: String,
    /// Buffer size for packet capture in bytes. Default: 10 MB.
    pub buffer_size: Option<usize>,
    /// Time after which a device is considered disconnected. Default: 10 s.
    pub device_timeout: Option<Duration>,
    /// Database format preference for loading rekordbox databases.
    pub database_preference: DatabasePreference,
}

impl PassiveNetworkConfig {
    pub fn new(iface: impl Into<String>) -> Self {
        Self { iface: iface.into(), buffer_size: None, device_timeout: None, database_preference: DatabasePreference::Auto }
    }
}

/// A passive monitoring interface to the Pro DJ Link network using packet
/// capture.
#[derive(Clone)]
pub struct PassiveProlinkNetwork {
    adapter: PcapAdapter,
    device_manager: PassiveDeviceManager,
    status_emitter: PassiveStatusEmitter,
    position_emitter: PassivePositionEmitter,
    localdb: PassiveLocalDatabase,
    remotedb: Arc<Mutex<Option<PassiveRemoteDatabase>>>,
    mixstatus: Arc<Mutex<Option<MixstatusProcessor>>>,
}

impl PassiveProlinkNetwork {
    pub fn new(config: PassiveNetworkConfig) -> Self {
        let adapter = PcapAdapter::new(PcapAdapterConfig { iface: config.iface, buffer_size: config.buffer_size });

        let device_manager = PassiveDeviceManager::new(
            &adapter,
            config.device_timeout.map(|device_timeout| PassiveDeviceManagerConfig { device_timeout }),
        );

        let status_emitter = PassiveStatusEmitter::new(&adapter);
        let position_emitter = position::passive_position_emitter(&adapter);
        let localdb = PassiveLocalDatabase::new(device_manager.clone(), status_emitter.clone(), config.database_preference);

        Self {
            adapter,
            device_manager,
            status_emitter,
            position_emitter,
            localdb,
            remotedb: Arc::new(Mutex::new(None)),
            mixstatus: Arc::new(Mutex::new(None)),
        }
    }

    /// Start passive packet capture. Requires root/sudo privileges.
    pub fn start(&self) -> Result<()> {
        self.adapter.start()
    }

    /// Stop packet capture and clean up all resources.
    pub async fn stop(&self) {
        self.device_manager.stop();
        self.status_emitter.stop();
        self.position_emitter.stop();
        self.localdb.stop();
        let remotedb = self.remotedb.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(r) = remotedb {
            r.stop().await;
        }
        self.adapter.stop();
    }

    /// Check if packet capture is active.
    pub fn is_capturing(&self) -> bool {
        self.adapter.is_capturing()
    }

    /// Get the network interface being monitored.
    pub fn interface_name(&self) -> &str {
        self.adapter.interface_name()
    }

    /// Get the pcap adapter for advanced usage.
    pub fn adapter(&self) -> &PcapAdapter {
        &self.adapter
    }

    /// Tracks devices on the network by listening to announcement packets.
    pub fn device_manager(&self) -> &PassiveDeviceManager {
        &self.device_manager
    }

    /// Reports CDJ status updates received via packet capture.
    pub fn status_emitter(&self) -> &PassiveStatusEmitter {
        &self.status_emitter
    }

    /// Reports absolute playhead position updates from CDJ-3000+ devices.
    pub fn position_emitter(&self) -> &PassivePositionEmitter {
        &self.position_emitter
    }

    /// Provides access to rekordbox databases on devices using NFS (works
    /// without announcing a VCDJ).
    pub fn localdb(&self) -> &PassiveLocalDatabase {
        &self.localdb
    }

    /// Get (and initialize) the remote database service. Provides access to
    /// track metadata via RemoteDB queries.
    ///
    /// Note: this sends TCP packets to devices, so it's not fully "passive",
    /// but it avoids UDP announcements that would conflict with rekordbox.
    /// Useful for getting metadata for rekordbox Link tracks where NFS access
    /// is not available.
    pub fn remotedb(&self) -> PassiveRemoteDatabase {
        let mut slot = self.remotedb.lock().unwrap_or_else(|e| e.into_inner());
        slot.get_or_insert_with(|| PassiveRemoteDatabase::new(self.device_manager.clone(), None)).clone()
    }

    /// Get (and initialize) the mix status processor. Can be used to monitor
    /// the 'status' of devices on the network as a whole.
    pub fn mixstatus(&self) -> MixstatusProcessor {
        let mut slot = self.mixstatus.lock().unwrap_or_else(|e| e.into_inner());
        slot.get_or_insert_with(|| {
            let processor = MixstatusProcessor::new(None);
            let feed = processor.clone();
            let _listener = self.status_emitter.on_status(move |s| feed.handle_state(s));
            processor
        })
        .clone()
    }
}

/// Create and start a passive Pro DJ Link network monitor.
///
/// This is the primary entrypoint for passive mode. It captures Pro DJ Link
/// packets via pcap without binding to UDP ports or announcing a virtual CDJ.
pub fn bring_online_passive(config: PassiveNetworkConfig) -> Result<PassiveProlinkNetwork> {
    let network = PassiveProlinkNetwork::new(config);
    network.start()?;
    Ok(network)
}

/// A network interface that can be used for packet capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureInterface {
    pub name: String,
    pub address: Ipv4Addr,
    /// "Link-local (USB device?)" for 169.254.x.x addresses.
    pub description: Option<String>,
}

/// List available network interfaces that can be used for packet capture.
/// Useful for finding USB-connected DJ hardware interfaces.
pub fn list_interfaces() -> Vec<CaptureInterface> {
    network_interfaces()
        .into_iter()
        // Only include IPv4, non-internal interfaces
        .filter(|i| !i.internal)
        .map(|i| CaptureInterface {
            name: i.name,
            address: i.address,
            // USB-connected DJ hardware typically uses link-local addresses
            description: i.address.is_link_local().then(|| "Link-local (USB device?)".to_string()),
        })
        .collect()
}
