//! OneLibrary database adapter.
//!
//! Provides an interface for reading the OneLibrary (exportLibrary.db) SQLite
//! database used by modern rekordbox versions and Pioneer DJ devices.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::entities::{
    Album, Artist, Artwork, Category, Color, CueAndLoop, CueColor, DeviceProperty, Genre, HistorySession, HotCueBankList,
    HotcueButton, Key, Label, MenuItem, MyTag, Playlist, PlaylistEntry, SortOption, Track,
};
use crate::localdb::database_adapter::{DatabaseAdapter, DatabaseType, PlaylistQueryResult};
use crate::localdb::onelibrary::connection::open_one_library_db;
use crate::localdb::onelibrary::schema::{my_tag_attribute, playlist_attribute};
use crate::utils::parse_date;
use crate::Result;

const TRACK_SELECT: &str = "
    SELECT c.*,
           a.name as artistName,
           al.name as albumName,
           g.name as genreName,
           k.name as keyName,
           col.name as colorName,
           lbl.name as labelName,
           img.path as artworkPath,
           remix.name as remixerName,
           orig.name as originalArtistName,
           comp.name as composerName
    FROM content c
    LEFT JOIN artist a ON c.artist_id_artist = a.artist_id
    LEFT JOIN album al ON c.album_id = al.album_id
    LEFT JOIN genre g ON c.genre_id = g.genre_id
    LEFT JOIN key k ON c.key_id = k.key_id
    LEFT JOIN color col ON c.color_id = col.color_id
    LEFT JOIN label lbl ON c.label_id = lbl.label_id
    LEFT JOIN image img ON c.image_id = img.image_id
    LEFT JOIN artist remix ON c.artist_id_remixer = remix.artist_id
    LEFT JOIN artist orig ON c.artist_id_originalArtist = orig.artist_id
    LEFT JOIN artist comp ON c.artist_id_composer = comp.artist_id
";

/// Adapter for the OneLibrary database. Queries the SQLite file directly
/// instead of hydrating into memory.
pub struct OneLibraryAdapter {
    db: Mutex<Connection>,
}

impl std::fmt::Debug for OneLibraryAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneLibraryAdapter").finish()
    }
}

fn col_i64(row: &Row<'_>, name: &str) -> Option<i64> {
    row.get::<_, Option<i64>>(name).ok().flatten()
}

fn col_u32(row: &Row<'_>, name: &str) -> Option<u32> {
    col_i64(row, name).and_then(|v| u32::try_from(v).ok())
}

fn col_str(row: &Row<'_>, name: &str) -> Option<String> {
    row.get::<_, Option<String>>(name).ok().flatten()
}

fn nonzero(v: Option<u32>) -> Option<u32> {
    v.filter(|v| *v != 0)
}

impl OneLibraryAdapter {
    /// Open the database at `db_path`.
    pub fn open(db_path: &Path) -> Result<Self> {
        Ok(Self { db: Mutex::new(open_one_library_db(db_path)?) })
    }

