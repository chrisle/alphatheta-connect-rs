//! Bringing the prolink network online and connecting to it.

use std::sync::{Arc, Mutex, RwLock};

use crate::constants::{ANNOUNCE_PORT, BEAT_PORT, DEFAULT_VCDJ_ID, STATUS_PORT};
use crate::control::Control;
use crate::db::Database;
use crate::devices::DeviceManager;
use crate::localdb::{DatabasePreference, LocalDatabase};
use crate::logger::{noop_logger, SharedLogger};
use crate::mixstatus::MixstatusProcessor;
use crate::remotedb::RemoteDatabase;
use crate::status::position::PositionEmitter;
use crate::status::StatusEmitter;
use crate::types::{shared_device, NetworkState, SharedDevice};
use crate::utils::udp::UdpFeed;
use crate::utils::{get_matching_interface, InterfaceInfo};
use crate::virtualcdj::{
    generate_stagehand_device_id, get_virtual_cdj, get_virtual_stagehand, Announcer, StagehandAnnouncer, StagehandHeartbeat,
};
use crate::{Error, Result};

const CONNECT_ERROR_HELP: &str = "Network must be configured. Try using `autoconfig_from_peers` or `configure`";

/// How to join the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectMethod {
    /// Actively join as a Virtual CDJ player.
    #[default]
    Active,
    /// Actively join posing as a Pioneer Stagehand iOS app device.
    Stagehand,
}

/// Configuration for the network.
#[derive(Clone, Default)]
pub struct NetworkConfig {
    /// The network interface to listen for devices on the network over.
    /// Filled in by [`ProlinkNetwork::autoconfig_from_peers`] when not given.
    pub iface: Option<InterfaceInfo>,
    /// The ID of the virtual CDJ or Stagehand device to pose as.
    ///
    /// You will likely want to configure this to be > 6: if you choose an ID
    /// within the 1-6 range, no other CDJ may exist on the network using
    /// that ID — you CAN NOT have 6 CDJs if you're using one of their slots.
    ///
    /// This choice does NOT affect remotedb metadata (unanalyzed media, CD
    /// disc data, streaming tracks). CDJs restrict remotedb to a device-ID
    /// byte in the 1-6 range, but that byte lives inside the remotedb
    /// messages and is picked per-connection by `RemoteDatabase`,
    /// independent of the announced ID (hardware-verified on CDJ-3000,
    /// 2026-08-30).
    ///
    /// Note that rekordbox analyzed media connected to the CDJ is accessed
    /// out of band of the network's remote database protocol, and was never
    /// limited by that restriction either.
    pub vcdj_id: Option<u8>,
    /// The name to announce the virtual CDJ or Stagehand device as on the
    /// network. This name will appear in device lists on other Pro DJ Link
    /// equipment.
    ///
    /// Default: 'ProLink-Connect' (or 'Stagehand' when the connect method is
    /// Stagehand).
    pub vcdj_name: Option<String>,
    /// Enable full startup protocol for robust device negotiation. When
    /// enabled, the virtual CDJ will go through the complete startup sequence
    /// (stages 0x0a → 0x00 → 0x02 → 0x04 → 0x06) before regular keep-alive
    /// announcements.
    ///
    /// This is recommended for production setups with CDJ-3000 and DJM-V10
    /// hardware to ensure proper device discovery and network stability.
    pub full_startup: bool,
    /// Send announcer packets to Pioneer Stagehand devices.
    ///
    /// Stagehand is excluded from announcer packets by default because it has
    /// been observed to crash when it receives them. Enabling this also turns
    /// on the cold-start subnet broadcast, which would otherwise reach
    /// Stagehand the same way.
    ///
    /// Enable this only for diagnostic work (e.g. reproducing that crash),
    /// never in production.
    pub announce_to_stagehand: bool,
    /// Database format preference for loading rekordbox databases.
    pub database_preference: DatabasePreference,
    /// Logger instance for diagnostic output. If not provided, logging is
    /// silently discarded.
    pub logger: Option<SharedLogger>,
    /// Connection method to use.
    pub connect_method: ConnectMethod,
}

