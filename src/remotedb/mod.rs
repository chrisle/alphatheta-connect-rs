//! The remote database: querying the database service running on rekordbox
//! and the CDJs themselves over TCP.

pub mod constants;
pub mod fields;
pub mod message;
pub mod queries;
pub mod utils;

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::devices::DeviceManager;
use crate::entities::{CueAndLoop, Track};
use crate::types::{device_snapshot, Device, DeviceId, DeviceType, MediaSlot, SharedDevice, TrackType};
use crate::virtualcdj::device_id::{pick_remote_db_query_id, QueryIdDeviceLike, REMOTEDB_MAX_DEVICE_ID};
use crate::{Error, Result};

use constants::REMOTEDB_SERVER_QUERY_PORT;
use fields::{read_field, Field, FieldType};
use message::{control_request, response_type as response, Message, MessageType};

pub use queries::PlaylistResult;

/// How long the initial TCP connect and handshake may take.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(10_000);

/// Menu target specifies where a menu should be "rendered". This differs
/// based on the request being made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuTarget {
    Main,
}

impl MenuTarget {
    pub const fn as_u8(self) -> u8 {
        match self {
            MenuTarget::Main => 0x01,
        }
    }
}

/// Used to specify where to lookup data when making queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryDescriptor {
    pub menu_target: MenuTarget,
    pub track_slot: MediaSlot,
    pub track_type: TrackType,
}

/// Used internally when making queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupDescriptor {
    pub menu_target: MenuTarget,
    pub track_slot: MediaSlot,
    pub track_type: TrackType,
    pub target_device: Device,
    pub host_device: Device,
}

/// Queries the remote device for the port that the remote database server is
/// listening on for requests.
pub async fn get_remote_db_server_port(device_ip: Ipv4Addr) -> Result<u16> {
    let addr = SocketAddr::V4(SocketAddrV4::new(device_ip, REMOTEDB_SERVER_QUERY_PORT));
    let mut conn = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await??;

    // Magic request packet asking the device to report its remoteDB port
    let mut data = vec![0x00, 0x00, 0x00, 0x0f];
    data.extend_from_slice(b"RemoteDBServer");
    data.push(0x00);

    conn.write_all(&data).await?;

    let mut resp = [0u8; 2];
    tokio::time::timeout(CONNECT_TIMEOUT, conn.read_exact(&mut resp)).await??;

    Ok(u16::from_be_bytes(resp))
}

/// Manages a connection to a single device.
pub struct Connection {
    socket: Mutex<TcpStream>,
    tx_id: AtomicU32,
    pub device: Device,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").field("device", &self.device.id).finish()
    }
}

impl Connection {
    pub fn new(device: Device, socket: TcpStream) -> Self {
        Self { socket: Mutex::new(socket), tx_id: AtomicU32::new(0), device }
    }

    /// Send a message, stamping it with the next transaction id.
    pub async fn write_message(&self, mut message: Message) -> Result<()> {
        message.transaction_id = Some(self.tx_id.fetch_add(1, Ordering::SeqCst) + 1);
        let mut socket = self.socket.lock().await;
        socket.write_all(&message.to_bytes()).await?;
        Ok(())
    }

    /// Read the next message, requiring it to be of type `expect`.
    pub async fn read_message(&self, expect: MessageType) -> Result<Message> {
        let mut socket = self.socket.lock().await;
        Message::from_stream(&mut *socket, expect).await
    }

    pub async fn close(&self) {
        let mut socket = self.socket.lock().await;
        let _ = socket.shutdown().await;
    }
}

/// The query interface for one device. Every query runs under the device's
/// lock, so queries on a device are serialised.
#[derive(Clone)]
pub struct QueryInterface {
    conn: Arc<Connection>,
    host_device: Device,
    lock: Arc<Mutex<()>>,
}

impl std::fmt::Debug for QueryInterface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryInterface").field("device", &self.conn.device.id).finish()
    }
}

