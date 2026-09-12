//! Reader for the DeviceSQL database export (`export.pdb`) rekordbox writes
//! to USB and SD media.
//!
//! A hand-written port of upstream's `rekordbox_pdb.ksy` Kaitai Struct
//! definition (by @henrybetts, @flesniak and Deep Symmetry). The file is
//! divided into fixed-size pages; the first page lists the tables, each a
//! linked list of pages whose rows are located through an index built
//! backwards from the end of the page.

use crate::{Error, Result};

/// The kind of rows a table holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageType {
    Tracks,
    Genres,
    Artists,
    Albums,
    Labels,
    Keys,
    Colors,
    PlaylistTree,
    PlaylistEntries,
    Unknown9,
    Unknown10,
    /// The rows all seem to have history file names in them, such as "HISTORY 001".
    Unknown11,
    Unknown12,
    Artwork,
    Unknown14,
    Unknown15,
    Columns,
    Unknown17,
    Unknown18,
    History,
    Other(u32),
}

impl PageType {
    pub const fn from_u32(v: u32) -> Self {
        match v {
            0 => PageType::Tracks,
            1 => PageType::Genres,
            2 => PageType::Artists,
            3 => PageType::Albums,
            4 => PageType::Labels,
            5 => PageType::Keys,
            6 => PageType::Colors,
            7 => PageType::PlaylistTree,
            8 => PageType::PlaylistEntries,
            9 => PageType::Unknown9,
            10 => PageType::Unknown10,
            11 => PageType::Unknown11,
            12 => PageType::Unknown12,
            13 => PageType::Artwork,
            14 => PageType::Unknown14,
            15 => PageType::Unknown15,
            16 => PageType::Columns,
            17 => PageType::Unknown17,
            18 => PageType::Unknown18,
            19 => PageType::History,
            other => PageType::Other(other),
        }
    }
}

/// A table header from the first page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub page_type: PageType,
    pub empty_candidate: u32,
    /// Index of the first page of the table. The first page seems to always
    /// contain similar garbage patterns and zero rows, but the next page it
    /// links to contains the start of the meaningful data rows.
    pub first_page: u32,
    /// Index of the last page that makes up this table.
    pub last_page: u32,
}

/// A row that holds an id and a name (genres, artists, albums, labels, keys,
/// colors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdNameRow {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkRow {
    pub id: u32,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistTreeRow {
    /// The ID of the playlist tree row in which this one can be found, or
    /// `0` if this playlist exists at the root level.
    pub parent_id: u32,
    pub sort_order: u32,
    pub id: u32,
    /// Has a non-zero value if this is actually a folder rather than a playlist.
    pub raw_is_folder: u32,
    pub name: String,
}

impl PlaylistTreeRow {
    pub fn is_folder(&self) -> bool {
        self.raw_is_folder != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntryRow {
    /// The position within the playlist represented by this entry.
    pub entry_index: u32,
    pub track_id: u32,
    pub playlist_id: u32,
}

/// A row that describes a track that can be played.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackRow {
    pub index_shift: u16,
    pub bitmask: u32,
    pub sample_rate: u32,
    pub composer_id: u32,
    pub file_size: u32,
    pub artwork_id: u32,
    pub key_id: u32,
    pub original_artist_id: u32,
    pub label_id: u32,
    pub remixer_id: u32,
    pub bitrate: u32,
    pub track_number: u32,
    /// Beats per minute, multiplied by 100.
    pub tempo: u32,
    pub genre_id: u32,
    pub album_id: u32,
    pub artist_id: u32,
    pub id: u32,
    pub disc_number: u16,
    pub play_count: u16,
    pub year: u16,
    pub sample_depth: u16,
    /// Seconds.
    pub duration: u16,
    pub color_id: u8,
    pub rating: u8,
    pub isrc: String,
    pub texter: String,
    pub unknown_string_2: String,
    pub unknown_string_3: String,
    pub unknown_string_4: String,
    pub message: String,
    /// Always either empty or "ON".
    pub kuvo_public: String,
    /// Always either empty or "ON".
    pub autoload_hotcues: String,
    pub unknown_string_5: String,
    pub unknown_string_6: String,
    pub date_added: String,
    pub release_date: String,
    pub mix_name: String,
    pub unknown_string_7: String,
    pub analyze_path: String,
    pub analyze_date: String,
    pub comment: String,
    pub title: String,
    pub unknown_string_8: String,
    pub filename: String,
    pub file_path: String,
}

/// A parsed row of any known table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Track(Box<TrackRow>),
    Genre(IdNameRow),
    Artist(IdNameRow),
    Album(IdNameRow),
    Label(IdNameRow),
    Key(IdNameRow),
    Color(IdNameRow),
    PlaylistTree(PlaylistTreeRow),
    PlaylistEntry(PlaylistEntryRow),
    Artwork(ArtworkRow),
}

