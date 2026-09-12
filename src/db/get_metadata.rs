//! Track metadata lookups.

use crate::db::utils::anlz_loader;
use crate::entities::Track;
use crate::localdb::rekordbox::{load_anlz, AnlzKind};
use crate::localdb::{DatabaseType, LocalDatabase, TrackLookupHint};
use crate::remotedb::{MenuTarget, QueryDescriptor, RemoteDatabase};
use crate::types::{Device, DeviceId, MediaSlot, TrackType};
use crate::{Error, Result};

/// Options for [`crate::db::Database::get_metadata`].
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The device to query the track metadata from.
    pub device_id: DeviceId,
    /// The media slot the track is present in.
    pub track_slot: MediaSlot,
    /// The type of track we are querying for.
    pub track_type: TrackType,
    /// The track id to retrieve metadata for.
    pub track_id: u32,
    /// The track's BPM as the player reports it in its status packet, when
    /// known. Used to check that the local database row for `track_id` is
    /// really the track the player is showing (see
    /// [`LocalDatabase::find_track`]).
    pub track_bpm: Option<f64>,
}

/// Why a local database lookup produced no track.
///
/// `via_local` used to answer both cases with a bare null, so a metadata
/// failure in the field was indistinguishable from a slot that never
/// hydrated — the reason NP3-361 could not be diagnosed from the logs a user
/// sent in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalMiss {
    /// No rekordbox database is loaded for that device slot.
    NoDatabase,
    /// The database is loaded, but holds no track with that id.
    TrackAbsent,
}

/// The outcome of a local database metadata lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalResult {
    pub track: Option<Track>,
    pub miss: Option<LocalMiss>,
    /// Set when the slot had to switch to its other database format to
    /// answer: the format it is now served from.
    pub switched_to: Option<DatabaseType>,
}

pub async fn via_remote(remote: &RemoteDatabase, opts: &Options) -> Result<Option<Track>> {
    let Some(conn) = remote.get(opts.device_id).await? else {
        return Ok(None);
    };

    let descriptor = QueryDescriptor { track_slot: opts.track_slot, track_type: opts.track_type, menu_target: MenuTarget::Main };

    let is_unanalyzed = matches!(opts.track_type, TrackType::Unanalyzed | TrackType::AudioCd);
    let is_streaming = opts.track_type == TrackType::Streaming;
    let skip_local_file_lookups = is_unanalyzed || is_streaming;

    // Unanalyzed tracks use GetGenericMetadata (reads ID3 tags from the audio
    // file). Streaming tracks (Beatport) use the regular GetMetadata query.
    let mut track = if is_unanalyzed {
        conn.get_generic_metadata(&descriptor, opts.track_id).await?
    } else {
        conn.get_metadata(&descriptor, opts.track_id).await?
    };

    // Try to get file path — for streaming tracks this returns the Beatport
    // track ID (e.g. "/26883657.m4a") which we use for Beatport API lookups
    match conn.get_track_info(&descriptor, opts.track_id).await {
        Ok(path) => track.file_path = path,
        Err(e) if skip_local_file_lookups => {
            tracing::debug!(target: "alphatheta_connect", "track info unavailable for track {}: {e}", opts.track_id);
        }
        Err(e) => return Err(e),
    }

    // Beat grid is only available for analyzed local tracks
    if !skip_local_file_lookups {
        track.beat_grid = Some(conn.get_beatgrid(&descriptor, opts.track_id).await?);
    }

    Ok(Some(track))
}

pub async fn via_local(local: &LocalDatabase, device: &Device, opts: &Options) -> Result<LocalResult> {
    if !opts.track_slot.is_database_slot() {
        return Err(Error::State("Expected USB or SD slot for local database query".into()));
    }

    let lookup =
        local.find_track(opts.device_id, opts.track_slot, opts.track_id, TrackLookupHint { track_bpm: opts.track_bpm }).await?;
    if lookup.adapter.is_none() {
        return Ok(LocalResult { track: None, miss: Some(LocalMiss::NoDatabase), switched_to: None });
    }

    let Some(mut track) = lookup.track else {
        return Ok(LocalResult { track: None, miss: Some(LocalMiss::TrackAbsent), switched_to: None });
    };

    let loader = anlz_loader(device, opts.track_slot);
    let anlz = match &track.analyze_path {
        Some(path) => load_anlz(path, AnlzKind::Dat, &loader).await?,
        None => Default::default(),
    };

    track.beat_grid = anlz.dat.beat_grid;
    track.waveform_hd = None;

    Ok(LocalResult { track: Some(track), miss: None, switched_to: lookup.switched_to })
}
