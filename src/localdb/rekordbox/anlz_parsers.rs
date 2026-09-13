//! Converters from parsed ANLZ sections to the crate's public types.

use crate::entities::{CueAndLoop, HotcueButton};
use crate::localdb::rekordbox::anlz::{CueEntry, CueExtendedEntry, SectionBody, SongStructureEntry};
use crate::localdb::utils::make_cue_loop_entry;
use crate::types::{
    Bank, Beat, BeatGrid, ExtendedCue, Mood, Phrase, Rgb, SongStructure, VocalConfig, Waveform3BandDetail, Waveform3BandPreview,
    WaveformHD, WaveformPreviewData,
};
use crate::utils::converters::convert_waveform_hd_data;

/// Fill beatgrid data from the ANLZ section.
pub fn make_beat_grid(beats: &[crate::localdb::rekordbox::anlz::BeatGridBeat]) -> BeatGrid {
    beats.iter().map(|b| Beat { offset: b.time, bpm: f64::from(b.tempo) / 100.0, count: b.beat_number.min(255) as u8 }).collect()
}

/// Fill cue and loop data from the ANLZ section.
pub fn make_cue_and_loop(cues: &[CueEntry]) -> Vec<CueAndLoop> {
    cues.iter()
        .filter_map(|entry| {
            // Cues with the status 0 are likely leftovers that were removed.
            // Upstream keys the hot cue button off `type` (1/2) once hot_cue
            // is non-zero; the button number itself is `hot_cue`.
            let button = if entry.hot_cue == 0 { None } else { HotcueButton::from_u8(entry.hot_cue.min(255) as u8) };
            let is_cue = entry.cue_type == 0x01;
            let is_loop = entry.cue_type == 0x02;

            // NOTE: Unlike the remotedb, these entries are already in milliseconds.
            let offset = f64::from(entry.time);
            let length = f64::from(entry.loop_time) - offset;

            make_cue_loop_entry(is_cue, is_loop, offset, length, button)
        })
        .collect()
}

/// Fill waveform HD data from the ANLZ section.
pub fn make_waveform_hd(entries: &[u8]) -> WaveformHD {
    convert_waveform_hd_data(entries)
}

/// Parse extended cues (PCO2) with colors and comments.
pub fn make_extended_cues(cues: &[CueExtendedEntry]) -> Vec<ExtendedCue> {
    cues.iter()
        .map(|entry| {
            let mut cue = ExtendedCue {
                hot_cue: entry.hot_cue,
                cue_type: entry.cue_type,
                time: entry.time,
                loop_time: None,
                color_id: None,
                color_code: None,
                color_rgb: None,
                comment: None,
                loop_numerator: None,
                loop_denominator: None,
            };

            // Add loop end time if this is a loop
            if entry.cue_type == 2 {
                cue.loop_time = Some(entry.loop_time);
            }

            // Add color ID for memory points/loops
            if entry.color_id > 0 {
                cue.color_id = Some(entry.color_id);
            }

            // Add hot cue color information
            if let Some(code) = entry.color_code.filter(|c| *c > 0) {
                cue.color_code = Some(code);
                cue.color_rgb = Some(Rgb {
                    r: entry.color_red.unwrap_or(0),
                    g: entry.color_green.unwrap_or(0),
                    b: entry.color_blue.unwrap_or(0),
                });
            }

            // Add comment if present
            if entry.len_comment > 0 {
                if let Some(c) = entry.comment.as_ref().filter(|c| !c.is_empty()) {
                    cue.comment = Some(c.clone());
                }
            }

            // Add quantized loop information if present
            if let Some(n) = entry.loop_numerator.filter(|n| *n > 0) {
                cue.loop_numerator = Some(n);
                cue.loop_denominator = Some(entry.loop_denominator.unwrap_or(1));
            }

            cue
        })
        .collect()
}

fn mood_from_u16(v: u16) -> Mood {
    match v {
        2 => Mood::Mid,
        3 => Mood::Low,
        // 1 and anything unknown
        _ => Mood::High,
    }
}

fn bank_from_u8(v: u8) -> Bank {
    match v {
        1 => Bank::Cool,
        2 => Bank::Natural,
        3 => Bank::Hot,
        4 => Bank::Subtle,
        5 => Bank::Warm,
        6 => Bank::Vivid,
        7 => Bank::Club1,
        8 => Bank::Club2,
        _ => Bank::Default,
    }
}

/// The human-readable phrase type for a kind in a mood.
fn phrase_type(mood: Mood, kind: u16) -> &'static str {
    match (mood, kind) {
        (Mood::High, 1) => "Intro",
        (Mood::High, 2) => "Up",
        (Mood::High, 3) => "Down",
        (Mood::High, 5) => "Chorus",
        (Mood::High, 6) => "Outro",

        (Mood::Mid, 1) => "Intro",
        (Mood::Mid, 2) => "Verse 1",
        (Mood::Mid, 3) => "Verse 2",
        (Mood::Mid, 4) => "Verse 3",
        (Mood::Mid, 5) => "Verse 4",
        (Mood::Mid, 6) => "Verse 5",
        (Mood::Mid, 7) => "Verse 6",
        (Mood::Mid, 8) => "Bridge",
        (Mood::Mid, 9) => "Chorus",
        (Mood::Mid, 10) => "Outro",

        (Mood::Low, 1) => "Intro",
        (Mood::Low, 2..=4) => "Verse 1",
        (Mood::Low, 5..=7) => "Verse 2",
        (Mood::Low, 8) => "Bridge",
        (Mood::Low, 9) => "Chorus",
        (Mood::Low, 10) => "Outro",

        _ => "Unknown",
    }
}