macro_rules! query_method {
    ($(#[$meta:meta])* $name:ident -> $ret:ty) => {
        $(#[$meta])*
        pub async fn $name(&self, descriptor: &QueryDescriptor, track_id: u32) -> Result<$ret> {
            let lookup = self.lookup(descriptor);
            let _guard = self.lock.lock().await;
            queries::$name(&self.conn, &lookup, track_id).await
        }
    };
}

impl QueryInterface {
    pub fn new(conn: Arc<Connection>, lock: Arc<Mutex<()>>, host_device: Device) -> Self {
        Self { conn, lock, host_device }
    }

    /// The device this interface queries.
    pub fn device(&self) -> &Device {
        &self.conn.device
    }

    fn lookup(&self, d: &QueryDescriptor) -> LookupDescriptor {
        LookupDescriptor {
            menu_target: d.menu_target,
            track_slot: d.track_slot,
            track_type: d.track_type,
            host_device: self.host_device.clone(),
            target_device: self.conn.device.clone(),
        }
    }

    query_method!(
        /// Lookup track metadata from rekordbox and coerce it into a Track entity.
        get_metadata -> Track
    );
    query_method!(
        /// Lookup generic metadata for an unanalyzed track.
        get_generic_metadata -> Track
    );
    query_method!(
        /// Lookup the beatgrid for the specified track id.
        get_beatgrid -> crate::types::BeatGrid
    );
    query_method!(
        /// Lookup the waveform preview for the specified track id.
        get_waveform_preview -> crate::types::WaveformPreview
    );
    query_method!(
        /// Lookup the detailed waveform for the specified track id.
        get_waveform_detailed -> crate::types::WaveformDetailed
    );
    query_method!(
        /// Lookup the HD (nexus2) waveform for the specified track id.
        get_waveform_hd -> crate::types::WaveformHD
    );
    query_method!(
        /// Lookup the [hot]cue points and [hot]loops for a track.
        get_cue_and_loops -> Vec<CueAndLoop>
    );
    query_method!(
        /// Lookup the "advanced" (nexus2) [hot]cue points and [hot]loops for a track.
        get_cue_and_loops_adv -> Vec<CueAndLoop>
    );
    query_method!(
        /// Lookup the track information, currently just returns the track path.
        get_track_info -> String
    );

    /// Lookup the artwork image given the artwork id obtained from a track.
    pub async fn get_artwork(&self, descriptor: &QueryDescriptor, artwork_id: u32) -> Result<Vec<u8>> {
        let lookup = self.lookup(descriptor);
        let _guard = self.lock.lock().await;
        queries::get_artwork(&self.conn, &lookup, artwork_id).await
    }

    /// Lookup playlist entries. See [`queries::get_playlist`].
    pub async fn get_playlist(
        &self,
        descriptor: &QueryDescriptor,
        id: Option<u32>,
        is_folder_request: bool,
    ) -> Result<PlaylistResult> {
        let lookup = self.lookup(descriptor);
        let _guard = self.lock.lock().await;
        queries::get_playlist(&self.conn, &lookup, id, is_folder_request).await
    }
}

struct RemoteInner {
    host_device: SharedDevice,
    device_manager: DeviceManager,
    /// Active device connection map, with the device ID each connection
    /// introduced itself with. Queries on that connection must carry the same
    /// ID.
    connections: std::sync::Mutex<HashMap<DeviceId, (Arc<Connection>, DeviceId)>>,
    /// Locks for each device when locating the connection.
    device_locks: std::sync::Mutex<HashMap<DeviceId, Arc<Mutex<()>>>>,
    /// Whether to substitute an in-range query ID when the host device sits
    /// outside 1-6. Disabled only by protocol probes that need to observe how
    /// a device treats the raw announced ID.
    pick_query_id: bool,
}

/// Service that maintains remote database connections with devices on the
/// network.
#[derive(Clone)]
pub struct RemoteDatabase {
    inner: Arc<RemoteInner>,
}

impl RemoteDatabase {
    pub fn new(device_manager: DeviceManager, host_device: SharedDevice) -> Self {
        Self::with_options(device_manager, host_device, true)
    }

    /// `pick_query_id`: whether to substitute an in-range query ID when the
    /// host device sits outside 1-6.
    pub fn with_options(device_manager: DeviceManager, host_device: SharedDevice, pick_query_id: bool) -> Self {
        Self {
            inner: Arc::new(RemoteInner {
                host_device,
                device_manager,
                connections: std::sync::Mutex::new(HashMap::new()),
                device_locks: std::sync::Mutex::new(HashMap::new()),
                pick_query_id,
            }),
        }
    }

    /// The device this service belongs to — the virtual CDJ announced on the
    /// network. Note that this is NOT necessarily the ID carried inside
    /// remotedb messages: CDJs only answer queries whose in-protocol device-ID
    /// byte is 1-6, so when this device sits outside that range (announced
    /// above the player range to avoid collisions), each connection picks an
    /// in-range query ID via [`pick_remote_db_query_id`] instead.
    pub fn host_device(&self) -> Device {
        device_snapshot(&self.inner.host_device)
    }

    /// The device ID to introduce ourselves with, and to carry in every
    /// query, on a connection to the given device.
    fn query_id_for(&self, device: &Device) -> DeviceId {
        let host_id = self.host_device().id;

        // An announced ID already inside the answered range keeps working
        // exactly as it always has. Rekordbox (which numbers itself far above
        // 6) answers queries regardless, so only CDJ targets need an in-range
        // stand-in.
        if !self.inner.pick_query_id || host_id <= REMOTEDB_MAX_DEVICE_ID || device.device_type != DeviceType::Cdj {
            return host_id;
        }

        let devices = self.inner.device_manager.devices();
        pick_remote_db_query_id(device.id, devices.values().map(QueryIdDeviceLike::from))
    }

    fn lock_for(&self, id: DeviceId) -> Arc<Mutex<()>> {
        let mut locks = self.inner.device_locks.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(locks.entry(id).or_insert_with(|| Arc::new(Mutex::new(()))))
    }

    /// Open a connection to the specified device for querying.
    pub async fn connect_to_device(&self, device: &Device) -> Result<()> {
        let db_port = get_remote_db_server_port(device.ip).await?;

        let addr = SocketAddr::V4(SocketAddrV4::new(device.ip, db_port));
        // A connection timeout prevents hanging forever
        let mut socket = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| Error::Timeout(format!("RemoteDB connection to {addr} timed out")))??;
        socket.set_nodelay(true)?;

        let handshake = async {
            // Send required preamble to open communications with the device
            socket.write_all(&Field::UInt32(0x01).to_bytes()).await?;

            // Read the response. It should be a UInt32 field with the value
            // 0x01. There is some kind of problem if not.
            let data = read_field(&mut socket, FieldType::UInt32).await?;
            if data.as_number() != Some(0x01) {
                return Err(Error::RemoteDb(format!("Expected 0x01 during preamble handshake. Got {:?}", data.as_number())));
            }

            // Send introduction message to set context for querying
            let query_id = self.query_id_for(device);
            let intro =
                Message::with_transaction(0xffff_fffe, control_request::INTRODUCE, vec![Field::UInt32(u32::from(query_id))]);
            socket.write_all(&intro.to_bytes()).await?;
            let resp = Message::from_stream(&mut socket, response::SUCCESS).await?;
            if resp.message_type != response::SUCCESS {
                return Err(Error::RemoteDb(format!("Failed to introduce self to device ID: {}", device.id)));
            }
            Ok(query_id)
        };

        let query_id = tokio::time::timeout(CONNECT_TIMEOUT, handshake)
            .await
            .map_err(|_| Error::Timeout(format!("RemoteDB handshake with {addr} timed out")))??;

        self.inner
            .connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device.id, (Arc::new(Connection::new(device.clone(), socket)), query_id));
        Ok(())
    }

    /// Disconnect from the specified device.
    pub async fn disconnect_from_device(&self, device: &Device) -> Result<()> {
        let conn = self.inner.connections.lock().unwrap_or_else(|e| e.into_inner()).remove(&device.id);
        let Some((conn, _)) = conn else {
            return Ok(());
        };

        let goodbye = Message::with_transaction(0xffff_fffe, control_request::DISCONNECT, vec![]);
        let write = conn.write_message(goodbye).await;
        conn.close().await;
        write
    }

    /// Gets the remote database query interface for the given device.
    ///
    /// If we have not already established a connection with the specified
    /// device, we will attempt to first connect.
    ///
    /// Returns `None` if the device is not on the network.
    pub async fn get(&self, device_id: DeviceId) -> Result<Option<QueryInterface>> {
        let Some(device) = self.inner.device_manager.device(device_id) else {
            return Ok(None);
        };

        let lock = self.lock_for(device.id);
        let guard = lock.lock().await;

        let existing = self.inner.connections.lock().unwrap_or_else(|e| e.into_inner()).get(&device_id).cloned();
        let (conn, query_id) = match existing {
            Some(c) => c,
            None => {
                self.connect_to_device(&device).await?;
                self.inner
                    .connections
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&device_id)
                    .cloned()
                    .ok_or_else(|| Error::RemoteDb("connection vanished after connect".into()))?
            }
        };
        drop(guard);

        // NOTE: We pass the same lock we use for this device to the query
        // interface to ensure all query interfaces use the same lock.
        //
        // Queries must carry the same device ID the connection introduced
        // itself with, which is not always the announced host ID.
        let mut host = self.host_device();
        host.id = query_id;

        Ok(Some(QueryInterface::new(conn, lock, host)))
    }

    /// Drop the cached connection for a device (after a failed query, so the
    /// next call reconnects).
    pub fn forget(&self, device_id: DeviceId) {
        self.inner.connections.lock().unwrap_or_else(|e| e.into_inner()).remove(&device_id);
    }
}
