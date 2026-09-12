//! The database service: the central place to query devices on the prolink
//! network for information from their databases, automatically choosing the
//! best strategy (local pdb / OneLibrary download, or the remote database
//! protocol) to access the data.

pub mod get_artwork_from_file;
pub mod get_artwork_thumbnail;
pub mod get_file;
pub mod get_metadata;
pub mod get_playlist;
pub mod get_track_analysis;
pub mod get_waveforms;
pub mod utils;

use crate::devices::DeviceManager;
use crate::entities::Track;
use crate::localdb::{DatabaseType, LocalDatabase};
use crate::logger::{noop_logger, SharedLogger};
use crate::remotedb::RemoteDatabase;
use crate::types::{Device, DeviceType, MediaSlot, PlaylistContents, TrackType, Waveforms};
use crate::utils::get_slot_name;
use crate::Result;

pub use get_metadata::{LocalMiss, LocalResult};
pub use get_track_analysis::TrackAnalysis;

/// Where a lookup can be answered from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LookupStrategy {
    Remote,
    Local,
    NoneAvailable,
}

/// The database service.
#[derive(Clone)]
pub struct Database {
    device_manager: DeviceManager,
    /// The local database service, used when querying media devices
    /// connected directly to CDJs containing a rekordbox formatted database.
    local: LocalDatabase,
    /// The remote database service, used when querying the Rekordbox software
    /// or a CDJ with an unanalyzed media device connected (when possible).
    remote: RemoteDatabase,
    logger: SharedLogger,
}

impl Database {
    pub fn new(
        local: LocalDatabase,
        remote: RemoteDatabase,
        device_manager: DeviceManager,
        logger: Option<SharedLogger>,
    ) -> Self {
        Self { device_manager, local, remote, logger: logger.unwrap_or_else(noop_logger) }
    }

    /// The local database service.
    pub fn local(&self) -> &LocalDatabase {
        &self.local
    }

    /// The remote database service.
    pub fn remote(&self) -> &RemoteDatabase {
        &self.remote
    }

    fn track_lookup_strategy(device: &Device, track_type: TrackType) -> LookupStrategy {
        let is_unanalyzed = matches!(track_type, TrackType::AudioCd | TrackType::Unanalyzed);
        let is_streaming = track_type == TrackType::Streaming;

        // Unanalyzed and streaming tracks on CDJs must use RemoteDB
        // (streaming services like Beatport have no local database)
        if device.device_type == DeviceType::Cdj && (is_unanalyzed || is_streaming) {
            return LookupStrategy::Remote;
        }

        match (device.device_type, track_type) {
            (DeviceType::Rekordbox, _) => LookupStrategy::Remote,
            (DeviceType::Cdj, TrackType::Rb) => LookupStrategy::Local,
            _ => LookupStrategy::NoneAvailable,
        }
    }

    fn media_lookup_strategy(device: &Device, slot: MediaSlot) -> LookupStrategy {
        match (device.device_type, slot) {
            (DeviceType::Rekordbox, MediaSlot::Rb) => LookupStrategy::Remote,
            (DeviceType::Rekordbox, _) => LookupStrategy::NoneAvailable,
            _ => LookupStrategy::Local,
        }
    }

    /// Get the database type (OneLibrary or pdb) for a loaded device slot.
    /// Returns `None` if the slot uses the remote database or no database is
    /// loaded.
    pub fn get_database_type(&self, device_id: u8, slot: MediaSlot) -> Option<DatabaseType> {
        self.local.get_database_type(device_id, slot)
    }