impl std::fmt::Debug for NetworkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkConfig")
            .field("iface", &self.iface)
            .field("vcdj_id", &self.vcdj_id)
            .field("vcdj_name", &self.vcdj_name)
            .field("full_startup", &self.full_startup)
            .field("announce_to_stagehand", &self.announce_to_stagehand)
            .field("database_preference", &self.database_preference)
            .field("connect_method", &self.connect_method)
            .finish()
    }
}

/// A partial configuration for [`ProlinkNetwork::configure`]: only the
/// fields given change.
#[derive(Clone, Default)]
pub struct NetworkConfigUpdate {
    pub iface: Option<InterfaceInfo>,
    pub vcdj_id: Option<u8>,
    pub vcdj_name: Option<String>,
    pub full_startup: Option<bool>,
    pub announce_to_stagehand: Option<bool>,
    pub database_preference: Option<DatabasePreference>,
    pub logger: Option<SharedLogger>,
    pub connect_method: Option<ConnectMethod>,
}

enum AnnouncerKind {
    Active(Announcer),
    Stagehand(StagehandAnnouncer),
}

impl AnnouncerKind {
    fn stop(&self) {
        match self {
            AnnouncerKind::Active(a) => a.stop(),
            AnnouncerKind::Stagehand(a) => a.stop(),
        }
    }

    async fn ready(&self) {
        match self {
            AnnouncerKind::Active(a) => a.ready().await,
            AnnouncerKind::Stagehand(a) => a.ready().await,
        }
    }
}

struct ConnectionService {
    announcer: AnnouncerKind,
    heartbeat: Option<StagehandHeartbeat>,
    control: Control,
    remotedb: RemoteDatabase,
    localdb: LocalDatabase,
    database: Database,
    vcdj: SharedDevice,
}

struct Inner {
    state: RwLock<NetworkState>,
    announce_feed: Arc<UdpFeed>,
    beat_feed: Arc<UdpFeed>,
    status_feed: Arc<UdpFeed>,
    device_manager: DeviceManager,
    status_emitter: StatusEmitter,
    position_emitter: PositionEmitter,
    logger: RwLock<SharedLogger>,
    config: RwLock<NetworkConfig>,
    connection: Mutex<Option<Arc<ConnectionService>>>,
    mixstatus: Mutex<Option<MixstatusProcessor>>,
}

/// Brings the Prolink network online.
///
/// This is the primary entrypoint for connecting to the prolink network. It
/// binds the announce, beat and status UDP sockets, which will FAIL if
/// rekordbox is running on the same computer, or a second instance of this
/// library is running on the same machine.
pub async fn bring_online(config: Option<NetworkConfig>) -> Result<ProlinkNetwork> {
    let config = config.unwrap_or_default();

    // Socket used to listen for devices on the network
    let announce_feed = Arc::new(UdpFeed::bind(ANNOUNCE_PORT).await?);
    // Socket used to listen for beat timing information
    let beat_feed = Arc::new(UdpFeed::bind(BEAT_PORT).await?);
    // Socket used to listen for status packets
    let status_feed = Arc::new(UdpFeed::bind(STATUS_PORT).await?);

    // Enable broadcast so the announcer can reach the whole subnet before any
    // devices have been discovered.
    announce_feed.set_broadcast(true)?;

    // Stagehand's experimental packet parsing (0x39 mixer state, 0x58 VU) is
    // gated to Stagehand mode only. In 'active'/passive mode these emitters
    // behave as they did before Stagehand was added.
    let stagehand_mode = config.connect_method == ConnectMethod::Stagehand;

    let device_manager = DeviceManager::new(announce_feed.packets(), None);
    let status_emitter = StatusEmitter::new(Arc::clone(&status_feed), stagehand_mode);
    let position_emitter = PositionEmitter::new(&beat_feed, stagehand_mode);

    Ok(ProlinkNetwork::new(config, announce_feed, beat_feed, status_feed, device_manager, status_emitter, position_emitter))
}

