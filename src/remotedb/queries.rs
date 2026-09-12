//! Each type of query: what arguments are required, and how to transform the
//! resulting items into something useful.

use crate::entities::{Album, Artist, Artwork, Color, Genre, Key, Label, Playlist, Track};
use crate::remotedb::fields::Field;
use crate::remotedb::message::item::{Item, ItemType};
use crate::remotedb::message::{data_request, menu_request, response_type as response, Message, ResponseData};
use crate::remotedb::utils::{field_from_descriptor, find_color, find_type, render_items};
use crate::remotedb::{Connection, LookupDescriptor};
use crate::types::{BeatGrid, WaveformDetailed, WaveformHD, WaveformPreview};
use crate::{Error, Result};

fn id_name<T, F: FnOnce(u32, String) -> T>(item: Option<&Item>, make: F) -> Option<T> {
    item.map(|i| {
        let (id, name) = i.id_name();
        make(id, name)
    })
}

/// Lookup track metadata from rekordbox and coerce it into a Track entity.
pub async fn get_metadata(conn: &Connection, d: &LookupDescriptor, track_id: u32) -> Result<Track> {
    let request = Message::new(data_request::GET_METADATA, vec![field_from_descriptor(d), Field::UInt32(track_id)]);

    conn.write_message(request).await?;
    let resp = conn.read_message(response::SUCCESS).await?;
    let items_available = items_available(&resp)?;

    // We'll get back these specific items when rendering out the items.
    //
    // NOTE: We actually also get back a color, but we'll find that one later,
    // since each color is it's own item type.
    let items = render_items(conn, d, items_available).await?;
    let get = |t: ItemType| find_type(&items, t);

    // Translate our items into a (partial) Track entity. Streaming tracks may
    // omit some fields.
    let title = get(ItemType::TrackTitle);

    Ok(Track {
        id: title.map(Item::id).unwrap_or(0),
        title: title.map(|t| t.title().to_string()).unwrap_or_default(),
        duration: get(ItemType::Duration).map(|i| f64::from(i.duration())).unwrap_or(0.0),
        tempo: get(ItemType::Tempo).map(Item::bpm).unwrap_or(0.0),
        comment: get(ItemType::Comment).map(|i| i.comment().to_string()).unwrap_or_default(),
        rating: get(ItemType::Rating).map(Item::rating).unwrap_or(0),
        year: get(ItemType::Year).and_then(Item::year),
        bitrate: get(ItemType::BitRate).map(Item::bitrate),

        artwork: Some(Artwork { id: title.map(|t| t.artwork_id).unwrap_or(0), path: None }),
        album: id_name(get(ItemType::AlbumTitle), |id, name| Album { id, name }),
        artist: id_name(get(ItemType::Artist), |id, name| Artist { id, name }),
        genre: id_name(get(ItemType::Genre), |id, name| Genre { id, name }),
        key: id_name(get(ItemType::Key), |id, name| Key { id, name }),
        color: id_name(find_color(&items), |id, name| Color { id, name }),
        label: id_name(get(ItemType::Label), |id, name| Label { id, name }),
        remixer: id_name(get(ItemType::Remixer), |id, name| Artist { id, name }),
        original_artist: id_name(get(ItemType::OriginalArtist), |id, name| Artist { id, name }),
        composer: None,

        file_name: String::new(),
        file_path: String::new(),

        beat_grid: None,
        cue_and_loops: None,
        waveform_hd: None,
        ..Default::default()
    })
}