/// A parsed `export.pdb`.
#[derive(Debug)]
pub struct RekordboxPdb<'a> {
    data: &'a [u8],
    /// The database page size, in bytes.
    pub len_page: u32,
    pub num_tables: u32,
    pub next_unused_page: u32,
    pub sequence: u32,
    pub tables: Vec<Table>,
}

fn u16_le(d: &[u8], at: usize) -> Result<u16> {
    d.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| Error::parse(format!("pdb: read past end at {at:#x}")))
}

fn u32_le(d: &[u8], at: usize) -> Result<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| Error::parse(format!("pdb: read past end at {at:#x}")))
}

fn u8_at(d: &[u8], at: usize) -> Result<u8> {
    d.get(at).copied().ok_or_else(|| Error::parse(format!("pdb: read past end at {at:#x}")))
}

/// A variable length string which can be stored in a variety of different
/// encodings.
fn device_sql_string(d: &[u8], at: usize) -> Result<String> {
    let length_and_kind = u8_at(d, at)?;
    match length_and_kind {
        // An ASCII-encoded string preceded by a two-byte length field.
        0x40 => {
            let length = usize::from(u16_le(d, at + 1)?);
            let text = d.get(at + 3..at + 3 + length).ok_or_else(|| Error::parse("pdb: long ascii string past end"))?;
            Ok(String::from_utf8_lossy(text).into_owned())
        }
        // A UTF-16LE-encoded string preceded by a two-byte length field.
        0x90 => {
            let length = usize::from(u16_le(d, at + 1)?);
            let text_len = length.saturating_sub(4);
            let text = d.get(at + 4..at + 4 + text_len).ok_or_else(|| Error::parse("pdb: long utf16 string past end"))?;
            let units: Vec<u16> = text.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            Ok(String::from_utf16_lossy(&units))
        }
        // An ASCII-encoded string up to 127 bytes long. The length byte
        // contains the actual length, incremented, doubled, and incremented
        // again.
        mangled => {
            if mangled % 2 == 0 {
                // Skip invalid strings
                return Ok(String::new());
            }
            let length = (i32::from(mangled) - 1) / 2 - 1;
            if length < 0 {
                return Ok(String::new());
            }
            let length = length as usize;
            let text = d.get(at + 1..at + 1 + length).ok_or_else(|| Error::parse("pdb: short string past end"))?;
            Ok(String::from_utf8_lossy(text).into_owned())
        }
    }
}

impl<'a> RekordboxPdb<'a> {
    /// Parse the file header and table list.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let len_page = u32_le(data, 4)?;
        let num_tables = u32_le(data, 8)?;
        let next_unused_page = u32_le(data, 12)?;
        let sequence = u32_le(data, 20)?;

        if len_page == 0 {
            return Err(Error::parse("pdb: page size is zero"));
        }

        let mut tables = Vec::with_capacity(num_tables as usize);
        let mut at = 28;
        for _ in 0..num_tables {
            tables.push(Table {
                page_type: PageType::from_u32(u32_le(data, at)?),
                empty_candidate: u32_le(data, at + 4)?,
                first_page: u32_le(data, at + 8)?,
                last_page: u32_le(data, at + 12)?,
            });
            at += 16;
        }