    /// Retrieve metadata for a track on a specific device slot.
    pub async fn get_metadata(&self, opts: get_metadata::Options) -> Result<Option<Track>> {
        let Some(device) = self.device_manager.get_device_ensured(opts.device_id, None).await else {
            return Ok(None);
        };

        match Self::track_lookup_strategy(&device, opts.track_type) {
            LookupStrategy::Remote => get_metadata::via_remote(&self.remote, &opts).await,
            LookupStrategy::Local => {
                let local = get_metadata::via_local(&self.local, &device, &opts).await?;

                if let Some(switched_to) = local.switched_to {
                    self.logger.info(&format!(
                        "Device {} {} is now read from the {} database: it is the one that holds track {} as the player reports it (NP3-399)",
                        opts.device_id,
                        get_slot_name(opts.track_slot),
                        match switched_to {
                            DatabaseType::Pdb => "legacy PDB",
                            DatabaseType::OneLibrary => "OneLibrary",
                        },
                        opts.track_id
                    ));
                }

                // A local miss used to end the lookup, so the DJ's track
                // silently never reached the overlay for the rest of the set
                // (NP3-361). Say what was missed, then give the CDJ's own
                // database a chance to answer.
                if let Some(miss) = local.miss {
                    self.logger.warn(&format!(
                        "Local metadata lookup missed track {} on device {} {}: {}",
                        opts.track_id,
                        opts.device_id,
                        get_slot_name(opts.track_slot),
                        match miss {
                            LocalMiss::NoDatabase => "no rekordbox database is loaded for that slot",
                            LocalMiss::TrackAbsent => "the loaded database holds no track with that id",
                        }
                    ));
                    return Ok(self.metadata_via_remote_fallback(&opts).await);
                }

                Ok(local.track)
            }
            LookupStrategy::NoneAvailable => Ok(None),
        }
    }

    /// Second chance for a track the local database could not answer for.
    ///
    /// Available regardless of the virtual CDJ's announced ID: CDJs restrict
    /// remote database queries to a device-ID byte in the 1-6 range, but that
    /// byte lives inside the remotedb messages and `RemoteDatabase` picks an
    /// in-range one per connection — the announced ID never constrains this
    /// lookup. A failure here is logged rather than raised, so a field report
    /// says what actually went wrong.
    async fn metadata_via_remote_fallback(&self, opts: &get_metadata::Options) -> Option<Track> {
        match get_metadata::via_remote(&self.remote, opts).await {
            Ok(Some(track)) => {
                self.logger.info(&format!("Remote fallback recovered track {} from device {}", opts.track_id, opts.device_id));
                Some(track)
            }
            Ok(None) => {
                self.logger.warn(&format!("Remote fallback found no track {} on device {}", opts.track_id, opts.device_id));
                None
            }
            Err(e) => {
                self.logger
                    .warn(&format!("Remote fallback failed for track {} on device {}: {e}", opts.track_id, opts.device_id));
                None
            }
        }
    }

    /// Retrieves the file off a specific device slot.
    pub async fn get_file(&self, opts: get_file::Options) -> Result<Option<Vec<u8>>> {
        let Some(device) = self.device_manager.get_device_ensured(opts.device_id, None).await else {
            return Ok(None);
        };

        match Self::track_lookup_strategy(&device, opts.track_type) {
            LookupStrategy::Remote => Ok(get_file::via_remote(&self.remote, &device, &opts)),
            LookupStrategy::Local => get_file::via_local(&self.local, &device, &opts).await,
            LookupStrategy::NoneAvailable => Ok(None),
        }
    }

    /// Retrieves the low-resolution artwork thumbnail from the rekordbox
    /// database.
    ///
    /// This returns the pre-generated thumbnail stored in the rekordbox
    /// database, which is typically small (around 80x80 pixels). For
    /// full-resolution artwork extracted from the audio file, use
    /// [`get_artwork`](Self::get_artwork).
    pub async fn get_artwork_thumbnail(&self, opts: get_artwork_thumbnail::Options) -> Result<Option<Vec<u8>>> {
        let Some(device) = self.device_manager.get_device_ensured(opts.device_id, None).await else {
            return Ok(None);
        };

        match Self::track_lookup_strategy(&device, opts.track_type) {
            LookupStrategy::Remote => get_artwork_thumbnail::via_remote(&self.remote, &opts).await,
            LookupStrategy::Local => get_artwork_thumbnail::via_local(&self.local, &device, &opts).await,
            LookupStrategy::NoneAvailable => Ok(None),
        }
    }

    /// Retrieves artwork for a track by extracting it from the audio file via
    /// NFS.
    ///
    /// This is the primary method for getting artwork. It reads embedded
    /// artwork from the audio file (ID3 tags for MP3, metadata atoms for M4A,
    /// PICTURE blocks for FLAC, etc.) using partial file reads to minimize
    /// data transfer. For low-resolution thumbnails from the rekordbox
    /// database, use [`get_artwork_thumbnail`](Self::get_artwork_thumbnail)
    /// instead.
    pub async fn get_artwork(&self, opts: get_artwork_from_file::Options) -> Option<Vec<u8>> {
        let device = self.device_manager.get_device_ensured(opts.device_id, None).await?;
        get_artwork_from_file::via_file_extraction(&device, &opts).await
    }

