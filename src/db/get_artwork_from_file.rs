//! Full-resolution artwork extracted from the audio file over NFS.

use crate::artwork::{extract_artwork_from_device, is_artwork_extraction_supported};
use crate::entities::Track;
use crate::logger::{noop_logger, SharedLogger};
use crate::nfs::is_nfs_media_slot;
use crate::types::{Device, DeviceId, MediaSlot};
use crate::utils::get_slot_name;

/// Options for [`crate::db::Database::get_artwork`].
#[derive(Clone)]
pub struct Options {
    pub device_id: DeviceId,
    pub track_slot: MediaSlot,
    pub track: Track,
    pub logger: Option<SharedLogger>,
}

/// Extract artwork directly from an audio file via NFS.
pub async fn via_file_extraction(device: &Device, opts: &Options) -> Option<Vec<u8>> {
    let logger = opts.logger.clone().unwrap_or_else(noop_logger);

    if !is_nfs_media_slot(opts.track_slot) {
        logger.debug(&format!(
            "[artwork-nfs] Skipping: unsupported slot {} (device {})",
            get_slot_name(opts.track_slot),
            device.name
        ));
        return None;
    }

    if opts.track.file_path.is_empty() {
        logger.debug("[artwork-nfs] Skipping: no filePath on track");
        return None;
    }

    let extension = opts.track.file_path.rsplit('.').next().unwrap_or("").to_lowercase();
    if !is_artwork_extraction_supported(&extension) {
        logger.debug(&format!("[artwork-nfs] Skipping: unsupported extension \".{extension}\" ({})", opts.track.file_path));
        return None;
    }

    logger.debug(&format!(
        "[artwork-nfs] Extracting from {} (slot={}, device={} @ {})",
        opts.track.file_path,
        get_slot_name(opts.track_slot),
        device.name,
        device.ip
    ));

    match extract_artwork_from_device(device, opts.track_slot, &opts.track.file_path, Some(logger.clone())).await {
        Ok(Some(artwork)) => {
            logger.debug(&format!("[artwork-nfs] Success: {} ({} bytes)", artwork.mime_type.as_str(), artwork.data.len()));
            Some(artwork.data)
        }
        Ok(None) => {
            logger.debug("[artwork-nfs] No embedded artwork found in file");
            None
        }
        Err(e) => {
            logger.warn(&format!("[artwork-nfs] NFS extraction failed: {e}"));
            None
        }
    }
}
