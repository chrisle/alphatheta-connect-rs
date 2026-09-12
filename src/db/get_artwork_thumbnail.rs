//! The low-resolution artwork thumbnail from the rekordbox database.

use crate::entities::Track;
use crate::localdb::LocalDatabase;
use crate::nfs::{fetch_file, FetchFileOptions};
use crate::remotedb::{MenuTarget, QueryDescriptor, RemoteDatabase};
use crate::types::{Device, DeviceId, MediaSlot, TrackType};
use crate::{Error, Result};

/// Options for the thumbnail, waveform and file lookups.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The device to query the track artwork off of.
    pub device_id: DeviceId,
    /// The media slot the track is present in.
    pub track_slot: MediaSlot,
    /// The type of track we are querying artwork for.
    pub track_type: TrackType,
    /// The track to lookup artwork for.
    pub track: Track,
}

pub async fn via_remote(remote: &RemoteDatabase, opts: &Options) -> Result<Option<Vec<u8>>> {
    let Some(conn) = remote.get(opts.device_id).await? else {
        return Ok(None);
    };

    let Some(artwork) = &opts.track.artwork else {
        return Ok(None);
    };

    let descriptor = QueryDescriptor { track_slot: opts.track_slot, track_type: opts.track_type, menu_target: MenuTarget::Main };

    Ok(Some(conn.get_artwork(&descriptor, artwork.id).await?))
}

pub async fn via_local(local: &LocalDatabase, device: &Device, opts: &Options) -> Result<Option<Vec<u8>>> {
    if !opts.track_slot.is_database_slot() {
        return Err(Error::State("Expected USB or SD slot for local database query".into()));
    }

    if local.get(opts.device_id, opts.track_slot).await?.is_none() {
        return Ok(None);
    }

    let Some(path) = opts.track.artwork.as_ref().and_then(|a| a.path.clone()) else {
        return Ok(None);
    };

    match fetch_file(device, opts.track_slot, &path, FetchFileOptions::default()).await {
        Ok(data) => Ok(Some(data)),
        Err(e) => {
            tracing::debug!(target: "alphatheta_connect", "artwork fetch failed: {e}");
            Ok(None)
        }
    }
}
