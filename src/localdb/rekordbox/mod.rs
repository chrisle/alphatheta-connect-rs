//! Rekordbox database (pdb) hydration and analysis file (ANLZ) loading.

pub mod anlz;
pub mod anlz_parsers;
pub mod entity_creators;
pub mod hydrator;
pub mod pdb;
pub mod table_mappings;
pub mod types;

use crate::localdb::orm::MetadataORM;
use crate::Result;

use anlz::{RekordboxAnlz, SectionBody, SectionTag};
use anlz_parsers::*;

pub use hydrator::{ProgressCallback, RekordboxHydrator};
pub use types::{AnlzKind, AnlzResolver, AnlzResponse, AnlzResponse2EX, AnlzResponseDAT, AnlzResponseEXT, HydrationProgress};

/// Given rekordbox pdb file contents, hydrate the provided ORM with all
/// entities from the rekordbox database. This includes all track metadata,
/// including analyzed metadata (such as beatgrids and waveforms).
pub async fn hydrate_database<'a, 'p>(
    orm: &'a MetadataORM,
    pdb_data: &'a [u8],
    on_progress: Option<&'a mut ProgressCallback<'p>>,
) -> Result<()> {
    let mut hydrator = RekordboxHydrator::new(orm, on_progress);
    hydrator.hydrate_from_pdb(pdb_data).await
}

/// Parse an ANLZ file's bytes into a response.
pub fn parse_anlz(data: &[u8]) -> Result<AnlzResponse> {
    let anlz = RekordboxAnlz::parse(data)?;
    let mut result = AnlzResponse::default();

    for section in &anlz.sections {
        match (&section.tag, &section.body) {
            (SectionTag::BeatGrid, SectionBody::BeatGrid { beats }) => {
                result.dat.beat_grid = Some(make_beat_grid(beats));
            }
            (SectionTag::Cues, SectionBody::Cues { cues, .. }) => {
                result.dat.cue_and_loops = Some(make_cue_and_loop(cues));
            }
            (SectionTag::Cues2, SectionBody::Cues2 { cues, .. }) => {
                result.ext.extended_cues = Some(make_extended_cues(cues));
            }
            (SectionTag::WavePreview, SectionBody::WavePreview { data }) => {
                result.dat.waveform_preview = Some(make_waveform_preview(data));
            }
            (SectionTag::WaveTiny, SectionBody::WavePreview { data }) => {
                result.dat.waveform_tiny = Some(make_waveform_preview(data));
            }
            (SectionTag::WaveScroll, SectionBody::WaveScroll { entries, .. }) => {
                result.ext.waveform_detail = Some(entries.clone());
            }
            (SectionTag::WaveColorPreview, SectionBody::WaveColorPreview { entries, .. }) => {
                result.ext.waveform_color_preview = Some(entries.clone());
            }
            (SectionTag::WaveColorScroll, SectionBody::WaveColorScroll { entries, .. }) => {
                result.ext.waveform_hd = Some(make_waveform_hd(entries));
            }
            (SectionTag::SongStructure, SectionBody::SongStructure { mood, end_beat, bank, entries, .. }) => {
                result.ext.song_structure = Some(make_song_structure(*mood, *bank, *end_beat, entries));
            }
            (SectionTag::WaveColor3BandPreview, SectionBody::WaveColor3BandPreview { len_entries, entries, .. }) => {
                result.two_ex.waveform_3band_preview = Some(make_waveform_3band_preview(*len_entries, entries));
            }
            (SectionTag::WaveColor3BandDetail, SectionBody::WaveColor3BandDetail { len_entries, entries, .. }) => {
                result.two_ex.waveform_3band_detail = Some(make_waveform_3band_detail(*len_entries, entries));
            }
            (SectionTag::VocalConfig, body @ SectionBody::VocalConfig { .. }) => {
                result.two_ex.vocal_config = make_vocal_config(body);
            }
            // VBR and PATH tags are defined but not currently extracted as
            // they're not commonly needed in the application.
            _ => {}
        }
    }

    Ok(result)
}

/// Loads the ANLZ data of a track from its `analyze_path`.
///
/// `analyze_path` is the track's analysis path with the extension trimmed
/// (as the hydrators store it); `kind` selects which file to append.
pub async fn load_anlz(analyze_path: &str, kind: AnlzKind, resolver: &AnlzResolver<'_>) -> Result<AnlzResponse> {
    let path = format!("{analyze_path}.{}", kind.extension());
    let data = resolver(path).await?;
    parse_anlz(&data)
}

#[cfg(test)]
mod tests {
    use super::anlz::fixtures::*;
    use super::*;
    use crate::types::Mood;

    #[tokio::test]
    async fn load_anlz_parses_each_kind() {
        let two_ex = file(&[pwv6(4), pwv7(6), pwvc(80, 90, 100)]);
        let ext = file(&[pwv5(3), pssi(1, 0, 64, &[(1, 1, 1, 0, 0)])]);
        let dat = file(&[pqtz(&[(1, 12800, 0)]), pcob(&[(0, 1, 100, 0)])]);

        let resolver = move |path: String| {
            let two_ex = two_ex.clone();
            let ext = ext.clone();
            let dat = dat.clone();
            Box::pin(async move {
                Ok(if path.ends_with(".2EX") {
                    two_ex
                } else if path.ends_with(".EXT") {
                    ext
                } else {
                    dat
                })
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<Vec<u8>>> + Send>>
        };

        let r = load_anlz("PIONEER/USBANLZ/ANLZ0001", AnlzKind::TwoEx, &resolver).await.unwrap();
        assert_eq!(r.two_ex.waveform_3band_preview.as_ref().unwrap().num_entries, 4);
        assert_eq!(r.two_ex.waveform_3band_detail.as_ref().unwrap().data.len(), 18);
        assert_eq!(r.two_ex.vocal_config.unwrap().threshold_high, 100);
        assert!(r.dat.beat_grid.is_none());

        let r = load_anlz("PIONEER/USBANLZ/ANLZ0001", AnlzKind::Ext, &resolver).await.unwrap();
        assert_eq!(r.ext.waveform_hd.as_ref().unwrap().len(), 3);
        assert_eq!(r.ext.song_structure.as_ref().unwrap().mood, Mood::High);

        let r = load_anlz("PIONEER/USBANLZ/ANLZ0001", AnlzKind::Dat, &resolver).await.unwrap();
        assert_eq!(r.dat.beat_grid.as_ref().unwrap().len(), 1);
        assert_eq!(r.dat.cue_and_loops.as_ref().unwrap().len(), 1);
    }
}