/// Lookup generic metadata for an unanalyzed track.
pub async fn get_generic_metadata(conn: &Connection, d: &LookupDescriptor, track_id: u32) -> Result<Track> {
    let request = Message::new(data_request::GET_GENERIC_METADATA, vec![field_from_descriptor(d), Field::UInt32(track_id)]);

    conn.write_message(request).await?;
    let resp = conn.read_message(response::SUCCESS).await?;
    let items_available = items_available(&resp)?;

    // NOTE: We actually also get back a color, but we'll find that one later,
    // since each color is it's own item type.
    let items = render_items(conn, d, items_available).await?;
    let get = |t: ItemType| find_type(&items, t);
    let require = |t: ItemType| get(t).ok_or_else(|| Error::RemoteDb(format!("generic metadata is missing a {t:?} item")));

    let title = require(ItemType::TrackTitle)?;

    // Translate our items into a (partial) Track entity.
    Ok(Track {
        id: title.id(),
        title: title.title().to_string(),
        duration: f64::from(require(ItemType::Duration)?.duration()),
        tempo: require(ItemType::Tempo)?.bpm(),
        comment: require(ItemType::Comment)?.comment().to_string(),
        rating: require(ItemType::Rating)?.rating(),
        bitrate: Some(require(ItemType::BitRate)?.bitrate()),

        artwork: Some(Artwork { id: title.artwork_id, path: None }),
        album: id_name(get(ItemType::AlbumTitle), |id, name| Album { id, name }),
        artist: id_name(get(ItemType::Artist), |id, name| Artist { id, name }),
        genre: id_name(get(ItemType::Genre), |id, name| Genre { id, name }),
        color: id_name(find_color(&items), |id, name| Color { id, name }),

        file_name: String::new(),
        file_path: String::new(),

        key: None,
        label: None,
        remixer: None,
        original_artist: None,
        composer: None,

        beat_grid: None,
        cue_and_loops: None,
        waveform_hd: None,
        ..Default::default()
    })
}

/// Lookup the artwork image given the artwork id obtained from a track.
pub async fn get_artwork(conn: &Connection, d: &LookupDescriptor, artwork_id: u32) -> Result<Vec<u8>> {
    let request = Message::new(data_request::GET_ARTWORK, vec![field_from_descriptor(d), Field::UInt32(artwork_id)]);
    conn.write_message(request).await?;
    let art = conn.read_message(response::ARTWORK).await?;
    match art.data()? {
        ResponseData::Artwork(bytes) => Ok(bytes),
        other => Err(unexpected(other)),
    }
}

/// Lookup the beatgrid for the specified track id.
pub async fn get_beatgrid(conn: &Connection, d: &LookupDescriptor, track_id: u32) -> Result<BeatGrid> {
    let request = Message::new(data_request::GET_BEAT_GRID, vec![field_from_descriptor(d), Field::UInt32(track_id)]);
    conn.write_message(request).await?;
    let grid = conn.read_message(response::BEAT_GRID).await?;
    match grid.data()? {
        ResponseData::BeatGrid(g) => Ok(g),
        other => Err(unexpected(other)),
    }
}

/// Lookup the waveform preview for the specified track id.
pub async fn get_waveform_preview(conn: &Connection, d: &LookupDescriptor, track_id: u32) -> Result<WaveformPreview> {
    let request = Message::new(
        data_request::GET_WAVEFORM_PREVIEW,
        vec![field_from_descriptor(d), Field::UInt32(0), Field::UInt32(track_id), Field::UInt32(0), Field::Binary(Vec::new())],
    );
    conn.write_message(request).await?;
    let resp = conn.read_message(response::WAVEFORM_PREVIEW).await?;
    match resp.data()? {
        ResponseData::WaveformPreview(w) => Ok(w),
        other => Err(unexpected(other)),
    }
}

/// Lookup the detailed waveform for the specified track id.
pub async fn get_waveform_detailed(conn: &Connection, d: &LookupDescriptor, track_id: u32) -> Result<WaveformDetailed> {
    let request = Message::new(
        data_request::GET_WAVEFORM_DETAILED,
        vec![field_from_descriptor(d), Field::UInt32(track_id), Field::UInt32(0)],
    );
    conn.write_message(request).await?;
    let resp = conn.read_message(response::WAVEFORM_DETAILED).await?;
    match resp.data()? {
        ResponseData::WaveformDetailed(w) => Ok(w),
        other => Err(unexpected(other)),
    }
}

/// Lookup the HD (nexus2) waveform for the specified track id.
pub async fn get_waveform_hd(conn: &Connection, d: &LookupDescriptor, track_id: u32) -> Result<WaveformHD> {
    let request = Message::new(
        data_request::GET_WAVEFORM_HD,
        vec![
            field_from_descriptor(d),
            Field::UInt32(track_id),
            Field::UInt32(u32::from_le_bytes(*b"PWV5")),
            Field::UInt32(u32::from_le_bytes(*b"EXT\0")),
        ],
    );
    conn.write_message(request).await?;
    let resp = conn.read_message(response::WAVEFORM_HD).await?;
    match resp.data()? {
        ResponseData::WaveformHD(w) => Ok(w),
        other => Err(unexpected(other)),
    }
}

