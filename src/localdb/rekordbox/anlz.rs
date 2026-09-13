//! Reader for rekordbox track analysis files (`.DAT`, `.EXT`, `.2EX`).
//!
//! A hand-written port of upstream's `rekordbox_anlz.ksy` Kaitai Struct
//! definition. The file is a `PMAI` header followed by type-tagged sections,
//! each identified by a four-byte magic sequence.

use crate::{Error, Result};

/// The four-byte tags of the sections this crate understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionTag {
    /// PCOB
    Cues,
    /// PCO2 (seen in .EXT)
    Cues2,
    /// PPTH
    Path,
    /// PVBR
    Vbr,
    /// PQTZ
    BeatGrid,
    /// PWAV
    WavePreview,
    /// PWV2
    WaveTiny,
    /// PWV3 (seen in .EXT)
    WaveScroll,
    /// PWV4 (seen in .EXT)
    WaveColorPreview,
    /// PWV5 (seen in .EXT)
    WaveColorScroll,
    /// PSSI (seen in .EXT)
    SongStructure,
    /// PWV6 (seen in .2EX)
    WaveColor3BandPreview,
    /// PWV7 (seen in .2EX)
    WaveColor3BandDetail,
    /// PWVC (seen in .2EX)
    VocalConfig,
    Other(u32),
}

impl SectionTag {
    pub const CUES: u32 = 0x5043_4f42;
    pub const CUES_2: u32 = 0x5043_4f32;
    pub const PATH: u32 = 0x5050_5448;
    pub const VBR: u32 = 0x5056_4252;
    pub const BEAT_GRID: u32 = 0x5051_545a;
    pub const WAVE_PREVIEW: u32 = 0x5057_4156;
    pub const WAVE_TINY: u32 = 0x5057_5632;
    pub const WAVE_SCROLL: u32 = 0x5057_5633;
    pub const WAVE_COLOR_PREVIEW: u32 = 0x5057_5634;
    pub const WAVE_COLOR_SCROLL: u32 = 0x5057_5635;
    pub const SONG_STRUCTURE: u32 = 0x5053_5349;
    pub const WAVE_COLOR_3BAND_PREVIEW: u32 = 0x5057_5636;
    pub const WAVE_COLOR_3BAND_DETAIL: u32 = 0x5057_5637;
    pub const VOCAL_CONFIG: u32 = 0x5057_5643;

    pub const fn from_u32(v: u32) -> Self {
        match v {
            Self::CUES => SectionTag::Cues,
            Self::CUES_2 => SectionTag::Cues2,
            Self::PATH => SectionTag::Path,
            Self::VBR => SectionTag::Vbr,
            Self::BEAT_GRID => SectionTag::BeatGrid,
            Self::WAVE_PREVIEW => SectionTag::WavePreview,
            Self::WAVE_TINY => SectionTag::WaveTiny,
            Self::WAVE_SCROLL => SectionTag::WaveScroll,
            Self::WAVE_COLOR_PREVIEW => SectionTag::WaveColorPreview,
            Self::WAVE_COLOR_SCROLL => SectionTag::WaveColorScroll,
            Self::SONG_STRUCTURE => SectionTag::SongStructure,
            Self::WAVE_COLOR_3BAND_PREVIEW => SectionTag::WaveColor3BandPreview,
            Self::WAVE_COLOR_3BAND_DETAIL => SectionTag::WaveColor3BandDetail,
            Self::VOCAL_CONFIG => SectionTag::VocalConfig,
            other => SectionTag::Other(other),
        }
    }
}

/// Describes an individual beat in a beat grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeatGridBeat {
    /// The position of the beat within its musical bar, where beat 1 is the
    /// down beat.
    pub beat_number: u16,
    /// The tempo at the time of this beat, in beats per minute, multiplied by 100.
    pub tempo: u16,
    /// The time, in milliseconds, at which this beat occurs when the track is
    /// played at normal (100%) pitch.
    pub time: u32,
}

/// Identifies whether a cue tag stores ordinary or hot cues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueListType {
    MemoryCues,
    HotCues,
    Other(u32),
}

impl CueListType {
    pub const fn from_u32(v: u32) -> Self {
        match v {
            0 => CueListType::MemoryCues,
            1 => CueListType::HotCues,
            other => CueListType::Other(other),
        }
    }
}