        Ok(Self { data, len_page, num_tables, next_unused_page, sequence, tables })
    }

    /// Every present row of the given table, page by page.
    pub fn table_rows(&self, table: &Table) -> Result<Vec<Row>> {
        let mut rows = Vec::new();
        let mut page_index = table.first_page;
        let mut visited = 0u32;

        loop {
            let page = self.page(page_index)?;

            // Adjust our page ref for the next iteration early so we can
            // continue without having to remember to update it.
            let next = page.next_page;

            // Ignore non-data pages. Not sure what these are for?
            if page.is_data_page() {
                self.page_rows(&page, table.page_type, &mut rows)?;
            }

            // Follow the chain until it points past the last page (as
            // upstream does), with a guard against a looping chain.
            visited += 1;
            if next > table.last_page || next == page_index || visited > 1_000_000 {
                break;
            }
            let next_offset = u64::from(next) * u64::from(self.len_page);
            if next_offset + u64::from(self.len_page) > self.data.len() as u64 {
                break;
            }
            page_index = next;
        }

        Ok(rows)
    }

    /// Every present row of every table this crate knows how to read.
    pub fn all_rows(&self) -> Result<Vec<(PageType, Vec<Row>)>> {
        let mut out = Vec::new();
        for table in &self.tables {
            if table_has_rows(table.page_type) {
                out.push((table.page_type, self.table_rows(table)?));
            }
        }
        Ok(out)
    }

    fn page(&self, index: u32) -> Result<Page<'a>> {
        let start = index as usize * self.len_page as usize;
        let end = start + self.len_page as usize;
        let data =
            self.data.get(start..end).ok_or_else(|| Error::parse(format!("pdb: page {index} is past the end of the file")))?;

        Ok(Page {
            data,
            page_index: u32_le(data, 4)?,
            page_type: PageType::from_u32(u32_le(data, 8)?),
            next_page: u32_le(data, 12)?,
            num_rows_small: u8_at(data, 24)?,
            page_flags: u8_at(data, 27)?,
            free_size: u16_le(data, 28)?,
            used_size: u16_le(data, 30)?,
            num_rows_large: u16_le(data, 34)?,
        })
    }

    fn page_rows(&self, page: &Page<'a>, page_type: PageType, out: &mut Vec<Row>) -> Result<()> {
        let len_page = self.len_page as usize;
        let num_rows = page.num_rows();
        if num_rows == 0 {
            return Ok(());
        }
        let num_groups = (num_rows - 1) / 16 + 1;

        for group_index in 0..num_groups {
            // The starting point of this group of row indices.
            let base = len_page - group_index * 0x24;
            if base < 4 {
                break;
            }
            let row_present_flags = u16_le(page.data, base - 4)?;
            let rows_in_group = if group_index < num_groups - 1 { 16 } else { (num_rows - 1) % 16 + 1 };

            for row_index in 0..rows_in_group {
                let present = (row_present_flags >> row_index) & 1 != 0;
                if !present {
                    continue;
                }
                let ofs_row = usize::from(u16_le(page.data, base - (6 + 2 * row_index))?);
                // The location of this row relative to the start of the page.
                // A variety of pointers (such as all device_sql_string values)
                // are calculated with respect to this position.
                let row_base = Page::HEAP_POS + ofs_row;
                match parse_row(page.data, row_base, page_type) {
                    Ok(Some(row)) => out.push(row),
                    Ok(None) => {}
                    Err(e) => {
                        tracing::debug!(target: "alphatheta_connect", "pdb: skipping row at {row_base:#x} of page {}: {e}", page.page_index);
                    }
                }
            }
        }

        Ok(())
    }
}

fn table_has_rows(t: PageType) -> bool {
    matches!(
        t,
        PageType::Tracks
            | PageType::Genres
            | PageType::Artists
            | PageType::Albums
            | PageType::Labels
            | PageType::Keys
            | PageType::Colors
            | PageType::PlaylistTree
            | PageType::PlaylistEntries
            | PageType::Artwork
    )
}

struct Page<'a> {
    data: &'a [u8],
    page_index: u32,
    #[allow(dead_code)]
    page_type: PageType,
    next_page: u32,
    num_rows_small: u8,
    page_flags: u8,
    #[allow(dead_code)]
    free_size: u16,
    #[allow(dead_code)]
    used_size: u16,
    num_rows_large: u16,
}

