//! The in-memory store the PDB hydrator fills.
//!
//! Upstream hydrates rows into an in-memory SQLite database and queries it
//! back with a small ORM. The rows are only ever looked up by id, so this
//! port keeps them in maps; the interface (`insert_*`, `find_track`,
//! `find_playlist`) is the same.

use std::collections::HashMap;
use std::sync::RwLock;

use crate::entities::{Album, Artist, Artwork, Color, Genre, Key, Label, Playlist, PlaylistEntry, Track};
use crate::localdb::database_adapter::{DatabaseAdapter, DatabaseType, PlaylistQueryResult};
use crate::Result;

/// Table names available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Table {
    Artist,
    Album,
    Genre,
    Color,
    Label,
    Key,
    Artwork,
    Playlist,
    PlaylistEntry,
    Track,
}

impl Table {
    pub const fn name(self) -> &'static str {
        match self {
            Table::Artist => "artist",
            Table::Album => "album",
            Table::Genre => "genre",
            Table::Color => "color",
            Table::Label => "label",
            Table::Key => "key",
            Table::Artwork => "artwork",
            Table::Playlist => "playlist",
            Table::PlaylistEntry => "playlist_entry",
            Table::Track => "track",
        }
    }
}

/// A track row as hydrated from the pdb, with foreign keys rather than
/// resolved relations (upstream's `Track<EntityFK.WithFKs>`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackWithFks {
    pub track: Track,
    pub artwork_id: Option<u32>,
    pub artist_id: Option<u32>,
    pub original_artist_id: Option<u32>,
    pub remixer_id: Option<u32>,
    pub composer_id: Option<u32>,
    pub album_id: Option<u32>,
    pub label_id: Option<u32>,
    pub genre_id: Option<u32>,
    pub color_id: Option<u32>,
    pub key_id: Option<u32>,
}

#[derive(Default)]
struct Tables {
    artists: HashMap<u32, Artist>,
    albums: HashMap<u32, Album>,
    genres: HashMap<u32, Genre>,
    colors: HashMap<u32, Color>,
    labels: HashMap<u32, Label>,
    keys: HashMap<u32, Key>,
    artwork: HashMap<u32, Artwork>,
    playlists: HashMap<u32, Playlist>,
    playlist_entries: Vec<PlaylistEntry>,
    tracks: HashMap<u32, TrackWithFks>,
}

/// Object relation mapper over the hydrated metadata. May be used to
/// populate a metadata database and query objects.
#[derive(Default)]
pub struct MetadataORM {
    tables: RwLock<Tables>,
}

impl std::fmt::Debug for MetadataORM {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let t = self.tables.read().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("MetadataORM").field("tracks", &t.tracks.len()).field("playlists", &t.playlists.len()).finish()
    }
}

impl MetadataORM {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_artist(&self, e: Artist) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).artists.insert(e.id, e);
    }

    pub fn insert_album(&self, e: Album) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).albums.insert(e.id, e);
    }

    pub fn insert_genre(&self, e: Genre) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).genres.insert(e.id, e);
    }

    pub fn insert_color(&self, e: Color) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).colors.insert(e.id, e);
    }

    pub fn insert_label(&self, e: Label) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).labels.insert(e.id, e);
    }

    pub fn insert_key(&self, e: Key) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).keys.insert(e.id, e);
    }

    pub fn insert_artwork(&self, e: Artwork) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).artwork.insert(e.id, e);
    }

    pub fn insert_playlist(&self, e: Playlist) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).playlists.insert(e.id, e);
    }

    pub fn insert_playlist_entry(&self, e: PlaylistEntry) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).playlist_entries.push(e);
    }

    pub fn insert_track(&self, e: TrackWithFks) {
        self.tables.write().unwrap_or_else(|e| e.into_inner()).tracks.insert(e.track.id, e);
    }

    /// Number of tracks hydrated.
    pub fn track_count(&self) -> usize {
        self.tables.read().unwrap_or_else(|e| e.into_inner()).tracks.len()
    }

    /// Locate a track by ID, resolving its relations.
    pub fn find_track_by_id(&self, id: u32) -> Option<Track> {
        let t = self.tables.read().unwrap_or_else(|e| e.into_inner());
        let row = t.tracks.get(&id)?;
        let mut track = row.track.clone();

        track.beat_grid = None;
        track.cue_and_loops = None;
        track.waveform_hd = None;

        track.artwork = row.artwork_id.and_then(|k| t.artwork.get(&k).cloned());
        track.artist = row.artist_id.and_then(|k| t.artists.get(&k).cloned());
        track.original_artist = row.original_artist_id.and_then(|k| t.artists.get(&k).cloned());
        track.remixer = row.remixer_id.and_then(|k| t.artists.get(&k).cloned());
        track.composer = row.composer_id.and_then(|k| t.artists.get(&k).cloned());
        track.album = row.album_id.and_then(|k| t.albums.get(&k).cloned());
        track.label = row.label_id.and_then(|k| t.labels.get(&k).cloned());
        track.genre = row.genre_id.and_then(|k| t.genres.get(&k).cloned());
        track.color = row.color_id.and_then(|k| t.colors.get(&k).cloned());
        track.key = row.key_id.and_then(|k| t.keys.get(&k).cloned());

        Some(track)
    }

    /// Query for a list of {folders, playlists, tracks} given a playlist ID.
    /// If no ID is provided the root list is queried.
    ///
    /// Note that when tracks are returned there will be no folders or
    /// playlists. But the API here is simpler to assume there could be.
    ///
    /// Tracks are returned in the order they are placed on the playlist.
    pub fn find_playlist_by_id(&self, playlist_id: Option<u32>) -> PlaylistQueryResult {
        let t = self.tables.read().unwrap_or_else(|e| e.into_inner());

        let mut children: Vec<&Playlist> = t.playlists.values().filter(|p| p.parent_id == playlist_id).collect();
        children.sort_by_key(|p| p.id);

        let (folders, playlists): (Vec<&Playlist>, Vec<&Playlist>) = children.into_iter().partition(|p| p.is_folder);

        let mut track_entries: Vec<PlaylistEntry> = match playlist_id {
            Some(id) => t.playlist_entries.iter().filter(|e| e.playlist_id == id).cloned().collect(),
            None => Vec::new(),
        };
        track_entries.sort_by_key(|e| e.sort_index);

        PlaylistQueryResult {
            folders: folders.into_iter().cloned().collect(),
            playlists: playlists.into_iter().cloned().collect(),
            track_entries,
        }
    }
}

impl DatabaseAdapter for MetadataORM {
    fn database_type(&self) -> DatabaseType {
        DatabaseType::Pdb
    }

    fn find_track(&self, id: u32) -> Result<Option<Track>> {
        Ok(self.find_track_by_id(id))
    }

    fn find_playlist(&self, playlist_id: Option<u32>) -> Result<PlaylistQueryResult> {
        Ok(self.find_playlist_by_id(playlist_id))
    }

    fn close(&self) {
        *self.tables.write().unwrap_or_else(|e| e.into_inner()) = Tables::default();
    }
}