    fn with_db<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        f(&db)
    }

    // ==========================================================================
    // Track Queries
    // ==========================================================================

    /// Find a track by ID.
    pub fn find_track_by_id(&self, id: u32) -> Result<Option<Track>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached(&format!("{TRACK_SELECT} WHERE c.content_id = ?"))?;
            Ok(stmt.query_row(params![id], |row| Ok(Self::content_to_track(row))).optional()?)
        })
    }

    /// Find all tracks in the database.
    pub fn find_all_tracks(&self) -> Result<Vec<Track>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached(TRACK_SELECT)?;
            let rows = stmt.query_map([], |row| Ok(Self::content_to_track(row)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Convert a content row to a Track entity.
    fn content_to_track(row: &Row<'_>) -> Track {
        let analysis_path = col_str(row, "analysisDataFilePath");

        Track {
            id: col_u32(row, "content_id").unwrap_or(0),
            title: col_str(row, "title").unwrap_or_default(),
            // ms to seconds
            duration: col_i64(row, "length").filter(|l| *l != 0).map(|l| l as f64 / 1000.0).unwrap_or(0.0),
            bitrate: col_u32(row, "bitrate"),
            tempo: col_i64(row, "bpmx100").filter(|b| *b != 0).map(|b| b as f64 / 100.0).unwrap_or(0.0),
            rating: col_u32(row, "rating").unwrap_or(0),
            comment: col_str(row, "djComment").unwrap_or_default(),
            file_path: col_str(row, "path").unwrap_or_default(),
            file_name: col_str(row, "fileName").unwrap_or_default(),
            track_number: col_u32(row, "trackNo"),
            disc_number: col_u32(row, "discNo"),
            sample_rate: col_u32(row, "samplingRate"),
            sample_depth: col_u32(row, "bitDepth"),
            play_count: col_u32(row, "djPlayCount"),
            year: col_u32(row, "releaseYear"),
            mix_name: col_str(row, "subtitle"),
            autoload_hotcues: Some(col_i64(row, "isHotCueAutoLoadOn").unwrap_or(0) != 0),
            kuvo_public: Some(col_i64(row, "isKuvoDeliverStatusOn").unwrap_or(0) != 0),
            file_size: col_i64(row, "fileSize").and_then(|v| u64::try_from(v).ok()),
            // Normalize analyze_path by trimming the .DAT extension (same as
            // the pdb hydrator). load_anlz() appends the appropriate extension
            // (.DAT / .EXT / .2EX) when loading.
            analyze_path: analysis_path.filter(|p| !p.is_empty()).map(|p| p[..p.len().saturating_sub(4)].to_string()),
            release_date: col_str(row, "releaseDate"),
            analyze_date: None,
            date_added: col_str(row, "dateAdded").and_then(|d| parse_date(&d)),

            cue_and_loops: None,

            artwork: nonzero(col_u32(row, "image_id")).map(|id| Artwork { id, path: col_str(row, "artworkPath") }),
            artist: nonzero(col_u32(row, "artist_id_artist"))
                .map(|id| Artist { id, name: col_str(row, "artistName").unwrap_or_default() }),
            original_artist: nonzero(col_u32(row, "artist_id_originalArtist"))
                .map(|id| Artist { id, name: col_str(row, "originalArtistName").unwrap_or_default() }),
            remixer: nonzero(col_u32(row, "artist_id_remixer"))
                .map(|id| Artist { id, name: col_str(row, "remixerName").unwrap_or_default() }),
            composer: nonzero(col_u32(row, "artist_id_composer"))
                .map(|id| Artist { id, name: col_str(row, "composerName").unwrap_or_default() }),
            album: nonzero(col_u32(row, "album_id")).map(|id| Album { id, name: col_str(row, "albumName").unwrap_or_default() }),
            label: nonzero(col_u32(row, "label_id")).map(|id| Label { id, name: col_str(row, "labelName").unwrap_or_default() }),
            genre: nonzero(col_u32(row, "genre_id")).map(|id| Genre { id, name: col_str(row, "genreName").unwrap_or_default() }),
            color: nonzero(col_u32(row, "color_id")).map(|id| Color { id, name: col_str(row, "colorName").unwrap_or_default() }),
            key: nonzero(col_u32(row, "key_id")).map(|id| Key { id, name: col_str(row, "keyName").unwrap_or_default() }),

            beat_grid: None,
            waveform_hd: None,
        }
    }

    // ==========================================================================
    // Cue Queries
    // ==========================================================================

    /// Find cue points for a track.
    pub fn find_cues(&self, track_id: u32) -> Result<Vec<CueAndLoop>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached("SELECT * FROM cue WHERE content_id = ?")?;
            let rows = stmt.query_map(params![track_id], |row| Ok(Self::cue_to_cue_and_loop(row)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Convert a cue row to a CueAndLoop entity.
    fn cue_to_cue_and_loop(row: &Row<'_>) -> CueAndLoop {
        let in_usec = col_i64(row, "inUsec").unwrap_or(0);
        // microseconds to ms
        let offset = if in_usec != 0 { in_usec as f64 / 1000.0 } else { 0.0 };
        let out_offset = col_i64(row, "outUsec");
        let is_loop = out_offset.is_some_and(|o| o > 0);
        let length = if is_loop { (out_offset.unwrap_or(0) - in_usec) as f64 / 1000.0 } else { 0.0 };

        // Determine if this is a hot cue (kind value mapping may vary).
        // Based on observation: kind 0 = memory cue, kind >= 1 may be hot cue.
        let kind = col_i64(row, "kind").unwrap_or(0);
        let button = if (1..=8).contains(&kind) { HotcueButton::from_u8(kind as u8) } else { None };

        let label = col_str(row, "cueComment");
        let color = col_i64(row, "colorTableIndex").map(|c| CueColor::from_u8(c as u8));

        match (is_loop, button) {
            (true, Some(button)) => CueAndLoop::HotLoop { offset, length, button, label, color },
            (true, None) => CueAndLoop::Loop { offset, length, label, color },
            (false, Some(button)) => CueAndLoop::HotCue { offset, button, label, color },
            (false, None) => CueAndLoop::CuePoint { offset, label, color },
        }
    }

    // ==========================================================================
    // Playlist Queries
    // ==========================================================================

    /// Find a playlist by ID.
    pub fn find_playlist_by_id(&self, playlist_id: u32) -> Result<Option<Playlist>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached("SELECT * FROM playlist WHERE playlist_id = ?")?;
            Ok(stmt.query_row(params![playlist_id], |row| Ok(Self::playlist_row_to_playlist(row))).optional()?)
        })
    }

    /// Query for a list of {folders, playlists, tracks} given a playlist ID.
    /// If no ID is provided the root list is queried.
    pub fn find_playlist_contents(&self, playlist_id: Option<u32>) -> Result<PlaylistQueryResult> {
        self.with_db(|db| {
            let playlists: Vec<Playlist> = match playlist_id {
                None => {
                    let mut stmt = db.prepare_cached("SELECT * FROM playlist WHERE playlist_id_parent IS NULL")?;
                    let rows = stmt.query_map([], |row| Ok(Self::playlist_row_to_playlist(row)))?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()?
                }
                Some(id) => {
                    let mut stmt = db.prepare_cached("SELECT * FROM playlist WHERE playlist_id_parent = ?")?;
                    let rows = stmt.query_map(params![id], |row| Ok(Self::playlist_row_to_playlist(row)))?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()?
                }
            };

            let (folders, playlists): (Vec<Playlist>, Vec<Playlist>) = playlists.into_iter().partition(|p| p.is_folder);

            // Get track entries for this playlist
            let track_entries = match playlist_id {
                None => Vec::new(),
                Some(id) => {
                    let mut stmt =
                        db.prepare_cached("SELECT * FROM playlist_content WHERE playlist_id = ? ORDER BY sequenceNo")?;
                    let rows = stmt.query_map(params![id], |row| {
                        Ok((
                            col_u32(row, "playlist_id").unwrap_or(0),
                            col_u32(row, "content_id").unwrap_or(0),
                            col_u32(row, "sequenceNo").unwrap_or(0),
                        ))
                    })?;
                    rows.enumerate()
                        .map(|(index, r)| {
                            let (playlist_id, content_id, sequence_no) = r?;
                            // playlist_content doesn't have a unique ID, use index
                            Ok(PlaylistEntry { id: index as u32, sort_index: sequence_no, playlist_id, track_id: content_id })
                        })
                        .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?
                }
            };

            Ok(PlaylistQueryResult { folders, playlists, track_entries })
        })
    }

    /// Get track IDs for a playlist in order.
    pub fn find_playlist_track_ids(&self, playlist_id: u32) -> Result<Vec<u32>> {
        self.ids("SELECT content_id FROM playlist_content WHERE playlist_id = ? ORDER BY sequenceNo", playlist_id)
    }

    fn playlist_row_to_playlist(row: &Row<'_>) -> Playlist {
        Playlist {
            id: col_u32(row, "playlist_id").unwrap_or(0),
            name: col_str(row, "name").unwrap_or_default(),
            is_folder: col_i64(row, "attribute") == Some(playlist_attribute::FOLDER),
            parent_id: col_u32(row, "playlist_id_parent"),
        }
    }

    fn ids(&self, sql: &str, id: u32) -> Result<Vec<u32>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached(sql)?;
            let rows = stmt.query_map(params![id], |row| row.get::<_, i64>(0))?;
            Ok(rows.filter_map(|r| r.ok()).filter_map(|v| u32::try_from(v).ok()).collect())
        })
    }

    fn id_name<T>(&self, sql: &str, id: u32, make: impl Fn(u32, String) -> T) -> Result<Option<T>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached(sql)?;
            Ok(stmt
                .query_row(params![id], |row| Ok((col_u32(row, "id").unwrap_or(0), col_str(row, "name").unwrap_or_default())))
                .optional()?
                .map(|(id, name)| make(id, name)))
        })
    }

    // ==========================================================================
    // Reference Table Queries
    // ==========================================================================

    pub fn find_artist(&self, artist_id: u32) -> Result<Option<Artist>> {
        self.id_name("SELECT artist_id as id, name FROM artist WHERE artist_id = ?", artist_id, |id, name| Artist { id, name })
    }

    pub fn find_album(&self, album_id: u32) -> Result<Option<Album>> {
        self.id_name("SELECT album_id as id, name FROM album WHERE album_id = ?", album_id, |id, name| Album { id, name })
    }

    pub fn find_genre(&self, genre_id: u32) -> Result<Option<Genre>> {
        self.id_name("SELECT genre_id as id, name FROM genre WHERE genre_id = ?", genre_id, |id, name| Genre { id, name })
    }

    pub fn find_key(&self, key_id: u32) -> Result<Option<Key>> {
        self.id_name("SELECT key_id as id, name FROM key WHERE key_id = ?", key_id, |id, name| Key { id, name })
    }

    pub fn find_color(&self, color_id: u32) -> Result<Option<Color>> {
        self.id_name("SELECT color_id as id, name FROM color WHERE color_id = ?", color_id, |id, name| Color { id, name })
    }

    pub fn find_label(&self, label_id: u32) -> Result<Option<Label>> {
        self.id_name("SELECT label_id as id, name FROM label WHERE label_id = ?", label_id, |id, name| Label { id, name })
    }

    pub fn find_artwork(&self, image_id: u32) -> Result<Option<Artwork>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached("SELECT * FROM image WHERE image_id = ?")?;
            Ok(stmt
                .query_row(params![image_id], |row| {
                    Ok(Artwork { id: col_u32(row, "image_id").unwrap_or(0), path: col_str(row, "path") })
                })
                .optional()?)
        })
    }

    // ==========================================================================
    // MyTag (User Tags) Queries
    // ==========================================================================

    /// Find MyTags under a parent (root-level when `None`): folders and tags.
    pub fn find_my_tags(&self, parent_id: Option<u32>) -> Result<(Vec<MyTag>, Vec<MyTag>)> {
        self.with_db(|db| {
            let rows: Vec<MyTag> = match parent_id {
                None => {
                    let mut stmt = db.prepare_cached(
                        "SELECT * FROM myTag WHERE myTag_id_parent IS NULL OR myTag_id_parent = 0 ORDER BY sequenceNo",
                    )?;
                    let rows = stmt.query_map([], |row| Ok(Self::my_tag_row_to_my_tag(row)))?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()?
                }
                Some(id) => {
                    let mut stmt = db.prepare_cached("SELECT * FROM myTag WHERE myTag_id_parent = ? ORDER BY sequenceNo")?;
                    let rows = stmt.query_map(params![id], |row| Ok(Self::my_tag_row_to_my_tag(row)))?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()?
                }
            };
            Ok(rows.into_iter().partition(|t| t.is_folder))
        })
    }

    /// Find a MyTag by ID.
    pub fn find_my_tag_by_id(&self, my_tag_id: u32) -> Result<Option<MyTag>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached("SELECT * FROM myTag WHERE myTag_id = ?")?;
            Ok(stmt.query_row(params![my_tag_id], |row| Ok(Self::my_tag_row_to_my_tag(row))).optional()?)
        })
    }

    /// Get track IDs for a MyTag.
    pub fn find_my_tag_contents(&self, my_tag_id: u32) -> Result<Vec<u32>> {
        self.ids("SELECT content_id FROM myTag_content WHERE myTag_id = ?", my_tag_id)
    }

    /// Get all MyTags assigned to a track.
    pub fn find_my_tags_for_track(&self, track_id: u32) -> Result<Vec<MyTag>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached(
                "SELECT t.* FROM myTag t INNER JOIN myTag_content tc ON t.myTag_id = tc.myTag_id WHERE tc.content_id = ?",
            )?;
            let rows = stmt.query_map(params![track_id], |row| Ok(Self::my_tag_row_to_my_tag(row)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    fn my_tag_row_to_my_tag(row: &Row<'_>) -> MyTag {
        MyTag {
            id: col_u32(row, "myTag_id").unwrap_or(0),
            name: col_str(row, "name").unwrap_or_default(),
            is_folder: col_i64(row, "attribute") == Some(my_tag_attribute::FOLDER),
            parent_id: col_u32(row, "myTag_id_parent"),
        }
    }

    // ==========================================================================
    // History Queries
    // ==========================================================================

    /// Find all history sessions.
    pub fn find_history_sessions(&self) -> Result<Vec<HistorySession>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached("SELECT * FROM history ORDER BY sequenceNo")?;
            let rows = stmt.query_map([], |row| {
                Ok(HistorySession {
                    id: col_u32(row, "history_id").unwrap_or(0),
                    name: col_str(row, "name").unwrap_or_default(),
                    parent_id: col_u32(row, "history_id_parent"),
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Get track IDs for a history session in order.
    pub fn find_history_contents(&self, history_id: u32) -> Result<Vec<u32>> {
        self.ids("SELECT content_id FROM history_content WHERE history_id = ? ORDER BY sequenceNo", history_id)
    }

    // ==========================================================================
    // Hot Cue Bank Queries
    // ==========================================================================

    /// Find all hot cue bank lists.
    pub fn find_hot_cue_bank_lists(&self) -> Result<Vec<HotCueBankList>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached("SELECT * FROM hotCueBankList ORDER BY sequenceNo")?;
            let rows = stmt.query_map([], |row| {
                Ok(HotCueBankList {
                    id: col_u32(row, "hotCueBankList_id").unwrap_or(0),
                    name: col_str(row, "name").unwrap_or_default(),
                    parent_id: col_u32(row, "hotCueBankList_id_parent"),
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Get cue IDs for a hot cue bank list.
    pub fn find_hot_cue_bank_list_cues(&self, bank_list_id: u32) -> Result<Vec<u32>> {
        self.ids("SELECT cue_id FROM hotCueBankList_cue WHERE hotCueBankList_id = ? ORDER BY sequenceNo", bank_list_id)
    }

    // ==========================================================================
    // Menu Configuration Queries
    // ==========================================================================

    /// Get all menu items (browse categories).
    pub fn find_menu_items(&self) -> Result<Vec<MenuItem>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached("SELECT * FROM menuItem ORDER BY menuItem_id")?;
            let rows = stmt.query_map([], |row| {
                Ok(MenuItem {
                    id: col_u32(row, "menuItem_id").unwrap_or(0),
                    kind: col_u32(row, "kind").unwrap_or(0),
                    name: col_str(row, "name").unwrap_or_default(),
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Get visible categories with their menu item info.
    pub fn find_visible_categories(&self) -> Result<Vec<Category>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached(
                "SELECT c.*, m.name as menuName, m.kind as menuKind
                 FROM category c
                 LEFT JOIN menuItem m ON c.menuItem_id = m.menuItem_id
                 WHERE c.isVisible = 1
                 ORDER BY c.sequenceNo",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(Category {
                    id: col_u32(row, "category_id").unwrap_or(0),
                    menu_item_id: col_u32(row, "menuItem_id").unwrap_or(0),
                    name: col_str(row, "menuName").unwrap_or_default(),
                    kind: col_u32(row, "menuKind").unwrap_or(0),
                    is_visible: col_i64(row, "isVisible").unwrap_or(0) != 0,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Get visible sort options.
    pub fn find_visible_sort_options(&self) -> Result<Vec<SortOption>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached(
                "SELECT s.*, m.name as menuName, m.kind as menuKind
                 FROM sort s
                 LEFT JOIN menuItem m ON s.menuItem_id = m.menuItem_id
                 WHERE s.isVisible = 1
                 ORDER BY s.sequenceNo",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(SortOption {
                    id: col_u32(row, "sort_id").unwrap_or(0),
                    menu_item_id: col_u32(row, "menuItem_id").unwrap_or(0),
                    name: col_str(row, "menuName").unwrap_or_default(),
                    kind: col_u32(row, "menuKind").unwrap_or(0),
                    is_visible: col_i64(row, "isVisible").unwrap_or(0) != 0,
                    is_selected_as_sub_column: col_i64(row, "isSelectedAsSubColumn").unwrap_or(0) != 0,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    // ==========================================================================
    // Device Property Queries
    // ==========================================================================

    /// Get device properties.
    pub fn get_property(&self) -> Result<Option<DeviceProperty>> {
        self.with_db(|db| {
            let mut stmt = db.prepare_cached("SELECT * FROM property LIMIT 1")?;
            Ok(stmt
                .query_row([], |row| {
                    Ok(DeviceProperty {
                        device_name: col_str(row, "deviceName").unwrap_or_default(),
                        db_version: col_str(row, "dbVersion").unwrap_or_default(),
                        number_of_contents: col_u32(row, "numberOfContents").unwrap_or(0),
                        created_date: col_str(row, "createdDate").unwrap_or_default(),
                        background_color_type: col_u32(row, "backGroundColorType").unwrap_or(0),
                    })
                })
                .optional()?)
        })
    }
}

impl DatabaseAdapter for OneLibraryAdapter {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::OneLibrary
    }

    fn find_track(&self, id: u32) -> Result<Option<Track>> {
        self.find_track_by_id(id)
    }

    fn find_playlist(&self, playlist_id: Option<u32>) -> Result<PlaylistQueryResult> {
        self.find_playlist_contents(playlist_id)
    }

    fn close(&self) {
        // The connection closes when the adapter is dropped; nothing to do
        // eagerly that would not leave a poisoned handle behind.
    }
}
