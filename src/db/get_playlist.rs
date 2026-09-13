//! Playlist lookups.

use crate::entities::Playlist;
use crate::localdb::{LocalDatabase, PlaylistQueryResult};
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

    Ok(Some(local_contents(result)))
}

/// The contents of a playlist as a local adapter reports it.
///
/// Playlist entries reference the track through `track_id`; `id` is the
/// entry's own row id (or its index, for OneLibrary), not a track.
fn local_contents(result: PlaylistQueryResult) -> PlaylistContents {
    let track_ids: Vec<u32> = result.track_entries.iter().map(|e| e.track_id).collect();

    PlaylistContents { folders: result.folders, playlists: result.playlists, total_tracks: track_ids.len(), track_ids }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::PlaylistEntry;

    #[test]
    fn local_entries_resolve_through_their_track_id() {
        // Entry ids are numbered independently of the tracks they reference,
        // the shape both the pdb ORM and the OneLibrary adapter produce.
        let result = PlaylistQueryResult {
            folders: Vec::new(),
            playlists: Vec::new(),
            track_entries: vec![
                PlaylistEntry { id: 1, sort_index: 0, playlist_id: 5, track_id: 20 },
                PlaylistEntry { id: 2, sort_index: 1, playlist_id: 5, track_id: 10 },
            ],
        };

        let contents = local_contents(result);
        assert_eq!(contents.total_tracks, 2);
        assert_eq!(contents.track_ids, vec![20, 10]);
    }
}
