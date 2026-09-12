//! Fetching files from a device's NFS server.
//!
//! CDJs export their media over NFSv2; rekordbox exports the folders the
//! Link tracks live in. This module caches one RPC connection and one
//! mounted root handle per device address.

pub mod programs;
pub mod rpc;
pub mod xdr;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::types::{Device, DeviceId, DeviceType, MediaSlot};
use crate::{Error, Result};

pub use programs::{FileHandle, FileInfo, REKORDBOX_PORTMAP_PORT, STANDARD_PORTMAP_PORT};
pub use rpc::{RetryConfig, RpcConnection, RpcProgram};

/// Progress of a file download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchProgress {
    pub read: u32,
    pub total: u32,
}

struct ClientSet {
    conn: Arc<RpcConnection>,
    mount_client: RpcProgram,
    nfs_client: RpcProgram,
}

/// The slot <-> mount name mapping is well known.
fn slot_mount_path(slot: MediaSlot) -> Option<&'static str> {
    match slot {
        MediaSlot::Usb => Some("/C/"),
        MediaSlot::Sd => Some("/B/"),
        _ => None,
    }
}

/// True for media slots that support NFS access (USB, SD and rekordbox).
pub fn is_nfs_media_slot(slot: MediaSlot) -> bool {
    matches!(slot, MediaSlot::Usb | MediaSlot::Sd | MediaSlot::Rb)
}

/// Where a file lives on a device's NFS server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NfsPath {
    pub mount_path: String,
    pub nfs_path: String,
}

/// Parse a Windows absolute path (e.g. `C:\Users\chris\Music\track.mp3`)
/// into the NFS mount path and relative file path.
pub fn parse_windows_path(file_path: &str) -> Option<NfsPath> {
    let mut chars = file_path.chars();
    let drive = chars.next()?;
    if !drive.is_ascii_alphabetic() || chars.next()? != ':' {
        return None;
    }
    let sep = chars.next()?;
    if sep != '/' && sep != '\\' {
        return None;
    }
    let rest: String = chars.collect();
    Some(NfsPath { mount_path: format!("/{}/", drive.to_ascii_uppercase()), nfs_path: rest.replace('\\', "/") })
}

/// Resolve the NFS mount path and file path for a given slot.
///
/// For USB/SD slots, the mount path is well-known (/C/ and /B/). For the RB
/// slot, the mount path is extracted from the file path returned by remotedb:
/// - Windows: `C:\Users\chris\Music\track.mp3` → mount `/C/`, path `Users/chris/Music/track.mp3`
/// - macOS: `/Users/chris/Music/track.mp3` → mount `/`, path `Users/chris/Music/track.mp3`
pub fn resolve_nfs_path(slot: MediaSlot, file_path: &str) -> Result<NfsPath> {
    if slot == MediaSlot::Rb {
        if let Some(parsed) = parse_windows_path(file_path) {
            return Ok(parsed);
        }
        if let Some(rest) = file_path.strip_prefix('/') {
            return Ok(NfsPath { mount_path: "/".into(), nfs_path: rest.to_string() });
        }
    }
    match slot_mount_path(slot) {
        Some(mount) => Ok(NfsPath { mount_path: mount.to_string(), nfs_path: file_path.to_string() }),
        None => Err(Error::Nfs(format!("slot {slot:?} is not reachable over NFS"))),
    }
}

struct Caches {
    /// The module-level retry configuration for newly created connections.
    retry_config: RwLock<RetryConfig>,
    /// player address -> active connections. It is not guaranteed that the
    /// connections in the cache will still be connected.
    clients: tokio::sync::Mutex<HashMap<Ipv4Addr, Arc<ClientSet>>>,
    /// (device address, mount path) -> root file handle. The file handles may
    /// become stale should the media connected to the player's slot change.
    root_handles: Mutex<HashMap<Ipv4Addr, HashMap<String, FileHandle>>>,
}

fn caches() -> &'static Caches {
    static CACHES: OnceLock<Caches> = OnceLock::new();
    CACHES.get_or_init(|| Caches {
        retry_config: RwLock::new(RetryConfig::default()),
        clients: tokio::sync::Mutex::new(HashMap::new()),
        root_handles: Mutex::new(HashMap::new()),
    })
}

