//! OneLibrary adapter tests against the encrypted fixture database that
//! matches the schema of rekordbox's exportLibrary.db (ported from upstream's
//! onelibrary-adapter.spec.ts).

use std::path::PathBuf;

use alphatheta_connect::entities::{CueAndLoop, HotcueButton};
use alphatheta_connect::localdb::{DatabaseAdapter, DatabaseType, OneLibraryAdapter};

fn fixture() -> OneLibraryAdapter {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/test-onelibrary.db");
    OneLibraryAdapter::open(&path).expect("fixture opens")
}

#[test]
fn finds_a_track_with_full_metadata() {
    let adapter = fixture();
    assert_eq!(adapter.database_type(), DatabaseType::OneLibrary);
    let track = adapter.find_track(1).unwrap().expect("track 1");
    assert_eq!(track.id, 1);
    assert_eq!(track.title, "Test Track");
    assert_eq!(track.mix_name.as_deref(), Some("Extended Mix"));
    assert_eq!(track.tempo, 128.0);
    assert_eq!(track.duration, 300.0);
    assert_eq!(track.rating, 5);
    assert_eq!(track.track_number, Some(1));
    assert_eq!(track.disc_number, Some(1));
    assert_eq!(track.comment, "Test comment");
    assert_eq!(track.file_path, "/Music/test.mp3");
    assert_eq!(track.file_name, "test.mp3");
    assert_eq!(track.file_size, Some(5_000_000));
    assert_eq!(track.bitrate, Some(320));
    assert_eq!(track.sample_rate, Some(44100));
    assert_eq!(track.sample_depth, Some(16));
    assert_eq!(track.play_count, Some(10));
    assert_eq!(track.autoload_hotcues, Some(true));
    assert_eq!(track.kuvo_public, Some(true));

    assert_eq!(track.artist.as_ref().map(|a| (a.id, a.name.as_str())), Some((1, "Test Artist")));
    assert_eq!(track.remixer.as_ref().map(|a| (a.id, a.name.as_str())), Some((3, "Remixer One")));
    assert_eq!(track.album.as_ref().map(|a| (a.id, a.name.as_str())), Some((1, "Test Album")));
    assert_eq!(track.genre.as_ref().map(|a| (a.id, a.name.as_str())), Some((1, "Electronic")));
    assert_eq!(track.key.as_ref().map(|a| (a.id, a.name.as_str())), Some((1, "Am")));
    assert_eq!(track.color.as_ref().map(|a| (a.id, a.name.as_str())), Some((1, "Pink")));
    assert_eq!(track.label.as_ref().map(|a| (a.id, a.name.as_str())), Some((1, "Test Label")));
    let artwork = track.artwork.unwrap();
    assert_eq!(artwork.id, 1);
    assert_eq!(artwork.path.as_deref(), Some("/PIONEER/USBANLZ/P001/0001/artwork.jpg"));
}

#[test]
fn missing_and_minimal_tracks() {
    let adapter = fixture();
    assert!(adapter.find_track(999).unwrap().is_none());

    let track = adapter.find_track(3).unwrap().expect("track 3");
    assert_eq!(track.title, "Unknown Track");
    assert_eq!(track.tempo, 0.0);
    assert!(track.artist.is_none());
    assert!(track.album.is_none());
    assert!(track.genre.is_none());
    assert!(track.key.is_none());
    assert!(track.color.is_none());
    assert!(track.label.is_none());
    assert!(track.artwork.is_none());

    let tracks = adapter.find_all_tracks().unwrap();
    let mut ids: Vec<u32> = tracks.iter().map(|t| t.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    assert_eq!(adapter.find_track(2).unwrap().unwrap().tempo, 140.0);
    assert_eq!(adapter.find_track(2).unwrap().unwrap().duration, 240.0);
}

#[test]
fn cues() {
    let adapter = fixture();
    let cues = adapter.find_cues(1).unwrap();
    assert_eq!(cues.len(), 4);
    assert!(adapter.find_cues(3).unwrap().is_empty());
    assert!(adapter.find_cues(999).unwrap().is_empty());

    let offsets: Vec<f64> = cues.iter().map(CueAndLoop::offset).collect();
    for expected in [0.0, 32000.0, 64000.0, 128000.0] {
        assert!(offsets.contains(&expected), "{offsets:?}");
    }

    let memory = cues.iter().find(|c| matches!(c, CueAndLoop::CuePoint { .. })).unwrap();
    assert_eq!(memory.offset(), 0.0);

    let hot = cues.iter().find(|c| matches!(c, CueAndLoop::HotCue { .. })).unwrap();
    assert_eq!(hot.offset(), 32000.0);
    assert_eq!(hot.label(), Some("Drop"));

    let lp = cues.iter().find(|c| matches!(c, CueAndLoop::Loop { .. })).unwrap();
    assert_eq!(lp.length(), Some(16000.0));

    let hot_loop = cues.iter().find(|c| matches!(c, CueAndLoop::HotLoop { .. })).unwrap();
    assert_eq!(hot_loop.button(), Some(HotcueButton::B));
    assert_eq!(hot_loop.length(), Some(8000.0));
}

#[test]
fn playlists() {
    let adapter = fixture();
    let root = adapter.find_playlist(None).unwrap();
    assert_eq!(root.playlists.len(), 2);
    assert_eq!(root.folders.len(), 1);
    assert!(root.track_entries.is_empty());

    let favorites = root.playlists.iter().find(|p| p.name == "My Favorites").unwrap();
    assert_eq!(favorites.id, 1);
    assert!(!favorites.is_folder);
    let dj_sets = &root.folders[0];
    assert_eq!(dj_sets.id, 2);
    assert!(dj_sets.is_folder);

    let nested = adapter.find_playlist(Some(2)).unwrap();
    assert_eq!(nested.playlists.len(), 2);
    let names: Vec<&str> = nested.playlists.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"Club Night"));
    assert!(names.contains(&"Festival"));
    assert!(nested.folders.is_empty());

    assert_eq!(adapter.find_playlist_track_ids(1).unwrap(), vec![1, 2, 4]);
    assert!(adapter.find_playlist_track_ids(999).unwrap().is_empty());
    let entries = adapter.find_playlist(Some(1)).unwrap().track_entries;
    assert_eq!(entries.iter().map(|e| e.track_id).collect::<Vec<_>>(), vec![1, 2, 4]);
}

#[test]
fn my_tags_history_and_menus() {
    let adapter = fixture();
    let (folders, tags) = adapter.find_my_tags(None).unwrap();
    assert_eq!(tags.len(), 2);
    assert_eq!(folders.len(), 1);
    assert!(folders[0].is_folder);
    let (nested_folders, nested_tags) = adapter.find_my_tags(Some(folders[0].id)).unwrap();
    assert_eq!(nested_tags.len(), 2);
    assert!(nested_folders.is_empty());
    let favorites = adapter.find_my_tag_by_id(tags[0].id).unwrap().unwrap();
    assert!(!favorites.is_folder);

    // The remaining queries just need to run against the schema.
    let _ = adapter.find_history_sessions().unwrap();
    let _ = adapter.find_hot_cue_bank_lists().unwrap();
    let _ = adapter.find_menu_items().unwrap();
    let _ = adapter.find_visible_categories().unwrap();
    let _ = adapter.find_visible_sort_options().unwrap();
    let _ = adapter.get_property().unwrap();
    assert_eq!(adapter.find_artist(1).unwrap().unwrap().name, "Test Artist");
    assert!(adapter.find_artist(999).unwrap().is_none());
}