/// Lookup the [hot]cue points and [hot]loops for a track.
pub async fn get_cue_and_loops(
    conn: &Connection,
    d: &LookupDescriptor,
    track_id: u32,
) -> Result<Vec<crate::entities::CueAndLoop>> {
    let request = Message::new(data_request::GET_CUE_AND_LOOPS, vec![field_from_descriptor(d), Field::UInt32(track_id)]);
    conn.write_message(request).await?;
    let resp = conn.read_message(response::CUE_AND_LOOP).await?;
    match resp.data()? {
        ResponseData::CueAndLoop(c) => Ok(c),
        other => Err(unexpected(other)),
    }
}

/// Lookup the "advanced" (nexus2) [hot]cue points and [hot]loops for a track.
pub async fn get_cue_and_loops_adv(
    conn: &Connection,
    d: &LookupDescriptor,
    track_id: u32,
) -> Result<Vec<crate::entities::CueAndLoop>> {
    let request = Message::new(
        data_request::GET_ADV_CUE_AND_LOOPS,
        vec![field_from_descriptor(d), Field::UInt32(track_id), Field::UInt32(0)],
    );
    conn.write_message(request).await?;
    let resp = conn.read_message(response::ADV_CUE_AND_LOOPS).await?;
    match resp.data()? {
        ResponseData::AdvCueAndLoops(c) => Ok(c),
        other => Err(unexpected(other)),
    }
}

/// Lookup the track information, currently just returns the track path.
pub async fn get_track_info(conn: &Connection, d: &LookupDescriptor, track_id: u32) -> Result<String> {
    let request = Message::new(data_request::GET_TRACK_INFO, vec![field_from_descriptor(d), Field::UInt32(track_id)]);
    conn.write_message(request).await?;
    let resp = conn.read_message(response::SUCCESS).await?;
    let items_available = items_available(&resp)?;

    let items = render_items(conn, d, items_available).await?;
    find_type(&items, ItemType::Path)
        .map(|p| p.path().to_string())
        .ok_or_else(|| Error::RemoteDb("track info has no path item".into()))
}

/// The result of a playlist lookup.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlaylistResult {
    pub folders: Vec<Playlist>,
    pub playlists: Vec<Playlist>,
    /// The track title items of the playlist, in order. `Item::id` is the
    /// track id.
    pub track_entries: Vec<Item>,
}

/// Lookup playlist entries.
///
/// - `id`: the ID of the playlist to query for. `None` (or 0) queries the
///   root playlist folder.
/// - `is_folder_request`: when querying for a playlist folder this must be true.
pub async fn get_playlist(
    conn: &Connection,
    d: &LookupDescriptor,
    id: Option<u32>,
    is_folder_request: bool,
) -> Result<PlaylistResult> {
    // XXX: coerce `0` into None to keep a consistent representation of parent_id.
    let parent_id = id.filter(|i| *i != 0);

    // TODO: Maybe sort could become a parameter
    let sort = Field::UInt32(0);
    let id = Field::UInt32(parent_id.unwrap_or(0));
    let is_folder = Field::UInt32(u32::from(is_folder_request));

    let request = Message::new(menu_request::MENU_PLAYLIST, vec![field_from_descriptor(d), sort, id, is_folder]);

    conn.write_message(request).await?;
    let resp = conn.read_message(response::SUCCESS).await?;
    let items_available = items_available(&resp)?;

    let items = render_items(conn, d, items_available).await?;

    let folders = items
        .iter()
        .filter(|i| i.item_type == ItemType::Folder)
        .map(|i| Playlist { is_folder: true, id: i.id(), name: i.name().to_string(), parent_id })
        .collect();

    let playlists = items
        .iter()
        .filter(|i| i.item_type == ItemType::Playlist)
        .map(|i| Playlist { is_folder: false, id: i.id(), name: i.name().to_string(), parent_id })
        .collect();

    let track_entries = items.into_iter().filter(|i| i.item_type == ItemType::TrackTitle).collect();

    Ok(PlaylistResult { folders, playlists, track_entries })
}

fn items_available(resp: &Message) -> Result<u32> {
    match resp.data()? {
        ResponseData::Success { items_available } => Ok(items_available),
        other => Err(unexpected(other)),
    }
}

fn unexpected(other: ResponseData) -> Error {
    Error::RemoteDb(format!("unexpected response data: {other:?}"))
}
