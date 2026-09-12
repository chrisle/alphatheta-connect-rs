//! Downloading the audio file of a track.

use crate::entities::Track;
use crate::localdb::LocalDatabase;
use crate::logger::{noop_logger, SharedLogger};
use crate::nfs::{fetch_file, FetchFileOptions, FetchProgress};
use crate::remotedb::RemoteDatabase;
use crate::types::{Device, DeviceId, MediaSlot, TrackType};
use crate::{Error, Result};

/// Options for [`crate::db::Database::get_file`].
#[derive(Clone)]
pub struct Options {
    /// The device to query the file off of.
    pub device_id: DeviceId,
    /// The media slot the track is present in.
    pub track_slot: MediaSlot,
    /// The type of track we are querying the file for.
    pub track_type: TrackType,
    /// The track to get the file for.
    pub track: Track,
    /// Logger instance for diagnostic output.
    pub logger: Option<SharedLogger>,
}

/// Maximum allowed XDR read size.
const CHUNK_SIZE: u32 = 8192;

pub fn via_remote(_remote: &RemoteDatabase, _device: &Device, opts: &Options) -> Option<Vec<u8>> {
    let logger = opts.logger.clone().unwrap_or_else(noop_logger);
    logger.error("Getting a file from Rekordbox via ProDJ-Link is not yet supported.");
    None
}

pub async fn via_local(local: &LocalDatabase, device: &Device, opts: &Options) -> Result<Option<Vec<u8>>> {
    let logger = opts.logger.clone().unwrap_or_else(noop_logger);

    if !opts.track_slot.is_database_slot() {
        return Err(Error::State("Expected USB or SD slot for local database query".into()));
    }

    if local.get(opts.device_id, opts.track_slot).await?.is_none() {
        return Ok(None);
    }

    let mut on_progress = |progress: FetchProgress| {
        logger.trace(&format!("{} {}", progress.read, progress.total));
    };

    match fetch_file(
        device,
        opts.track_slot,
        &opts.track.file_path,
        FetchFileOptions { on_progress: Some(&mut on_progress), chunk_size: Some(CHUNK_SIZE) },
    )
    .await
    {
        Ok(data) => Ok(Some(data)),
        Err(e) => {
            tracing::debug!(target: "alphatheta_connect", "file fetch failed: {e}");
            Ok(None)
        }
    }
}
