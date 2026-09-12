//! RemoteDB queries from passive mode.
//!
//! This allows querying track metadata from rekordbox Link without fully
//! announcing a virtual CDJ on the network. It uses a "virtual" device ID
//! (default: 5) for the introduction handshake.
//!
//! Note: this makes the mode not fully "passive" since TCP packets are sent,
//! but it avoids the UDP announcements that would conflict with rekordbox.

use std::collections::HashMap;
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::entities::Track;
use crate::passive::devices::PassiveDeviceManager;
use crate::remotedb::fields::{read_field, Field, FieldType};
use crate::remotedb::message::{control_request, response_type as response, Message};
use crate::remotedb::{get_remote_db_server_port, Connection, MenuTarget, QueryDescriptor, QueryInterface};
use crate::types::{Device, DeviceId, DeviceType, MediaSlot, TrackType};
use crate::{Error, Result};

struct Inner {
    device_manager: PassiveDeviceManager,
    virtual_device_id: DeviceId,
    connections: Mutex<HashMap<DeviceId, Arc<Connection>>>,
    device_locks: Mutex<HashMap<DeviceId, Arc<tokio::sync::Mutex<()>>>>,
}

/// RemoteDB query support for passive mode.
#[derive(Clone)]
pub struct PassiveRemoteDatabase {
    inner: Arc<Inner>,
}

impl PassiveRemoteDatabase {
    /// `virtual_device_id` defaults to 5.
    pub fn new(device_manager: PassiveDeviceManager, virtual_device_id: Option<DeviceId>) -> Self {
        Self {
            inner: Arc::new(Inner {
                device_manager,
                virtual_device_id: virtual_device_id.unwrap_or(5),
                connections: Mutex::new(HashMap::new()),
                device_locks: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Open a connection to the specified device for querying.
    pub async fn connect_to_device(&self, device: &Device) -> Result<()> {
        let db_port = get_remote_db_server_port(device.ip).await?;

        let mut socket = TcpStream::connect(SocketAddr::V4(SocketAddrV4::new(device.ip, db_port))).await?;
        socket.set_nodelay(true)?;

        // Send required preamble to open communications with the device
        socket.write_all(&Field::UInt32(0x01).to_bytes()).await?;

        // Read the response. It should be a UInt32 field with the value 0x01.
        let data = read_field(&mut socket, FieldType::UInt32).await?;
        if data.as_number() != Some(0x01) {
            return Err(Error::RemoteDb(format!("Expected 0x01 during preamble handshake. Got {:?}", data.as_number())));
        }

        // Send introduction message with our virtual device ID
        let intro = Message::with_transaction(
            0xffff_fffe,
            control_request::INTRODUCE,
            vec![Field::UInt32(u32::from(self.inner.virtual_device_id))],
        );
        socket.write_all(&intro.to_bytes()).await?;
        let resp = Message::from_stream(&mut socket, response::SUCCESS).await?;
        if resp.message_type != response::SUCCESS {
            return Err(Error::RemoteDb(format!("Failed to introduce self to device ID: {}", device.id)));
        }

        self.inner
            .connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device.id, Arc::new(Connection::new(device.clone(), socket)));
        Ok(())
    }

    /// Disconnect from the specified device.
    pub async fn disconnect_from_device(&self, device: &Device) {
        let conn = self.inner.connections.lock().unwrap_or_else(|e| e.into_inner()).remove(&device.id);
        if let Some(conn) = conn {
            // Errors during disconnect are ignored
            let goodbye = Message::with_transaction(0xffff_fffe, control_request::DISCONNECT, vec![]);
            let _ = conn.write_message(goodbye).await;
            conn.close().await;
        }
    }

    /// Gets the remote database query interface for the given device,
    /// connecting first if needed. `None` if the device is not on the network.
    pub async fn get(&self, device_id: DeviceId) -> Result<Option<QueryInterface>> {
        let Some(device) = self.inner.device_manager.device(device_id) else {
            return Ok(None);
        };

        let lock = {
            let mut locks = self.inner.device_locks.lock().unwrap_or_else(|e| e.into_inner());
            Arc::clone(locks.entry(device.id).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))))
        };

        let conn = {
            let _guard = lock.lock().await;
            let existing = self.inner.connections.lock().unwrap_or_else(|e| e.into_inner()).get(&device_id).cloned();
            match existing {
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
            }
        };

        // Create a virtual host device for the query interface
        let virtual_host = Device {
            id: self.inner.virtual_device_id,
            name: "alphatheta-connect".into(),
            device_type: DeviceType::Cdj,
            mac_addr: [0; 6],
            // Not really used, just needs to be valid
            ip: device.ip,
            last_active: None,
        };

        Ok(Some(QueryInterface::new(conn, lock, virtual_host)))
    }

    async fn with_track_info(
        &self,
        conn: &QueryInterface,
        descriptor: &QueryDescriptor,
        track_id: u32,
        mut track: Track,
    ) -> Track {
        // The file path is not available for streaming tracks (Beatport,
        // etc.) and may not be available for unanalyzed tracks.
        if let Ok(path) = conn.get_track_info(descriptor, track_id).await {
            track.file_path = path;
        }
        track
    }

    /// Query track metadata from a device.
    ///
    /// - `device_id`: the device to query (e.g., 17 for Rekordbox)
    /// - `track_slot`: the media slot (e.g., `MediaSlot::Rb` for Rekordbox Link)
    /// - `track_type`: the track type (e.g., `TrackType::Rb`)
    /// - `track_id`: the track ID to look up
    pub async fn get_track_metadata(
        &self,
        device_id: DeviceId,
        track_slot: MediaSlot,
        track_type: TrackType,
        track_id: u32,
    ) -> Result<Option<Track>> {
        let Some(conn) = self.get(device_id).await? else {
            return Ok(None);
        };
        let descriptor = QueryDescriptor { track_slot, track_type, menu_target: MenuTarget::Main };

        match conn.get_metadata(&descriptor, track_id).await {
            Ok(track) => Ok(Some(self.with_track_info(&conn, &descriptor, track_id, track).await)),
            Err(e) => {
                // Connection may have been closed, remove it
                self.inner.connections.lock().unwrap_or_else(|e| e.into_inner()).remove(&device_id);
                Err(e)
            }
        }
    }

    /// Query metadata for an unanalyzed track (loaded directly from USB
    /// without rekordbox analysis). Uses GetGenericMetadata which reads ID3
    /// tags from the audio file via the CDJ.
    pub async fn get_generic_track_metadata(
        &self,
        device_id: DeviceId,
        track_slot: MediaSlot,
        track_type: TrackType,
        track_id: u32,
    ) -> Result<Option<Track>> {
        let Some(conn) = self.get(device_id).await? else {
            return Ok(None);
        };
        let descriptor = QueryDescriptor { track_slot, track_type, menu_target: MenuTarget::Main };

        match conn.get_generic_metadata(&descriptor, track_id).await {
            Ok(track) => Ok(Some(self.with_track_info(&conn, &descriptor, track_id, track).await)),
            Err(e) => {
                self.inner.connections.lock().unwrap_or_else(|e| e.into_inner()).remove(&device_id);
                Err(e)
            }
        }
    }

    /// Stop all connections.
    pub async fn stop(&self) {
        let conns: Vec<Arc<Connection>> =
            self.inner.connections.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, c)| c).collect();
        for conn in conns {
            conn.close().await;
        }
    }
}