/// A cue list entry (PCPT). Can either represent a memory cue or a loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueEntry {
    /// If zero, this is an ordinary memory cue, otherwise this a hot cue with
    /// the specified number.
    pub hot_cue: u32,
    /// If zero, this entry should be ignored.
    pub status: u32,
    pub order_first: u16,
    pub order_last: u16,
    /// 1 = memory cue, 2 = loop.
    pub cue_type: u8,
    /// The position, in milliseconds, at which the cue point lies in the track.
    pub time: u32,
    /// The position, in milliseconds, at which the player loops back to the
    /// cue time if this is a loop.
    pub loop_time: u32,
}

/// A cue extended list entry (PCP2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueExtendedEntry {
    pub hot_cue: u32,
    /// 1 = memory cue, 2 = loop.
    pub cue_type: u8,
    pub time: u32,
    pub loop_time: u32,
    /// References a row in the colors table if this is a memory cue or loop
    /// and has been assigned a color.
    pub color_id: u8,
    /// The comment assigned to this cue by the DJ, if any.
    pub comment: Option<String>,
    /// The length of the comment in bytes (0 when absent).
    pub len_comment: u32,
    /// A lookup value for a color table, used to index the hot cue colors
    /// shown in rekordbox.
    pub color_code: Option<u8>,
    pub color_red: Option<u8>,
    pub color_green: Option<u8>,
    pub color_blue: Option<u8>,
    /// For quantized loops, the loop size numerator / denominator (the last
    /// four bytes of the reserved area after `color_id`).
    pub loop_numerator: Option<u16>,
    pub loop_denominator: Option<u16>,
}

/// A song structure entry, a single phrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SongStructureEntry {
    /// The absolute number of the phrase, starting at one.
    pub phrase_number: u16,
    /// The beat number at which the phrase starts.
    pub beat_number: u16,
    /// The kind of phrase as displayed in rekordbox; meaning depends on the mood.
    pub kind: u16,
    /// If nonzero, fill-in is present.
    pub fill_in: u8,
    /// The beat number at which fill-in starts.
    pub fill_in_beat_number: u16,
}

/// The body of a section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionBody {
    BeatGrid {
        beats: Vec<BeatGridBeat>,
    },
    Cues {
        list_type: CueListType,
        memory_count: u32,
        cues: Vec<CueEntry>,
    },
    Cues2 {
        list_type: CueListType,
        cues: Vec<CueExtendedEntry>,
    },
    Path {
        path: Option<String>,
    },
    Vbr {
        index: Vec<u32>,
    },
    /// PWAV / PWV2. `data` is empty when the tag carries no preview.
    WavePreview {
        data: Vec<u8>,
    },
    /// PWV3.
    WaveScroll {
        len_entry_bytes: u32,
        len_entries: u32,
        entries: Vec<u8>,
    },
    /// PWV4.
    WaveColorPreview {
        len_entry_bytes: u32,
        len_entries: u32,
        entries: Vec<u8>,
    },
    /// PWV5.
    WaveColorScroll {
        len_entry_bytes: u32,
        len_entries: u32,
        entries: Vec<u8>,
    },
    SongStructure {
        len_entry_bytes: u32,
        len_entries: u16,
        /// 1 high, 2 mid, 3 low.
        mood: u16,
        end_beat: u16,
        bank: u8,
        entries: Vec<SongStructureEntry>,
    },
    /// PWV6.
    WaveColor3BandPreview {
        len_entry_bytes: u32,
        len_entries: u32,
        entries: Vec<u8>,
    },
    /// PWV7.
    WaveColor3BandDetail {
        len_entry_bytes: u32,
        len_entries: u32,
        entries: Vec<u8>,
    },
    VocalConfig {
        threshold_low: u16,
        threshold_mid: u16,
        threshold_high: u16,
    },
    Unknown,
}

/// A type-tagged file section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedSection {
    pub tag: SectionTag,
    pub fourcc: u32,
    pub len_header: u32,
    pub len_tag: u32,
    pub body: SectionBody,
}

/// A parsed analysis file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RekordboxAnlz {
    pub len_header: u32,
    pub len_file: u32,
    pub sections: Vec<TaggedSection>,
}

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.d.len().saturating_sub(self.pos)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let out =
            self.d.get(self.pos..self.pos + n).ok_or_else(|| Error::parse(format!("anlz: read past end at {:#x}", self.pos)))?;
        self.pos += n;
        Ok(out)
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

