//! Playlist lookups.

use crate::entities::Playlist;
use crate::localdb::LocalDatabase;
use crate::remotedb::{MenuTarget, QueryDescriptor, RemoteDatabase};
use crate::types::{DeviceId, MediaSlot, PlaylistContents, TrackType};
use crate::{Error, Result};

/// Options for [`crate::db::Database::get_playlist`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The playlist or folder to query the entries of. `None` retrieves the
    /// root playlist.
    pub playlist: Option<Playlist>,
    /// The device to query the playlist from.
    pub device_id: DeviceId,
    /// The media slot the playlist is present in.
    pub media_slot: MediaSlot,
}

pub async fn via_remote(remote: &RemoteDatabase, opts: &Options) -> Result<Option<PlaylistContents>> {
    let Some(conn) = remote.get(opts.device_id).await? else {
        return Ok(None);
    };

    let descriptor = QueryDescriptor { track_slot: opts.media_slot, track_type: TrackType::Rb, menu_target: MenuTarget::Main };

    let id = opts.playlist.as_ref().map(|p| p.id);
    let is_folder_request = opts.playlist.as_ref().map(|p| p.is_folder).unwrap_or(true);

    let result = conn.get_playlist(&descriptor, id, is_folder_request).await?;
    let track_ids: Vec<u32> = result.track_entries.iter().map(|e| e.id()).collect();

    Ok(Some(PlaylistContents { folders: result.folders, playlists: result.playlists, total_tracks: track_ids.len(), track_ids }))
}

pub async fn via_local(local: &LocalDatabase, opts: &Options) -> Result<Option<PlaylistContents>> {
    if !opts.media_slot.is_database_slot() {
        return Err(Error::State("Expected USB or SD slot for local database query".into()));
    }

    let Some(adapter) = local.get(opts.device_id, opts.media_slot).await? else {
        return Ok(None);
    };

    let result = adapter.find_playlist(opts.playlist.as_ref().map(|p| p.id))?;
    let track_ids: Vec<u32> = result.track_entries.iter().map(|e| e.track_id).collect();

    Ok(Some(PlaylistContents { folders: result.folders, playlists: result.playlists, total_tracks: track_ids.len(), track_ids }))
}