/// Brings the Prolink network online using the Pioneer Stagehand connection
/// method. This connects to the network as a non-player Stagehand device
/// using its abbreviated handshake.
pub async fn bring_online_stagehand(config: Option<NetworkConfig>) -> Result<ProlinkNetwork> {
    let mut config = config.unwrap_or_default();
    config.connect_method = ConnectMethod::Stagehand;
    bring_online(Some(config)).await
}

/// The prolink network. Cloning yields another handle on the same network.
#[derive(Clone)]
pub struct ProlinkNetwork {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ProlinkNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProlinkNetwork").field("state", &self.state()).finish()
    }
}

impl ProlinkNetwork {
    #[allow(clippy::too_many_arguments)]
    fn new(
        config: NetworkConfig,
        announce_feed: Arc<UdpFeed>,
        beat_feed: Arc<UdpFeed>,
        status_feed: Arc<UdpFeed>,
        device_manager: DeviceManager,
        status_emitter: StatusEmitter,
        position_emitter: PositionEmitter,
    ) -> Self {
        let logger = config.logger.clone().unwrap_or_else(noop_logger);
        Self {
            inner: Arc::new(Inner {
                // We always start online when constructing the network
                state: RwLock::new(NetworkState::Online),
                announce_feed,
                beat_feed,
                status_feed,
                device_manager,
                status_emitter,
                position_emitter,
                logger: RwLock::new(logger),
                config: RwLock::new(config),
                connection: Mutex::new(None),
                mixstatus: Mutex::new(None),
            }),
        }
    }