fn utf16be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

impl RekordboxAnlz {
    /// Parse an analysis file.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        if r.bytes(4)? != b"PMAI" {
            return Err(Error::parse("anlz: missing PMAI magic"));
        }
        let len_header = r.u32()?;
        let len_file = r.u32()?;
        r.pos = (len_header as usize).min(data.len());

        let mut sections = Vec::new();
        while r.remaining() >= 12 {
            let start = r.pos;
            let fourcc = r.u32()?;
            let section_len_header = r.u32()?;
            let len_tag = r.u32()?;
            if len_tag < 12 {
                return Err(Error::parse(format!("anlz: section at {start:#x} has length {len_tag}")));
            }
            let body_len = (len_tag as usize - 12).min(r.remaining());
            let body_bytes = r.bytes(body_len)?;
            let tag = SectionTag::from_u32(fourcc);
            let body = match parse_body(tag, body_bytes, section_len_header, len_tag) {
                Ok(body) => body,
                Err(e) => {
                    tracing::debug!(target: "alphatheta_connect", "anlz: section {fourcc:#x} at {start:#x} did not parse: {e}");
                    SectionBody::Unknown
                }
            };
            sections.push(TaggedSection { tag, fourcc, len_header: section_len_header, len_tag, body });
        }

        Ok(Self { len_header, len_file, sections })
    }

    /// The first section with the given tag.
    pub fn section(&self, tag: SectionTag) -> Option<&TaggedSection> {
        self.sections.iter().find(|s| s.tag == tag)
    }
}

