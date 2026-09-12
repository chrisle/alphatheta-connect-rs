//! The common interface for database adapters.
//!
//! Both [`MetadataORM`](crate::localdb::orm::MetadataORM) (for PDB files) and
//! [`OneLibraryAdapter`](crate::localdb::onelibrary::OneLibraryAdapter) (for
//! exportLibrary.db) implement this interface, allowing `LocalDatabase` to
//! use either transparently.

use serde::{Deserialize, Serialize};

use crate::entities::{Playlist, PlaylistEntry, Track};
use crate::Result;

/// Database format preference for loading rekordbox databases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DatabasePreference {
    /// Try OneLibrary first (rekordbox 7.x+), fall back to PDB (rekordbox 6.x).
    #[default]
    Auto,
    /// Only use OneLibrary format (exportLibrary.db).
    OneLibrary,
    /// Only use PDB format (export.pdb).
    Pdb,
}

/// Database type identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DatabaseType {
    OneLibrary,
    Pdb,
}

/// Result of a playlist query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistQueryResult {
    pub folders: Vec<Playlist>,
    pub playlists: Vec<Playlist>,
    pub track_entries: Vec<PlaylistEntry>,
}

/// Common interface for database adapters.
pub trait DatabaseAdapter: Send + Sync {
    /// The type of database (OneLibrary or PDB).
    fn database_type(&self) -> DatabaseType;

    /// Find a track by ID.
    fn find_track(&self, id: u32) -> Result<Option<Track>>;

    /// Query for a list of {folders, playlists, tracks} given a playlist ID.
    /// If no ID is provided the root list is queried.
    fn find_playlist(&self, playlist_id: Option<u32>) -> Result<PlaylistQueryResult>;

    /// Close the database connection.
    fn close(&self);
}
