//! Types for ANLZ loading and pdb hydration.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::entities::CueAndLoop;
use crate::types::{
    BeatGrid, ExtendedCue, SongStructure, VocalConfig, Waveform3BandDetail, Waveform3BandPreview, WaveformHD, WaveformPreviewData,
};
use crate::Result;

/// Resolves ANLZ files into bytes. Typically you would just read the file,
/// but in the case of the prolink network, this would handle loading the file
/// over NFS.
pub type AnlzResolver<'a> = dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> + Send + Sync + 'a;

/// Which analysis file to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnlzKind {
    /// The `.DAT` file, which is missing some extended information, for the
    /// older Pioneer equipment (likely due to memory constraints).
    Dat,
    /// The `.EXT` file which includes colored waveforms and other extended data.
    Ext,
    /// The `.2EX` file with 3-band waveforms and vocal detection.
    TwoEx,
}

impl AnlzKind {
    /// The file extension, as rekordbox writes it.
    pub const fn extension(self) -> &'static str {
        match self {
            AnlzKind::Dat => "DAT",
            AnlzKind::Ext => "EXT",
            AnlzKind::TwoEx => "2EX",
        }
    }
}

/// Data returned from loading DAT anlz files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnlzResponseDAT {
    /// Embedded beat grid information.
    pub beat_grid: Option<BeatGrid>,
    /// Embedded cue and loop information.
    pub cue_and_loops: Option<Vec<CueAndLoop>>,
    /// Standard waveform preview (400 bytes, PWAV tag).
    pub waveform_preview: Option<WaveformPreviewData>,
    /// Tiny waveform preview (100 bytes, PWV2 tag).
    pub waveform_tiny: Option<WaveformPreviewData>,
}

/// Data returned from loading EXT anlz files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnlzResponseEXT {
    /// HD Waveform information (PWV5 tag).
    pub waveform_hd: Option<WaveformHD>,
    /// Extended cues with colors and comments (PCO2 tag).
    pub extended_cues: Option<Vec<ExtendedCue>>,
    /// Song structure / phrase analysis (PSSI tag).
    pub song_structure: Option<SongStructure>,
    /// Monochrome detailed waveform (PWV3 tag).
    pub waveform_detail: Option<Vec<u8>>,
    /// Color waveform preview (PWV4 tag, 7200 bytes = 1200 columns × 6 bytes).
    pub waveform_color_preview: Option<Vec<u8>>,
}

/// Data returned from loading 2EX anlz files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnlzResponse2EX {
    /// 3-band color waveform preview (PWV6 tag).
    pub waveform_3band_preview: Option<Waveform3BandPreview>,
    /// 3-band color detail waveform (PWV7 tag).
    pub waveform_3band_detail: Option<Waveform3BandDetail>,
    /// Vocal detection config (PWVC tag).
    pub vocal_config: Option<VocalConfig>,
}

/// Everything an analysis file of any kind can hold. Each kind of file
/// populates its own group of fields; the rest stay `None`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnlzResponse {
    #[serde(flatten)]
    pub dat: AnlzResponseDAT,
    #[serde(flatten)]
    pub ext: AnlzResponseEXT,
    #[serde(flatten)]
    pub two_ex: AnlzResponse2EX,
}

/// Details about the current state of the hydration task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HydrationProgress {
    /// The specific table that progress is being reported for.
    pub table: String,
    /// The total progress steps for this table.
    pub total: usize,
    /// The completed number of progress steps.
    pub complete: usize,
}