impl Page<'_> {
    /// The heap starts right after the fixed page header.
    const HEAP_POS: usize = 0x28;

    fn is_data_page(&self) -> bool {
        self.page_flags & 0x40 == 0
    }

    /// The number of rows on this page (controls the number of row index
    /// entries there are, but some of those may not be marked as present in
    /// the table due to deletion).
    fn num_rows(&self) -> usize {
        if self.num_rows_large > u16::from(self.num_rows_small) && self.num_rows_large != 0x1fff {
            usize::from(self.num_rows_large)
        } else {
            usize::from(self.num_rows_small)
        }
    }
}

fn parse_row(d: &[u8], base: usize, page_type: PageType) -> Result<Option<Row>> {
    Ok(Some(match page_type {
        PageType::Albums => {
            let ofs_name = usize::from(u8_at(d, base + 21)?);
            Row::Album(IdNameRow { id: u32_le(d, base + 12)?, name: device_sql_string(d, base + ofs_name)? })
        }
        PageType::Artists => {
            let subtype = u16_le(d, base)?;
            let ofs = if subtype == 0x64 { usize::from(u16_le(d, base + 0x0a)?) } else { usize::from(u8_at(d, base + 9)?) };
            Row::Artist(IdNameRow { id: u32_le(d, base + 4)?, name: device_sql_string(d, base + ofs)? })
        }
        PageType::Artwork => Row::Artwork(ArtworkRow { id: u32_le(d, base)?, path: device_sql_string(d, base + 4)? }),
        PageType::Colors => Row::Color(IdNameRow { id: u32::from(u16_le(d, base + 5)?), name: device_sql_string(d, base + 8)? }),
        PageType::Genres => Row::Genre(IdNameRow { id: u32_le(d, base)?, name: device_sql_string(d, base + 4)? }),
        PageType::Keys => Row::Key(IdNameRow { id: u32_le(d, base)?, name: device_sql_string(d, base + 8)? }),
        PageType::Labels => Row::Label(IdNameRow { id: u32_le(d, base)?, name: device_sql_string(d, base + 4)? }),
        PageType::PlaylistTree => Row::PlaylistTree(PlaylistTreeRow {
            parent_id: u32_le(d, base)?,
            sort_order: u32_le(d, base + 8)?,
            id: u32_le(d, base + 12)?,
            raw_is_folder: u32_le(d, base + 16)?,
            name: device_sql_string(d, base + 20)?,
        }),
        PageType::PlaylistEntries => Row::PlaylistEntry(PlaylistEntryRow {
            entry_index: u32_le(d, base)?,
            track_id: u32_le(d, base + 4)?,
            playlist_id: u32_le(d, base + 8)?,
        }),
        PageType::Tracks => {
            let mut ofs_strings = [0usize; 21];
            for (i, slot) in ofs_strings.iter_mut().enumerate() {
                *slot = usize::from(u16_le(d, base + 94 + i * 2)?);
            }
            let s = |i: usize| device_sql_string(d, base + ofs_strings[i]);
            Row::Track(Box::new(TrackRow {
                index_shift: u16_le(d, base + 2)?,
                bitmask: u32_le(d, base + 4)?,
                sample_rate: u32_le(d, base + 8)?,
                composer_id: u32_le(d, base + 12)?,
                file_size: u32_le(d, base + 16)?,
                artwork_id: u32_le(d, base + 28)?,
                key_id: u32_le(d, base + 32)?,
                original_artist_id: u32_le(d, base + 36)?,
                label_id: u32_le(d, base + 40)?,
                remixer_id: u32_le(d, base + 44)?,
                bitrate: u32_le(d, base + 48)?,
                track_number: u32_le(d, base + 52)?,
                tempo: u32_le(d, base + 56)?,
                genre_id: u32_le(d, base + 60)?,
                album_id: u32_le(d, base + 64)?,
                artist_id: u32_le(d, base + 68)?,
                id: u32_le(d, base + 72)?,
                disc_number: u16_le(d, base + 76)?,
                play_count: u16_le(d, base + 78)?,
                year: u16_le(d, base + 80)?,
                sample_depth: u16_le(d, base + 82)?,
                duration: u16_le(d, base + 84)?,
                color_id: u8_at(d, base + 88)?,
                rating: u8_at(d, base + 89)?,
                isrc: s(0)?,
                texter: s(1)?,
                unknown_string_2: s(2)?,
                unknown_string_3: s(3)?,
                unknown_string_4: s(4)?,
                message: s(5)?,
                kuvo_public: s(6)?,
                autoload_hotcues: s(7)?,
                unknown_string_5: s(8)?,
                unknown_string_6: s(9)?,
                date_added: s(10)?,
                release_date: s(11)?,
                mix_name: s(12)?,
                unknown_string_7: s(13)?,
                analyze_path: s(14)?,
                analyze_date: s(15)?,
                comment: s(16)?,
                title: s(17)?,
                unknown_string_8: s(18)?,
                filename: s(19)?,
                file_path: s(20)?,
            }))
        }
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_sql_strings_decode() {
        // short ascii "abc": mangled = ((3 + 1) * 2) + 1 = 9
        let mut d = vec![9u8];
        d.extend_from_slice(b"abc");
        assert_eq!(device_sql_string(&d, 0).unwrap(), "abc");

        // long ascii
        let mut d = vec![0x40, 3, 0];
        d.extend_from_slice(b"xyz");
        assert_eq!(device_sql_string(&d, 0).unwrap(), "xyz");

        // long utf16le: length includes the 4 header bytes
        let mut d = vec![0x90, 8, 0, 0];
        d.extend_from_slice(&[b'h', 0, b'i', 0]);
        assert_eq!(device_sql_string(&d, 0).unwrap(), "hi");

        // even mangled length is invalid -> empty
        assert_eq!(device_sql_string(&[4u8, b'a'], 0).unwrap(), "");
        // mangled 1 -> length -1 -> empty
        assert_eq!(device_sql_string(&[1u8], 0).unwrap(), "");
    }

    /// Build a two-page pdb holding one genre table with one row, to exercise
    /// the page / row-group / row-index walk end to end.
    #[test]
    fn walks_pages_and_row_groups() {
        let len_page = 256usize;
        let mut file = vec![0u8; len_page * 3];
        // header
        file[4..8].copy_from_slice(&(len_page as u32).to_le_bytes());
        file[8..12].copy_from_slice(&1u32.to_le_bytes()); // num_tables
                                                          // table 0: genres, first page 1, last page 2
        file[28..32].copy_from_slice(&1u32.to_le_bytes());
        file[36..40].copy_from_slice(&1u32.to_le_bytes());
        file[40..44].copy_from_slice(&2u32.to_le_bytes());

        // page 1: a non-data page linking to page 2
        let p1 = len_page;
        file[p1 + 4..p1 + 8].copy_from_slice(&1u32.to_le_bytes());
        file[p1 + 8..p1 + 12].copy_from_slice(&1u32.to_le_bytes());
        file[p1 + 12..p1 + 16].copy_from_slice(&2u32.to_le_bytes());
        file[p1 + 27] = 0x40; // not a data page

        // page 2: one genre row at heap offset 0
        let p2 = len_page * 2;
        file[p2 + 4..p2 + 8].copy_from_slice(&2u32.to_le_bytes());
        file[p2 + 8..p2 + 12].copy_from_slice(&1u32.to_le_bytes());
        file[p2 + 12..p2 + 16].copy_from_slice(&3u32.to_le_bytes());
        file[p2 + 24] = 1; // num_rows_small
        file[p2 + 27] = 0x24;
        let row = p2 + 0x28;
        file[row..row + 4].copy_from_slice(&42u32.to_le_bytes());
        file[row + 4] = 11; // short ascii, length 4
        file[row + 5..row + 9].copy_from_slice(b"Tech");
        // row index: group 0 base = end of page
        let base = p2 + len_page;
        file[base - 4..base - 2].copy_from_slice(&1u16.to_le_bytes()); // present flags
        file[base - 6..base - 4].copy_from_slice(&0u16.to_le_bytes()); // ofs_row 0

        let pdb = RekordboxPdb::parse(&file).unwrap();
        assert_eq!(pdb.tables.len(), 1);
        let rows = pdb.table_rows(&pdb.tables[0]).unwrap();
        assert_eq!(rows, vec![Row::Genre(IdNameRow { id: 42, name: "Tech".into() })]);
    }
}