    fn logger(&self) -> SharedLogger {
        Arc::clone(&self.inner.logger.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// Configure / reconfigure the network. Only the fields given change.
    ///
    /// You may need to disconnect and re-connect the network after making a
    /// networking configuration change.
    pub fn configure(&self, update: NetworkConfigUpdate) {
        let mut config = self.inner.config.write().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = update.iface {
            config.iface = Some(v);
        }
        if let Some(v) = update.vcdj_id {
            config.vcdj_id = Some(v);
        }
        if let Some(v) = update.vcdj_name {
            config.vcdj_name = Some(v);
        }
        if let Some(v) = update.full_startup {
            config.full_startup = v;
        }
        if let Some(v) = update.announce_to_stagehand {
            config.announce_to_stagehand = v;
        }
        if let Some(v) = update.database_preference {
            config.database_preference = v;
        }
        if let Some(v) = update.connect_method {
            config.connect_method = v;
        }
        if let Some(v) = update.logger {
            config.logger = Some(Arc::clone(&v));
            *self.inner.logger.write().unwrap_or_else(|e| e.into_inner()) = v;
        }
    }

    /// The current configuration.
    pub fn config(&self) -> NetworkConfig {
        self.inner.config.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Wait for another device to show up on the network to determine which
    /// network interface to listen on.
    ///
    /// Defaults the Virtual CDJ ID to 7 (or a random Stagehand ID in
    /// Stagehand mode).
    pub async fn autoconfig_from_peers(&self) -> Result<()> {
        let mut rx = self.inner.device_manager.connected().subscribe();

        // wait for first device to appear on the network
        let first_device = if let Some(existing) = self.inner.device_manager.devices().into_values().next() {
            existing
        } else {
            loop {
                match rx.recv().await {
                    Ok(device) => break device,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(Error::State("device manager closed".into()))
                    }
                }
            }
        };

        let iface = get_matching_interface(first_device.ip).ok_or_else(|| {
            Error::State(format!("Unable to determine network interface for device {} at {}", first_device.name, first_device.ip))
        })?;

        let mut config = self.inner.config.write().unwrap_or_else(|e| e.into_inner());
        let default_id =
            if config.connect_method == ConnectMethod::Stagehand { generate_stagehand_device_id() } else { DEFAULT_VCDJ_ID };
        config.vcdj_id = Some(default_id);
        config.iface = Some(iface);
        Ok(())
    }

    /// Connect to the network.
    ///
    /// The network must first have been configured (either with
    /// [`autoconfig_from_peers`](Self::autoconfig_from_peers) or manual
    /// configuration). This will then initialize all the network services.
    pub async fn connect(&self) -> Result<()> {
        let config = self.config();
        let Some(iface) = config.iface.clone() else {
            return Err(Error::State(CONNECT_ERROR_HELP.into()));
        };
        let logger = self.logger();

        let connect_method = config.connect_method;

        // Pick the device number once and keep it. A CDJ-3000 that already
        // holds a record for our IP ignores a re-claim under a different
        // number, so a disconnect/connect cycle must present the same
        // identity (see `get_stagehand_mac`).
        let vcdj_id = config.vcdj_id.unwrap_or_else(|| {
            if connect_method == ConnectMethod::Stagehand {
                generate_stagehand_device_id()
            } else {
                DEFAULT_VCDJ_ID
            }
        });
        self.inner.config.write().unwrap_or_else(|e| e.into_inner()).vcdj_id = Some(vcdj_id);
        let vcdj_name =
            config.vcdj_name.clone().or_else(|| (connect_method == ConnectMethod::Stagehand).then(|| "Stagehand".to_string()));

        // Create VCDJ or Stagehand device
        let vcdj = if connect_method == ConnectMethod::Stagehand {
            get_virtual_stagehand(&iface, Some(vcdj_id), vcdj_name.as_deref(), None)
        } else {
            get_virtual_cdj(&iface, vcdj_id, vcdj_name.as_deref())
        };
        let vcdj = shared_device(vcdj);

        // Update device manager to filter out our device name
        if let Some(name) = &vcdj_name {
            self.inner.device_manager.reconfigure(None, Some(name.clone()));
        }

        // Start announcing
        let announcer = if connect_method == ConnectMethod::Stagehand {
            let a = StagehandAnnouncer::new(
                Arc::clone(&vcdj),
                Arc::clone(&self.inner.announce_feed),
                iface.clone(),
                Some(Arc::clone(&logger)),
            );
            a.start();
            AnnouncerKind::Stagehand(a)
        } else {
            let a = Announcer::new(
                Arc::clone(&vcdj),
                Arc::clone(&self.inner.announce_feed),
                self.inner.device_manager.clone(),
                iface.clone(),
                config.full_startup,
                config.announce_to_stagehand,
                Some(Arc::clone(&logger)),
            );
            a.start();
            AnnouncerKind::Active(a)
        };

        // In Stagehand mode, unicast keep-alives to discovered players and
        // the mixer so they push live state to us (mixer 0x39/0x58, CDJ
        // 0x69/waveforms). Broadcast presence alone does not bootstrap that
        // stream.
        let heartbeat = (connect_method == ConnectMethod::Stagehand).then(|| {
            let h = StagehandHeartbeat::new(
                Arc::clone(&vcdj),
                Arc::clone(&self.inner.status_feed),
                self.inner.device_manager.clone(),
                Some(Arc::clone(&logger)),
            );
            h.start();
            h
        });

        // Create remote and local databases
        let remotedb = RemoteDatabase::new(self.inner.device_manager.clone(), Arc::clone(&vcdj));
        let localdb = LocalDatabase::new(
            Arc::clone(&vcdj),
            self.inner.device_manager.clone(),
            self.inner.status_emitter.clone(),
            config.database_preference,
        );

        // Create unified database
        let database =
            Database::new(localdb.clone(), remotedb.clone(), self.inner.device_manager.clone(), Some(Arc::clone(&logger)));

        // Create controller service
        let control = Control::new(Arc::clone(&self.inner.beat_feed), Arc::clone(&vcdj));

        *self.inner.state.write().unwrap_or_else(|e| e.into_inner()) = NetworkState::Connected;
        *self.inner.connection.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Arc::new(ConnectionService { announcer, heartbeat, control, remotedb, localdb, database, vcdj }));

        Ok(())
    }

    /// Disconnect from the network: stop announcing and drop the database
    /// connections to every device. Call [`close`](Self::close) afterwards
    /// to close the sockets.
    pub async fn disconnect(&self) -> Result<()> {
        if self.config().iface.is_none() {
            return Err(Error::State(CONNECT_ERROR_HELP.into()));
        }

        let connection = self.inner.connection.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(conn) = connection {
            // Stop announcing ourself
            conn.announcer.stop();
            if let Some(h) = &conn.heartbeat {
                h.stop();
            }

            // Disconnect devices from the remote and local databases
            for device in self.inner.device_manager.devices().into_values() {
                let _ = conn.remotedb.disconnect_from_device(&device).await;
                conn.localdb.disconnect_for_device(&device);
            }
        }

        *self.inner.state.write().unwrap_or_else(|e| e.into_inner()) = NetworkState::Online;
        Ok(())
    }

    /// Close the UDP sockets.
    pub fn close(&self) {
        self.inner.announce_feed.close();
        self.inner.status_feed.close();
        self.inner.beat_feed.close();
        *self.inner.state.write().unwrap_or_else(|e| e.into_inner()) = NetworkState::Offline;
    }

    /// Get the current [`NetworkState`] of the network.
    ///
    /// When the network is Online you may use the device manager to list and
    /// react to devices on the network. Once the network is Connected you may
    /// use the status emitter to listen for player status events, query the
    /// media databases of devices using the db service (or specifically
    /// query the localdb or remotedb).
    pub fn state(&self) -> NetworkState {
        *self.inner.state.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Check if the network has been configured. You cannot connect to the
    /// network until it has been configured.
    pub fn is_configured(&self) -> bool {
        self.config().iface.is_some()
    }

    /// True once connected, when the service accessors return `Some`.
    pub fn is_connected(&self) -> bool {
        self.state() == NetworkState::Connected
    }

    fn connection(&self) -> Option<Arc<ConnectionService>> {
        self.inner.connection.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The virtual device announced on the network, once connected.
    pub fn virtual_device(&self) -> Option<SharedDevice> {
        self.connection().map(|c| Arc::clone(&c.vcdj))
    }

    /// Get the [`DeviceManager`] service. This service is used to monitor
    /// and react to devices connecting and disconnecting from the prolink
    /// network.
    pub fn device_manager(&self) -> &DeviceManager {
        &self.inner.device_manager
    }

    /// Get the [`StatusEmitter`] service. This service is used to monitor
    /// status updates on each CDJ.
    ///
    /// Even though the status emitter service does not need to wait for the
    /// network to be Connected, it does not make sense to use it unless it
    /// is, so this is `None` until then.
    pub fn status_emitter(&self) -> Option<&StatusEmitter> {
        self.is_connected().then_some(&self.inner.status_emitter)
    }

    /// Get the [`PositionEmitter`] service. This service provides events with
    /// absolute playhead position updates from CDJ-3000+ devices.
    ///
    /// Position packets are sent approximately every 30ms while a track is
    /// loaded, providing precise position tracking independent of beat
    /// grids. This enables accurate timecode/video sync even during
    /// scratching, reverse play, and loops.
    pub fn position_emitter(&self) -> Option<&PositionEmitter> {
        self.is_connected().then_some(&self.inner.position_emitter)
    }

    /// Get the [`Control`] service. This service can be used to control the
    /// play state of CDJs on the network.
    pub fn control(&self) -> Option<Control> {
        self.connection().map(|c| c.control.clone())
    }

    /// Get the [`Database`] service. This service is used to retrieve
    /// metadata and listings from devices on the network, automatically
    /// choosing the best strategy to access the data.
    pub fn db(&self) -> Option<Database> {
        self.connection().map(|c| c.database.clone())
    }

    /// Get the [`LocalDatabase`] service. This service is used to query and
    /// sync metadata that is downloaded directly from the rekordbox database
    /// present on media connected to the CDJs.
    pub fn localdb(&self) -> Option<LocalDatabase> {
        self.connection().map(|c| c.localdb.clone())
    }

    /// Get the [`RemoteDatabase`] service. This service is used to query
    /// metadata directly from the database service running on rekordbox and
    /// the CDJs themselves.
    pub fn remotedb(&self) -> Option<RemoteDatabase> {
        self.connection().map(|c| c.remotedb.clone())
    }

    /// Resolves when the full startup protocol completes. Resolves
    /// immediately if full startup is disabled or not connected.
    pub async fn startup_ready(&self) {
        if let Some(conn) = self.connection() {
            conn.announcer.ready().await;
        }
    }

    /// Get (and initialize) the [`MixstatusProcessor`] service. This service
    /// can be used to monitor the 'status' of devices on the network as a
    /// whole.
    pub fn mixstatus(&self) -> Option<MixstatusProcessor> {
        self.connection()?;

        // Delay initialization of the mixstatus processor so that we don't
        // consume status events unless we actually want to.
        let mut slot = self.inner.mixstatus.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            let processor = MixstatusProcessor::new(None);
            let feed = processor.clone();
            // Keep feeding for the network's lifetime; the listener is
            // detached on purpose.
            let _listener = self.inner.status_emitter.on_status(move |s| feed.handle_state(s));
            *slot = Some(processor);
        }
        slot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::device_snapshot;
    use crate::virtualcdj::get_stagehand_mac;
    use std::net::Ipv4Addr;

    fn iface() -> InterfaceInfo {
        InterfaceInfo {
            name: "en0".into(),
            address: Ipv4Addr::new(192, 0, 2, 10),
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            mac: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
            internal: false,
        }
    }

    /// A Stagehand network on ephemeral sockets: nothing binds the real Pro
    /// DJ Link ports, and the interface sits in TEST-NET-1 so the announcer's
    /// broadcasts have nowhere to go.
    async fn stagehand_network() -> ProlinkNetwork {
        let announce_feed = Arc::new(UdpFeed::bind(0).await.expect("bind announce"));
        let beat_feed = Arc::new(UdpFeed::bind(0).await.expect("bind beat"));
        let status_feed = Arc::new(UdpFeed::bind(0).await.expect("bind status"));

        let config = NetworkConfig { iface: Some(iface()), connect_method: ConnectMethod::Stagehand, ..NetworkConfig::default() };

        let device_manager = DeviceManager::new(announce_feed.packets(), None);
        let status_emitter = StatusEmitter::new(Arc::clone(&status_feed), true);
        let position_emitter = PositionEmitter::new(&beat_feed, true);

        ProlinkNetwork::new(config, announce_feed, beat_feed, status_feed, device_manager, status_emitter, position_emitter)
    }

    /// A CDJ-3000 keeps one record per peer IP and ignores a later claim from
    /// that IP under a different MAC or device number, so a
    /// disconnect/connect cycle must re-present the identity it first
    /// announced.
    #[tokio::test]
    async fn stagehand_identity_survives_a_reconnect() {
        let network = stagehand_network().await;

        network.connect().await.expect("first connect");
        let first = device_snapshot(&network.virtual_device().expect("virtual device"));
        network.disconnect().await.expect("disconnect");

        network.connect().await.expect("second connect");
        let second = device_snapshot(&network.virtual_device().expect("virtual device"));
        network.disconnect().await.expect("disconnect");
        network.close();

        assert_eq!(first.id, second.id);
        assert_eq!(first.mac_addr, second.mac_addr);
        // The claim MAC is derived from the interface, not randomized.
        assert_eq!(first.mac_addr, get_stagehand_mac(&iface()));
        // ...and the chosen device number is kept in the config.
        assert_eq!(network.config().vcdj_id, Some(first.id));
        assert!((141..=211).contains(&first.id));
    }
}