/// Parse song structure (PSSI) with phrase analysis.
pub fn make_song_structure(mood: u16, bank: u8, end_beat: u16, entries: &[SongStructureEntry]) -> SongStructure {
    let mood = mood_from_u16(mood);
    let bank = bank_from_u8(bank);

    let phrases = entries
        .iter()
        .map(|entry| {
            let mut phrase = Phrase {
                index: entry.phrase_number,
                beat: entry.beat_number,
                kind: entry.kind,
                phrase_type: phrase_type(mood, entry.kind).to_string(),
                fill: None,
                fill_beat: None,
            };

            // Add fill-in information if present
            if entry.fill_in > 0 {
                phrase.fill = Some(entry.fill_in);
                phrase.fill_beat = Some(entry.fill_in_beat_number);
            }

            phrase
        })
        .collect();

    SongStructure { mood, bank, end_beat, phrases }
}

/// Parse waveform preview data (PWAV/PWV2).
pub fn make_waveform_preview(data: &[u8]) -> WaveformPreviewData {
    WaveformPreviewData { data: data.to_vec() }
}

/// Parse 3-band color waveform preview (PWV6 from .2EX files).
pub fn make_waveform_3band_preview(len_entries: u32, entries: &[u8]) -> Waveform3BandPreview {
    Waveform3BandPreview { num_entries: len_entries, data: entries.to_vec() }
}

/// Parse 3-band color detail waveform (PWV7 from .2EX files).
pub fn make_waveform_3band_detail(len_entries: u32, entries: &[u8]) -> Waveform3BandDetail {
    Waveform3BandDetail { num_entries: len_entries, data: entries.to_vec() }
}