fn parse_body(tag: SectionTag, b: &[u8], len_header: u32, len_tag: u32) -> Result<SectionBody> {
    let mut r = Reader::new(b);
    Ok(match tag {
        SectionTag::BeatGrid => {
            r.u32()?;
            r.u32()?; // @flesniak says this is always 0x80000
            let len_beats = r.u32()?;
            let mut beats = Vec::with_capacity((len_beats as usize).min(r.remaining() / 8));
            for _ in 0..len_beats {
                beats.push(BeatGridBeat { beat_number: r.u16()?, tempo: r.u16()?, time: r.u32()? });
            }
            SectionBody::BeatGrid { beats }
        }
        SectionTag::Cues => {
            let list_type = CueListType::from_u32(r.u32()?);
            r.skip(2)?;
            let len_cues = r.u16()?;
            let memory_count = r.u32()?;
            let mut cues = Vec::with_capacity(usize::from(len_cues));
            for _ in 0..len_cues {
                let entry_start = r.pos;
                if r.bytes(4)? != b"PCPT" {
                    return Err(Error::parse("anlz: cue entry missing PCPT magic"));
                }
                let _len_header = r.u32()?;
                let len_entry = r.u32()?;
                let hot_cue = r.u32()?;
                let status = r.u32()?;
                r.u32()?; // Seems to always be 0x10000
                let order_first = r.u16()?;
                let order_last = r.u16()?;
                let cue_type = r.u8()?;
                r.skip(3)?; // seems to always be 1000
                let time = r.u32()?;
                let loop_time = r.u32()?;
                r.skip(16)?;
                cues.push(CueEntry { hot_cue, status, order_first, order_last, cue_type, time, loop_time });
                // Entries are fixed size, but trust the declared length.
                r.pos = entry_start + (len_entry as usize).max(56);
            }
            SectionBody::Cues { list_type, memory_count, cues }
        }
        SectionTag::Cues2 => {
            let list_type = CueListType::from_u32(r.u32()?);
            let len_cues = r.u16()?;
            r.skip(2)?;
            let mut cues = Vec::with_capacity(usize::from(len_cues));
            for _ in 0..len_cues {
                let entry_start = r.pos;
                if r.bytes(4)? != b"PCP2" {
                    return Err(Error::parse("anlz: extended cue entry missing PCP2 magic"));
                }
                let _len_header = r.u32()?;
                let len_entry = r.u32()?;
                let hot_cue = r.u32()?;
                let cue_type = r.u8()?;
                r.skip(3)?;
                let time = r.u32()?;
                let loop_time = r.u32()?;
                let color_id = r.u8()?;
                // Loops seem to have some non-zero values in the last four
                // bytes of this: the quantized loop numerator / denominator.
                let reserved = r.bytes(11)?;
                let loop_numerator = u16::from_be_bytes([reserved[7], reserved[8]]);
                let loop_denominator = u16::from_be_bytes([reserved[9], reserved[10]]);

                let mut len_comment = 0u32;
                let mut comment = None;
                if len_entry > 43 {
                    len_comment = r.u32()?;
                    let text = r.bytes(len_comment as usize)?;
                    // The tag stores the comment with a trailing NUL, which is
                    // not part of what the DJ typed.
                    comment = Some(utf16be(text).trim_end_matches('\0').to_string());
                }
                let after_comment = len_entry.saturating_sub(len_comment);
                let color_code = if after_comment > 44 { Some(r.u8()?) } else { None };
                let color_red = if after_comment > 45 { Some(r.u8()?) } else { None };
                let color_green = if after_comment > 46 { Some(r.u8()?) } else { None };
                let color_blue = if after_comment > 47 { Some(r.u8()?) } else { None };

                cues.push(CueExtendedEntry {
                    hot_cue,
                    cue_type,
                    time,
                    loop_time,
                    color_id,
                    comment,
                    len_comment,
                    color_code,
                    color_red,
                    color_green,
                    color_blue,
                    loop_numerator: Some(loop_numerator),
                    loop_denominator: Some(loop_denominator),
                });
                r.pos = entry_start + len_entry as usize;
            }
            SectionBody::Cues2 { list_type, cues }
        }
        SectionTag::Path => {
            let len_path = r.u32()?;
            let path = if len_path > 1 { Some(utf16be(r.bytes(len_path as usize - 2)?)) } else { None };
            SectionBody::Path { path }
        }
        SectionTag::Vbr => {
            r.u32()?;
            let mut index = Vec::with_capacity(400);
            for _ in 0..400 {
                index.push(r.u32()?);
            }
            SectionBody::Vbr { index }
        }
        SectionTag::WavePreview | SectionTag::WaveTiny => {
            let len_preview = r.u32()?;
            r.u32()?; // This seems to always have the value 0x10000
            let data = if len_tag > len_header { r.bytes(len_preview as usize)?.to_vec() } else { Vec::new() };
            SectionBody::WavePreview { data }
        }
        SectionTag::WaveScroll
        | SectionTag::WaveColorPreview
        | SectionTag::WaveColorScroll
        | SectionTag::WaveColor3BandDetail => {
            let len_entry_bytes = r.u32()?;
            let len_entries = r.u32()?;
            r.u32()?;
            let entries = r.bytes((len_entry_bytes as usize).saturating_mul(len_entries as usize))?.to_vec();
            match tag {
                SectionTag::WaveScroll => SectionBody::WaveScroll { len_entry_bytes, len_entries, entries },
                SectionTag::WaveColorPreview => SectionBody::WaveColorPreview { len_entry_bytes, len_entries, entries },
                SectionTag::WaveColorScroll => SectionBody::WaveColorScroll { len_entry_bytes, len_entries, entries },
                _ => SectionBody::WaveColor3BandDetail { len_entry_bytes, len_entries, entries },
            }
        }
        SectionTag::WaveColor3BandPreview => {
            let len_entry_bytes = r.u32()?;
            let len_entries = r.u32()?;
            let entries = r.bytes((len_entry_bytes as usize).saturating_mul(len_entries as usize))?.to_vec();
            SectionBody::WaveColor3BandPreview { len_entry_bytes, len_entries, entries }
        }
        SectionTag::SongStructure => {
            let len_entry_bytes = r.u32()?;
            let len_entries = r.u16()?;
            // The rest of the tag needs to be unmasked before it can be parsed.
            let mut body = r.bytes(r.remaining())?.to_vec();
            let mask = song_structure_mask(len_entries);
            for (i, byte) in body.iter_mut().enumerate() {
                *byte ^= mask[i % mask.len()];
            }
            let mut br = Reader::new(&body);
            let mood = br.u16()?;
            br.skip(6)?;
            let end_beat = br.u16()?;
            br.skip(2)?;
            let bank = br.u8()?;
            br.skip(1)?;
            let mut entries = Vec::with_capacity(usize::from(len_entries));
            let gap = (len_entry_bytes as usize).saturating_sub(9);
            for _ in 0..len_entries {
                let phrase_number = br.u16()?;
                let beat_number = br.u16()?;
                let kind = br.u16()?;
                br.skip(gap)?;
                let fill_in = br.u8()?;
                let fill_in_beat_number = br.u16()?;
                entries.push(SongStructureEntry { phrase_number, beat_number, kind, fill_in, fill_in_beat_number });
            }
            SectionBody::SongStructure { len_entry_bytes, len_entries, mood, end_beat, bank, entries }
        }
        SectionTag::VocalConfig => {
            r.u16()?; // unknown, always 0
            SectionBody::VocalConfig { threshold_low: r.u16()?, threshold_mid: r.u16()?, threshold_high: r.u16()? }
        }
        SectionTag::Other(_) => SectionBody::Unknown,
    })
}

