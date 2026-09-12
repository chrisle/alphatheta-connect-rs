//! Translate pdb rows into entities.

use crate::entities::{Album, Artist, Artwork, Color, Genre, Key, Label, Playlist, PlaylistEntry};
use crate::localdb::orm::TrackWithFks;
use crate::localdb::rekordbox::pdb::{ArtworkRow, IdNameRow, PlaylistEntryRow, PlaylistTreeRow, TrackRow};
use crate::utils::parse_date;

fn nonzero(v: u32) -> Option<u32> {
    if v == 0 {
        None
    } else {
        Some(v)
    }
}

/// Translates a pdb track row entry to a track entity (with foreign keys).
pub fn create_track(row: &TrackRow) -> TrackWithFks {
    let analyze_path = &row.analyze_path;

    let mut track = crate::entities::Track {
        id: row.id,
        title: row.title.clone(),
        track_number: Some(row.track_number),
        disc_number: Some(u32::from(row.disc_number)),
        duration: f64::from(row.duration),
        sample_rate: Some(row.sample_rate),
        sample_depth: Some(u32::from(row.sample_depth)),
        bitrate: Some(row.bitrate),
        tempo: f64::from(row.tempo) / 100.0,
        play_count: Some(u32::from(row.play_count)),
        year: Some(u32::from(row.year)),
        rating: u32::from(row.rating),
        mix_name: Some(row.mix_name.clone()),
        comment: row.comment.clone(),
        autoload_hotcues: Some(row.autoload_hotcues == "ON"),
        kuvo_public: Some(row.kuvo_public == "ON"),
        file_path: row.file_path.clone(),
        file_name: row.filename.clone(),
        file_size: Some(u64::from(row.file_size)),
        release_date: Some(row.release_date.clone()),
        analyze_date: parse_date(&row.analyze_date),
        date_added: parse_date(&row.date_added),
        ..Default::default()
    };

    // The analyze file comes in 3 forms:
    //
    //  1. A `DAT` file, which is missing some extended information, for the
    //     older Pioneer equipment (likely due to memory constraints).
    //  2. A `EXT` file which includes colored waveforms and other extended data.
    //  3. A `2EX` file with 3-band waveforms and vocal detection.
    //
    // We normalize this path by trimming the DAT extension off. Later we will
    // try and read whatever is available.
    track.analyze_path =
        if analyze_path.is_empty() { None } else { Some(analyze_path[..analyze_path.len().saturating_sub(4)].to_string()) };

    // NOTE: There are a few additional columns that will be hydrated through
    // the analyze files (given the analyze_path) which we do not assign here.
    track.beat_grid = None;
    track.cue_and_loops = None;
    track.waveform_hd = None;

    TrackWithFks {
        track,
        artwork_id: nonzero(row.artwork_id),
        artist_id: nonzero(row.artist_id),
        original_artist_id: nonzero(row.original_artist_id),
        remixer_id: nonzero(row.remixer_id),
        composer_id: nonzero(row.composer_id),
        album_id: nonzero(row.album_id),
        label_id: nonzero(row.label_id),
        genre_id: nonzero(row.genre_id),
        color_id: nonzero(u32::from(row.color_id)),
        key_id: nonzero(row.key_id),
    }
}

/// Translates a pdb playlist row entry into a [`Playlist`] entity.
pub fn create_playlist(row: &PlaylistTreeRow) -> Playlist {
    Playlist { id: row.id, name: row.name.clone(), is_folder: row.is_folder(), parent_id: nonzero(row.parent_id) }
}

/// Translates a pdb playlist track entry into a [`PlaylistEntry`] entity.
///
/// The pdb row has no id of its own; upstream inserts `undefined`. The
/// entry index doubles as the id here.
pub fn create_playlist_entry(row: &PlaylistEntryRow) -> PlaylistEntry {
    PlaylistEntry { id: row.entry_index, sort_index: row.entry_index, playlist_id: row.playlist_id, track_id: row.track_id }
}

/// Translates a pdb artwork entry into an [`Artwork`] entity.
pub fn create_artwork_entry(row: &ArtworkRow) -> Artwork {
    Artwork { id: row.id, path: Some(row.path.clone()) }
}

pub fn create_artist(row: &IdNameRow) -> Artist {
    Artist { id: row.id, name: row.name.clone() }
}

pub fn create_album(row: &IdNameRow) -> Album {
    Album { id: row.id, name: row.name.clone() }
}

pub fn create_genre(row: &IdNameRow) -> Genre {
    Genre { id: row.id, name: row.name.clone() }
}

pub fn create_label(row: &IdNameRow) -> Label {
    Label { id: row.id, name: row.name.clone() }
}

pub fn create_color(row: &IdNameRow) -> Color {
    Color { id: row.id, name: row.name.clone() }
}

pub fn create_key(row: &IdNameRow) -> Key {
    Key { id: row.id, name: row.name.clone() }
}