/// Get the portmapper port for the given device. Rekordbox software uses a
/// non-standard port (50111) while CDJs and other hardware use the standard
/// port (111).
pub fn get_portmap_port(device: &Device) -> u16 {
    if device.device_type == DeviceType::Rekordbox {
        REKORDBOX_PORTMAP_PORT
    } else {
        STANDARD_PORTMAP_PORT
    }
}

/// Given a device running a nfs and mountd RPC server, provide RpcProgram
/// clients that may be used to call these services.
///
/// The clients are cached for the address; the connections are recreated if
/// the cached clients have disconnected.
async fn get_clients(device: &Device) -> Result<Arc<ClientSet>> {
    let address = device.ip;
    let mut clients = caches().clients.lock().await;

    if let Some(set) = clients.get(&address) {
        if set.conn.connected() {
            return Ok(Arc::clone(set));
        }
        // Cached socket is no longer connected. Remove and reconnect
        clients.remove(&address);
    }

    let retry_config = caches().retry_config.read().unwrap_or_else(|e| e.into_inner()).clone();
    let conn = Arc::new(RpcConnection::new(address, Some(retry_config)).await?);
    let portmap_port = get_portmap_port(device);

    let mount_client = programs::make_program_client(
        Arc::clone(&conn),
        programs::Program { id: xdr::mount::PROGRAM, version: xdr::mount::VERSION },
        portmap_port,
    )
    .await?;

    let nfs_client = programs::make_program_client(
        Arc::clone(&conn),
        programs::Program { id: xdr::nfs::PROGRAM, version: xdr::nfs::VERSION },
        portmap_port,
    )
    .await?;

    let set = Arc::new(ClientSet { conn, mount_client, nfs_client });
    clients.insert(address, Arc::clone(&set));
    Ok(set)
}

/// Locate the root filehandle of the given device mount path, cached per
/// device + mount path. Returns `None` when the mount path is not exported.
async fn get_root_handle(device: &Device, mount_path: &str, mount_client: &RpcProgram) -> Result<Option<FileHandle>> {
    let address = device.ip;

    let cached =
        caches().root_handles.lock().unwrap_or_else(|e| e.into_inner()).get(&address).and_then(|m| m.get(mount_path).copied());
    if let Some(handle) = cached {
        return Ok(Some(handle));
    }

    let exports = programs::get_exports(mount_client).await?;
    let Some(target) = exports.iter().find(|e| e.filesystem == mount_path) else {
        return Ok(None);
    };

    let root_handle = programs::mount_filesystem(mount_client, target).await?;

    caches()
        .root_handles
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(address)
        .or_default()
        .insert(mount_path.to_string(), root_handle);

    Ok(Some(root_handle))
}

fn bad_roothandle_error(slot: MediaSlot, device_id: DeviceId) -> Error {
    Error::Nfs(format!("The slot ({}) is not exported on Device {device_id}", slot.as_u8()))
}

/// Resolve the file, clearing and re-mounting the root handle once if the
/// first lookup fails (the root handle may be stale after a media change).
async fn resolve_file(device: &Device, slot: MediaSlot, path: &str) -> Result<(Arc<ClientSet>, FileInfo)> {
    let NfsPath { mount_path, nfs_path } = resolve_nfs_path(slot, path)?;
    let clients = get_clients(device).await?;

    let root_handle = get_root_handle(device, &mount_path, &clients.mount_client)
        .await?
        .ok_or_else(|| bad_roothandle_error(slot, device.id))?;

    // It's possible that our roothandle is no longer valid, if we fail to
    // lookup a path lets first try and clear our roothandle cache
    let info = match programs::lookup_path(&clients.nfs_client, &root_handle, &nfs_path).await {
        Ok(info) => info,
        Err(_) => {
            caches().root_handles.lock().unwrap_or_else(|e| e.into_inner()).remove(&device.ip);
            let root_handle = get_root_handle(device, &mount_path, &clients.mount_client)
                .await?
                .ok_or_else(|| bad_roothandle_error(slot, device.id))?;
            // Desperately try once more to lookup the file
            programs::lookup_path(&clients.nfs_client, &root_handle, &nfs_path).await?
        }
    };

    Ok((clients, info))
}