/// The XOR mask rekordbox applies to the song structure body, keyed on the
/// phrase count.
fn song_structure_mask(len_entries: u16) -> [u8; 19] {
    let c = len_entries as u8;
    let base: [u8; 19] =
        [0xCB, 0xE1, 0xEE, 0xFA, 0xE5, 0xEE, 0xAD, 0xEE, 0xE9, 0xD2, 0xE9, 0xEB, 0xE1, 0xE9, 0xF3, 0xE8, 0xE9, 0xF4, 0xE1];
    let mut mask = [0u8; 19];
    for (m, b) in mask.iter_mut().zip(base) {
        *m = b.wrapping_add(c);
    }
    mask
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Builders for synthetic ANLZ files, ported from upstream's test fixtures.

    pub fn section(fourcc: u32, body: &[u8]) -> Vec<u8> {
        let mut s = Vec::with_capacity(12 + body.len());
        s.extend_from_slice(&fourcc.to_be_bytes());
        s.extend_from_slice(&12u32.to_be_bytes());
        s.extend_from_slice(&((12 + body.len()) as u32).to_be_bytes());
        s.extend_from_slice(body);
        s
    }

    pub fn file(sections: &[Vec<u8>]) -> Vec<u8> {
        let total: usize = 12 + sections.iter().map(Vec::len).sum::<usize>();
        let mut f = Vec::with_capacity(total);
        f.extend_from_slice(b"PMAI");
        f.extend_from_slice(&12u32.to_be_bytes());
        f.extend_from_slice(&(total as u32).to_be_bytes());
        for s in sections {
            f.extend_from_slice(s);
        }
        f
    }

    pub fn pwv6(num_entries: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&3u32.to_be_bytes());
        body.extend_from_slice(&num_entries.to_be_bytes());
        for i in 0..num_entries * 3 {
            body.push((i % 256) as u8);
        }
        section(super::SectionTag::WAVE_COLOR_3BAND_PREVIEW, &body)
    }

    pub fn pwv7(num_entries: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&3u32.to_be_bytes());
        body.extend_from_slice(&num_entries.to_be_bytes());
        body.extend_from_slice(&0x0096_0000u32.to_be_bytes());
        for i in 0..num_entries * 3 {
            body.push((i % 256) as u8);
        }
        section(super::SectionTag::WAVE_COLOR_3BAND_DETAIL, &body)
    }

    pub fn pwvc(low: u16, mid: u16, high: u16) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0u16.to_be_bytes());
        body.extend_from_slice(&low.to_be_bytes());
        body.extend_from_slice(&mid.to_be_bytes());
        body.extend_from_slice(&high.to_be_bytes());
        section(super::SectionTag::VOCAL_CONFIG, &body)
    }

    pub fn pwv5(num_entries: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&2u32.to_be_bytes());
        body.extend_from_slice(&num_entries.to_be_bytes());
        body.extend_from_slice(&0x0096_0000u32.to_be_bytes());
        for i in 0..num_entries {
            // red 7, green 0, blue 7, height (i % 32)
            let word: u16 = (0b111 << 13) | (0b111 << 7) | (((i % 32) as u16) << 2);
            body.extend_from_slice(&word.to_be_bytes());
        }
        section(super::SectionTag::WAVE_COLOR_SCROLL, &body)
    }

    pub fn pqtz(beats: &[(u16, u16, u32)]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&0x0008_0000u32.to_be_bytes());
        body.extend_from_slice(&(beats.len() as u32).to_be_bytes());
        for (n, tempo, time) in beats {
            body.extend_from_slice(&n.to_be_bytes());
            body.extend_from_slice(&tempo.to_be_bytes());
            body.extend_from_slice(&time.to_be_bytes());
        }
        section(super::SectionTag::BEAT_GRID, &body)
    }

    /// A PCOB tag with the given (hot_cue, type, time, loop_time) entries.
    pub fn pcob(cues: &[(u32, u8, u32, u32)]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(&(cues.len() as u16).to_be_bytes());
        body.extend_from_slice(&0u32.to_be_bytes());
        for (hot_cue, cue_type, time, loop_time) in cues {
            let mut e = Vec::new();
            e.extend_from_slice(b"PCPT");
            e.extend_from_slice(&0x1cu32.to_be_bytes());
            e.extend_from_slice(&56u32.to_be_bytes());
            e.extend_from_slice(&hot_cue.to_be_bytes());
            e.extend_from_slice(&1u32.to_be_bytes());
            e.extend_from_slice(&0x0001_0000u32.to_be_bytes());
            e.extend_from_slice(&0xffffu16.to_be_bytes());
            e.extend_from_slice(&0xffffu16.to_be_bytes());
            e.push(*cue_type);
            e.extend_from_slice(&[0, 0x03, 0xe8]);
            e.extend_from_slice(&time.to_be_bytes());
            e.extend_from_slice(&loop_time.to_be_bytes());
            e.extend_from_slice(&[0u8; 16]);
            body.extend_from_slice(&e);
        }
        section(super::SectionTag::CUES, &body)
    }

    /// A PCO2 tag with one entry carrying a comment and hot cue colour.
    pub fn pco2_with_comment(
        hot_cue: u32,
        cue_type: u8,
        time: u32,
        loop_time: u32,
        color_id: u8,
        comment: &str,
        rgb: (u8, u8, u8, u8),
    ) -> Vec<u8> {
        let mut comment_bytes = Vec::new();
        for u in comment.encode_utf16() {
            comment_bytes.extend_from_slice(&u.to_be_bytes());
        }
        comment_bytes.extend_from_slice(&[0, 0]);
        let len_comment = comment_bytes.len() as u32;
        let len_entry = 48 + len_comment;

        let mut e = Vec::new();
        e.extend_from_slice(b"PCP2");
        e.extend_from_slice(&0x2cu32.to_be_bytes());
        e.extend_from_slice(&len_entry.to_be_bytes());
        e.extend_from_slice(&hot_cue.to_be_bytes());
        e.push(cue_type);
        e.extend_from_slice(&[0, 0x03, 0xe8]);
        e.extend_from_slice(&time.to_be_bytes());
        e.extend_from_slice(&loop_time.to_be_bytes());
        e.push(color_id);
        e.extend_from_slice(&[0u8; 7]);
        e.extend_from_slice(&4u16.to_be_bytes()); // loop numerator
        e.extend_from_slice(&1u16.to_be_bytes()); // loop denominator
        e.extend_from_slice(&len_comment.to_be_bytes());
        e.extend_from_slice(&comment_bytes);
        e.extend_from_slice(&[rgb.0, rgb.1, rgb.2, rgb.3]);
        assert_eq!(e.len(), len_entry as usize);

        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&1u16.to_be_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(&e);
        section(super::SectionTag::CUES_2, &body)
    }

    /// A PSSI tag, masked the way rekordbox writes it.
    pub fn pssi(mood: u16, bank: u8, end_beat: u16, phrases: &[(u16, u16, u16, u8, u16)]) -> Vec<u8> {
        let len_entry_bytes = 24u32;
        let mut plain = Vec::new();
        plain.extend_from_slice(&mood.to_be_bytes());
        plain.extend_from_slice(&[0u8; 6]);
        plain.extend_from_slice(&end_beat.to_be_bytes());
        plain.extend_from_slice(&[0u8; 2]);
        plain.push(bank);
        plain.push(0);
        for (n, beat, kind, fill, fill_beat) in phrases {
            plain.extend_from_slice(&n.to_be_bytes());
            plain.extend_from_slice(&beat.to_be_bytes());
            plain.extend_from_slice(&kind.to_be_bytes());
            plain.extend_from_slice(&[0u8; 15]);
            plain.push(*fill);
            plain.extend_from_slice(&fill_beat.to_be_bytes());
        }
        let mask = super::song_structure_mask(phrases.len() as u16);
        for (i, b) in plain.iter_mut().enumerate() {
            *b ^= mask[i % mask.len()];
        }
        let mut body = Vec::new();
        body.extend_from_slice(&len_entry_bytes.to_be_bytes());
        body.extend_from_slice(&(phrases.len() as u16).to_be_bytes());
        body.extend_from_slice(&plain);
        section(super::SectionTag::SONG_STRUCTURE, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn header_and_empty_file() {
        let f = file(&[]);
        assert_eq!(f.len(), 12);
        let anlz = RekordboxAnlz::parse(&f).unwrap();
        assert!(anlz.sections.is_empty());
        assert_eq!(anlz.len_file, 12);
        assert!(RekordboxAnlz::parse(b"NOPE").is_err());
    }

    #[test]
    fn parses_2ex_sections() {
        let f = file(&[pwv6(10), pwv7(20), pwvc(90, 100, 110)]);
        let anlz = RekordboxAnlz::parse(&f).unwrap();
        assert_eq!(anlz.sections.len(), 3);
        match &anlz.sections[0].body {
            SectionBody::WaveColor3BandPreview { len_entry_bytes, len_entries, entries } => {
                assert_eq!((*len_entry_bytes, *len_entries, entries.len()), (3, 10, 30));
            }
            other => panic!("{other:?}"),
        }
        match &anlz.sections[1].body {
            SectionBody::WaveColor3BandDetail { len_entries, entries, .. } => {
                assert_eq!((*len_entries, entries.len()), (20, 60));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            anlz.sections[2].body,
            SectionBody::VocalConfig { threshold_low: 90, threshold_mid: 100, threshold_high: 110 }
        );
    }

    #[test]
    fn parses_beat_grid_and_cues() {
        let f = file(&[pqtz(&[(1, 12800, 0), (2, 12800, 468)]), pcob(&[(0, 1, 1000, 0), (2, 2, 2000, 3000)])]);
        let anlz = RekordboxAnlz::parse(&f).unwrap();
        match &anlz.sections[0].body {
            SectionBody::BeatGrid { beats } => {
                assert_eq!(beats.len(), 2);
                assert_eq!(beats[1], BeatGridBeat { beat_number: 2, tempo: 12800, time: 468 });
            }
            other => panic!("{other:?}"),
        }
        match &anlz.sections[1].body {
            SectionBody::Cues { cues, .. } => {
                assert_eq!(cues.len(), 2);
                assert_eq!(cues[1].hot_cue, 2);
                assert_eq!(cues[1].cue_type, 2);
                assert_eq!(cues[1].loop_time, 3000);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parses_extended_cues_with_comment_and_color() {
        let f = file(&[pco2_with_comment(1, 2, 5000, 7000, 3, "Drop", (0x2a, 255, 0, 0))]);
        let anlz = RekordboxAnlz::parse(&f).unwrap();
        match &anlz.sections[0].body {
            SectionBody::Cues2 { cues, .. } => {
                let c = &cues[0];
                assert_eq!(c.hot_cue, 1);
                assert_eq!(c.cue_type, 2);
                assert_eq!(c.time, 5000);
                assert_eq!(c.loop_time, 7000);
                assert_eq!(c.color_id, 3);
                assert_eq!(c.comment.as_deref(), Some("Drop"));
                assert_eq!(c.color_code, Some(0x2a));
                assert_eq!((c.color_red, c.color_green, c.color_blue), (Some(255), Some(0), Some(0)));
                assert_eq!(c.loop_numerator, Some(4));
                assert_eq!(c.loop_denominator, Some(1));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unmasks_song_structure() {
        let f = file(&[pssi(2, 3, 128, &[(1, 1, 1, 0, 0), (2, 33, 9, 1, 60)])]);
        let anlz = RekordboxAnlz::parse(&f).unwrap();
        match &anlz.sections[0].body {
            SectionBody::SongStructure { mood, end_beat, bank, entries, .. } => {
                assert_eq!((*mood, *end_beat, *bank), (2, 128, 3));
                assert_eq!(entries.len(), 2);
                assert_eq!(
                    entries[1],
                    SongStructureEntry { phrase_number: 2, beat_number: 33, kind: 9, fill_in: 1, fill_in_beat_number: 60 }
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_sections_are_kept_as_unknown() {
        let f = file(&[section(0x5858_5858, &[1, 2, 3])]);
        let anlz = RekordboxAnlz::parse(&f).unwrap();
        assert_eq!(anlz.sections[0].tag, SectionTag::Other(0x5858_5858));
        assert_eq!(anlz.sections[0].body, SectionBody::Unknown);
    }
}