    /// Retrieves the waveforms for a track on a specific device slot.
    pub async fn get_waveforms(&self, opts: get_artwork_thumbnail::Options) -> Result<Option<Waveforms>> {
        let Some(device) = self.device_manager.get_device_ensured(opts.device_id, None).await else {
            return Ok(None);
        };

        match Self::track_lookup_strategy(&device, opts.track_type) {
            LookupStrategy::Remote => get_waveforms::via_remote(&self.remote, &opts).await,
            LookupStrategy::Local => get_waveforms::via_local(&self.local, &device, &opts).await,
            LookupStrategy::NoneAvailable => Ok(None),
        }
    }

    /// Retrieves all analysis data from the EXT (and 2EX) file for a track:
    /// extended cues, song structure, waveform color preview, HD waveform,
    /// 3-band waveforms and vocal config.
    pub async fn get_track_analysis(&self, opts: get_track_analysis::Options) -> Result<Option<TrackAnalysis>> {
        let Some(device) = self.device_manager.get_device_ensured(opts.device_id, None).await else {
            return Ok(None);
        };

        match Self::track_lookup_strategy(&device, opts.track_type) {
            LookupStrategy::Local => get_track_analysis::via_local(&self.local, &device, &opts).await,
            LookupStrategy::Remote | LookupStrategy::NoneAvailable => Ok(None),
        }
    }

    /// Retrieve folders, playlists, and tracks within the playlist tree. The
    /// playlist may be left `None` to query the root of the playlist tree.
    ///
    /// NOTE: You will never receive a track list and playlists or folders at
    /// the same time. But the API is simpler to combine the lookup for these.
    pub async fn get_playlist(&self, opts: get_playlist::Options) -> Result<Option<PlaylistContents>> {
        let Some(device) = self.device_manager.get_device_ensured(opts.device_id, None).await else {
            return Ok(None);
        };

        match Self::media_lookup_strategy(&device, opts.media_slot) {
            LookupStrategy::Remote => get_playlist::via_remote(&self.remote, &opts).await,
            LookupStrategy::Local => get_playlist::via_local(&self.local, &opts).await,
            LookupStrategy::NoneAvailable => Ok(None),
        }
    }

    /// Resolve one track of a playlist returned by
    /// [`get_playlist`](Self::get_playlist), by the same strategy.
    pub async fn playlist_track(&self, device_id: u8, media_slot: MediaSlot, track_id: u32) -> Result<Option<Track>> {
        self.get_metadata(get_metadata::Options {
            device_id,
            track_slot: media_slot,
            track_type: TrackType::Rb,
            track_id,
            track_bpm: None,
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn dev(t: DeviceType) -> Device {
        Device::new("x", 1, t, [0; 6], Ipv4Addr::LOCALHOST)
    }

    #[test]
    fn strategies() {
        assert_eq!(Database::track_lookup_strategy(&dev(DeviceType::Cdj), TrackType::Rb), LookupStrategy::Local);
        assert_eq!(Database::track_lookup_strategy(&dev(DeviceType::Cdj), TrackType::Unanalyzed), LookupStrategy::Remote);
        assert_eq!(Database::track_lookup_strategy(&dev(DeviceType::Cdj), TrackType::Streaming), LookupStrategy::Remote);
        assert_eq!(Database::track_lookup_strategy(&dev(DeviceType::Rekordbox), TrackType::Rb), LookupStrategy::Remote);
        assert_eq!(Database::track_lookup_strategy(&dev(DeviceType::Mixer), TrackType::Rb), LookupStrategy::NoneAvailable);
        assert_eq!(Database::media_lookup_strategy(&dev(DeviceType::Rekordbox), MediaSlot::Rb), LookupStrategy::Remote);
        assert_eq!(Database::media_lookup_strategy(&dev(DeviceType::Rekordbox), MediaSlot::Usb), LookupStrategy::NoneAvailable);
        assert_eq!(Database::media_lookup_strategy(&dev(DeviceType::Cdj), MediaSlot::Usb), LookupStrategy::Local);
    }
}