/// Fetch a range of bytes from a file on a device's NFS server. Optimized for
/// partial reads (e.g., reading file headers for metadata extraction).
pub async fn fetch_file_range(device: &Device, slot: MediaSlot, path: &str, offset: u32, length: u32) -> Result<Vec<u8>> {
    let (clients, info) = resolve_file(device, slot, path).await?;

    let actual_offset = offset.min(info.size);
    let actual_length = length.min(info.size - actual_offset);

    if actual_length == 0 {
        return Ok(Vec::new());
    }

    programs::fetch_file_range(&clients.nfs_client, &info, actual_offset, actual_length).await
}

/// Get file info (size, handle) without fetching the file content.
pub async fn get_file_info(device: &Device, slot: MediaSlot, path: &str) -> Result<FileInfo> {
    let (_, info) = resolve_file(device, slot, path).await?;
    Ok(info)
}

/// Options for [`fetch_file`].
#[derive(Default)]
pub struct FetchFileOptions<'a> {
    /// Called as each chunk arrives.
    pub on_progress: Option<&'a mut (dyn FnMut(FetchProgress) + Send)>,
    /// Bytes per read, at most 8192.
    pub chunk_size: Option<u32>,
}

/// Fetch a file from a device's NFS server.
///
/// The connection and root filehandle (the 'mounted' NFS export on the
/// device) is cached to improve subsequent fetching performance. It's
/// important that when the device disconnects you call
/// [`reset_device_cache`].
pub async fn fetch_file(device: &Device, slot: MediaSlot, path: &str, options: FetchFileOptions<'_>) -> Result<Vec<u8>> {
    let (clients, info) = resolve_file(device, slot, path).await?;
    programs::fetch_file(&clients.nfs_client, &info, options.on_progress, options.chunk_size).await
}

/// Clear the cached NFS connection and root filehandle for the given device.
pub async fn reset_device_cache(device: &Device) {
    caches().clients.lock().await.remove(&device.ip);
    caches().root_handles.lock().unwrap_or_else(|e| e.into_inner()).remove(&device.ip);
}

/// Configure the retry strategy for making NFS calls using this module.
pub async fn configure_retry_strategy(config: RetryConfig) {
    *caches().retry_config.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
    for client in caches().clients.lock().await.values() {
        client.conn.set_retry_config(config.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_paths() {
        let p = parse_windows_path(r"C:\Users\chris\Music\track.mp3").unwrap();
        assert_eq!(p.mount_path, "/C/");
        assert_eq!(p.nfs_path, "Users/chris/Music/track.mp3");
        let p = parse_windows_path("d:/Music/a.flac").unwrap();
        assert_eq!(p.mount_path, "/D/");
        assert_eq!(p.nfs_path, "Music/a.flac");
        assert!(parse_windows_path("/Users/chris/a.mp3").is_none());
        assert!(parse_windows_path("relative/path.mp3").is_none());
    }

    #[test]
    fn resolves_slots() {
        assert_eq!(
            resolve_nfs_path(MediaSlot::Usb, "PIONEER/x").unwrap(),
            NfsPath { mount_path: "/C/".into(), nfs_path: "PIONEER/x".into() }
        );
        assert_eq!(resolve_nfs_path(MediaSlot::Sd, "a").unwrap().mount_path, "/B/");
        let rb = resolve_nfs_path(MediaSlot::Rb, "/Users/chris/Music/t.mp3").unwrap();
        assert_eq!(rb, NfsPath { mount_path: "/".into(), nfs_path: "Users/chris/Music/t.mp3".into() });
        let win = resolve_nfs_path(MediaSlot::Rb, r"C:\Music\t.mp3").unwrap();
        assert_eq!(win.mount_path, "/C/");
        assert!(resolve_nfs_path(MediaSlot::Cd, "x").is_err());
    }

    #[test]
    fn portmap_port_by_device_type() {
        let cdj = Device::new("CDJ", 1, DeviceType::Cdj, [0; 6], Ipv4Addr::LOCALHOST);
        let rb = Device::new("rekordbox", 17, DeviceType::Rekordbox, [0; 6], Ipv4Addr::LOCALHOST);
        assert_eq!(get_portmap_port(&cdj), 111);
        assert_eq!(get_portmap_port(&rb), 50111);
    }
}
