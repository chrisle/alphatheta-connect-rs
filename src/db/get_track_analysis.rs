//! All analysis data of a track from its EXT and 2EX files.

use serde::{Deserialize, Serialize};

use crate::db::utils::anlz_loader;
use crate::entities::Track;
use crate::localdb::rekordbox::{load_anlz, AnlzKind};
use crate::localdb::LocalDatabase;
use crate::types::{
    Device, DeviceId, ExtendedCue, MediaSlot, SongStructure, TrackType, VocalConfig, Waveform3BandDetail, Waveform3BandPreview,
    WaveformHD,
};
use crate::{Error, Result};

/// Everything the EXT and 2EX analysis files hold for a track.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackAnalysis {
    /// Extended cues with colors and comments (PCO2 tag).
    pub extended_cues: Option<Vec<ExtendedCue>>,
    /// Song structure / phrase analysis (PSSI tag).
    pub song_structure: Option<SongStructure>,
    /// Color waveform preview (PWV4 tag).
    pub waveform_color_preview: Option<Vec<u8>>,
    /// HD waveform data (PWV5 tag).
    pub waveform_hd: Option<WaveformHD>,
    /// 3-band color waveform preview (PWV6 tag from .2EX).
    pub waveform_3band_preview: Option<Waveform3BandPreview>,
    /// 3-band color detail waveform (PWV7 tag from .2EX).
    pub waveform_3band_detail: Option<Waveform3BandDetail>,
    /// Vocal detection config (PWVC tag from .2EX).
    pub vocal_config: Option<VocalConfig>,
}

/// Options for [`crate::db::Database::get_track_analysis`].
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub device_id: DeviceId,
    pub track_slot: MediaSlot,
    pub track_type: TrackType,
    pub track: Track,
}

pub async fn via_local(local: &LocalDatabase, device: &Device, opts: &Options) -> Result<Option<TrackAnalysis>> {
    if !opts.track_slot.is_database_slot() {
        return Err(Error::State("Expected USB or SD slot for local database query".into()));
    }

    if local.get(opts.device_id, opts.track_slot).await?.is_none() {
        return Ok(None);
    }

    let Some(analyze_path) = opts.track.analyze_path.as_deref() else {
        return Ok(Some(TrackAnalysis::default()));
    };

    let resolver = anlz_loader(device, opts.track_slot);
    let (ext, two_ex) =
        tokio::join!(load_anlz(analyze_path, AnlzKind::Ext, &resolver), load_anlz(analyze_path, AnlzKind::TwoEx, &resolver));
    let ext = ext?;
    // The 2EX file is optional (only rekordbox 7 writes it).
    let two_ex = two_ex.ok();

    Ok(Some(TrackAnalysis {
        extended_cues: ext.ext.extended_cues,
        song_structure: ext.ext.song_structure,
        waveform_color_preview: ext.ext.waveform_color_preview,
        waveform_hd: ext.ext.waveform_hd,
        waveform_3band_preview: two_ex.as_ref().and_then(|t| t.two_ex.waveform_3band_preview.clone()),
        waveform_3band_detail: two_ex.as_ref().and_then(|t| t.two_ex.waveform_3band_detail.clone()),
        vocal_config: two_ex.as_ref().and_then(|t| t.two_ex.vocal_config),
    }))
}