/// Parse vocal detection config (PWVC from .2EX files).
pub fn make_vocal_config(body: &SectionBody) -> Option<VocalConfig> {
    match body {
        SectionBody::VocalConfig { threshold_low, threshold_mid, threshold_high } => {
            Some(VocalConfig { threshold_low: *threshold_low, threshold_mid: *threshold_mid, threshold_high: *threshold_high })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::CueAndLoop;

    #[test]
    fn cue_and_loops_from_entries() {
        let cues = vec![
            CueEntry { hot_cue: 0, status: 1, order_first: 0, order_last: 0, cue_type: 1, time: 1000, loop_time: 0 },
            CueEntry { hot_cue: 3, status: 1, order_first: 0, order_last: 0, cue_type: 2, time: 2000, loop_time: 3000 },
            CueEntry { hot_cue: 0, status: 0, order_first: 0, order_last: 0, cue_type: 0, time: 0, loop_time: 0 },
        ];
        let out = make_cue_and_loop(&cues);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], CueAndLoop::CuePoint { offset: 1000.0, label: None, color: None });
        assert_eq!(
            out[1],
            CueAndLoop::HotLoop { offset: 2000.0, length: 1000.0, button: HotcueButton::C, label: None, color: None }
        );
    }

    fn cue(hot_cue: u32, cue_type: u8, time: u32, loop_time: u32) -> CueEntry {
        CueEntry { hot_cue, status: 1, order_first: 0, order_last: 0, cue_type, time, loop_time }
    }

    /// The hot cue button comes from `hot_cue`, not from the entry `type`
    /// (which only says cue point vs loop).
    #[test]
    fn hot_cue_button_comes_from_the_hot_cue_field() {
        let out = make_cue_and_loop(&[cue(3, 1, 4000, 0), cue(8, 2, 5000, 6000), cue(1, 1, 7000, 0)]);
        assert_eq!(
            out,
            vec![
                CueAndLoop::HotCue { offset: 4000.0, button: HotcueButton::C, label: None, color: None },
                CueAndLoop::HotLoop { offset: 5000.0, length: 1000.0, button: HotcueButton::H, label: None, color: None },
                CueAndLoop::HotCue { offset: 7000.0, button: HotcueButton::A, label: None, color: None },
            ]
        );
    }

    /// Entries that are neither a cue point nor a loop must not leak into the
    /// returned list.
    #[test]
    fn drops_entries_that_are_neither_a_cue_nor_a_loop() {
        let out = make_cue_and_loop(&[cue(0, 0, 100, 0), cue(0, 1, 200, 0), cue(0, 9, 300, 0)]);
        assert_eq!(out, vec![CueAndLoop::CuePoint { offset: 200.0, label: None, color: None }]);

        assert!(make_cue_and_loop(&[cue(0, 0, 100, 0)]).is_empty());
    }

    fn extended(cue_type: u8, time: u32, loop_time: u32, color_id: u8) -> CueExtendedEntry {
        CueExtendedEntry {
            hot_cue: 0,
            cue_type,
            time,
            loop_time,
            color_id,
            comment: None,
            len_comment: 0,
            color_code: None,
            color_red: None,
            color_green: None,
            color_blue: None,
            loop_numerator: None,
            loop_denominator: None,
        }
    }

    #[test]
    fn extended_cue_carries_the_quantized_loop_size() {
        let mut e = extended(2, 2000, 6000, 5);
        e.loop_numerator = Some(4);
        e.loop_denominator = Some(1);

        let out = make_extended_cues(&[e]);
        assert_eq!(out[0].loop_time, Some(6000));
        assert_eq!(out[0].color_id, Some(5));
        assert_eq!(out[0].loop_numerator, Some(4));
        assert_eq!(out[0].loop_denominator, Some(1));
    }

    #[test]
    fn extended_cue_omits_loop_size_when_not_quantized() {
        let mut e = extended(2, 0, 500, 0);
        e.loop_numerator = Some(0);
        e.loop_denominator = Some(0);

        let out = make_extended_cues(&[e]);
        assert_eq!(out[0].loop_numerator, None);
        assert_eq!(out[0].loop_denominator, None);
    }

    #[test]
    fn extended_cue_keeps_a_real_comment_and_drops_an_empty_one() {
        let mut with_comment = extended(1, 3000, 0, 0);
        with_comment.hot_cue = 3;
        with_comment.len_comment = 10;
        with_comment.comment = Some("Drop".into());
        with_comment.color_code = Some(0x2a);
        with_comment.color_red = Some(255);
        with_comment.color_green = Some(0);
        with_comment.color_blue = Some(0);

        let out = make_extended_cues(&[with_comment]);
        assert_eq!(out[0].comment.as_deref(), Some("Drop"));
        assert_eq!(out[0].color_code, Some(0x2a));
        assert_eq!(out[0].color_rgb, Some(Rgb { r: 255, g: 0, b: 0 }));

        // A comment that was only the trailing NUL decodes to an empty string
        // and is dropped.
        let mut empty = extended(1, 0, 0, 0);
        empty.len_comment = 2;
        empty.comment = Some(String::new());
        assert_eq!(make_extended_cues(&[empty])[0].comment, None);
    }

    /// The phrase index, beat, kind and bank come from the fields the ANLZ
    /// spec declares.
    #[test]
    fn song_structure_reads_phrase_fields() {
        let entries = vec![
            SongStructureEntry { phrase_number: 1, beat_number: 1, kind: 1, fill_in: 0, fill_in_beat_number: 0 },
            SongStructureEntry { phrase_number: 2, beat_number: 65, kind: 2, fill_in: 0, fill_in_beat_number: 0 },
            SongStructureEntry { phrase_number: 3, beat_number: 129, kind: 5, fill_in: 0, fill_in_beat_number: 0 },
        ];
        let s = make_song_structure(1, 0, 256, &entries);

        let got: Vec<_> = s.phrases.iter().map(|p| (p.index, p.beat, p.kind, p.phrase_type.as_str())).collect();
        assert_eq!(got, vec![(1, 1, 1, "Intro"), (2, 65, 2, "Up"), (3, 129, 5, "Chorus")]);
        assert!(s.phrases.iter().all(|p| p.fill.is_none() && p.fill_beat.is_none()));
    }

    #[test]
    fn song_structure_reads_body_fields() {
        let s = make_song_structure(2, 7, 512, &[]);
        assert_eq!(s.mood, Mood::Mid);
        assert_eq!(s.bank, Bank::Club1);
        assert_eq!(s.end_beat, 512);
        assert!(s.phrases.is_empty());
    }

    #[test]
    fn song_structure_labels_by_mood() {
        let entries = vec![
            SongStructureEntry { phrase_number: 1, beat_number: 1, kind: 1, fill_in: 0, fill_in_beat_number: 0 },
            SongStructureEntry { phrase_number: 2, beat_number: 65, kind: 9, fill_in: 1, fill_in_beat_number: 120 },
            SongStructureEntry { phrase_number: 3, beat_number: 129, kind: 42, fill_in: 0, fill_in_beat_number: 0 },
        ];
        let s = make_song_structure(3, 7, 200, &entries);
        assert_eq!(s.mood, Mood::Low);
        assert_eq!(s.bank, Bank::Club1);
        assert_eq!(s.end_beat, 200);
        assert_eq!(s.phrases[0].phrase_type, "Intro");
        assert_eq!(s.phrases[1].phrase_type, "Chorus");
        assert_eq!(s.phrases[1].fill, Some(1));
        assert_eq!(s.phrases[1].fill_beat, Some(120));
        assert_eq!(s.phrases[2].phrase_type, "Unknown");
        assert_eq!(make_song_structure(9, 9, 0, &[]).mood, Mood::High);
        assert_eq!(make_song_structure(9, 9, 0, &[]).bank, Bank::Default);
    }
}
