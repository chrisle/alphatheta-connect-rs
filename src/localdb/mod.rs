//! The local database: rekordbox databases downloaded from media slots on
//! the CDJs and queried locally.

pub mod database_adapter;
pub mod onelibrary;
pub mod orm;
pub mod rekordbox;
pub mod utils;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;

use crate::constants::{MAX_CDJ_DEVICE_ID, MIN_CDJ_DEVICE_ID};
use crate::devices::DeviceManager;
use crate::emitter::{Emitter, Listener};
use crate::entities::Track;
use crate::nfs::{fetch_file, FetchFileOptions, FetchProgress};
use crate::status::StatusEmitter;
use crate::types::{device_snapshot, Device, DeviceId, DeviceType, MediaSlot, MediaSlotInfo, SharedDevice, TrackType};
use crate::{Error, Result};

pub use database_adapter::{DatabaseAdapter, DatabasePreference, DatabaseType, PlaylistQueryResult};
pub use onelibrary::OneLibraryAdapter;
pub use orm::MetadataORM;
pub use rekordbox::{hydrate_database, load_anlz, HydrationProgress};

/// A shared database adapter.
pub type SharedAdapter = Arc<dyn DatabaseAdapter>;

/// Progress of a database download from a CDJ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgressEvent {
    /// The device progress is being reported for.
    pub device: Device,
    /// The media slot progress is being reported for.
    pub slot: MediaSlot,
    /// The current progress of the fetch.
    pub progress: FetchProgress,
}

/// Progress of hydrating a rekordbox database into memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationProgressEvent {
    pub device: Device,
    pub slot: MediaSlot,
    pub progress: HydrationProgress,
}

/// Fired when the database has been fully hydrated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationDoneEvent {
    pub device: Device,
    pub slot: MediaSlot,
}

/// One loaded database file: the adapter plus the temp file backing it, if any.
#[derive(Clone)]
pub struct LoadedAdapter {
    /// The database adapter instance (MetadataORM or OneLibraryAdapter).
    pub adapter: SharedAdapter,
    /// Path to the temp file (for OneLibrary), removed on close.
    pub temp_file: Option<PathBuf>,
}

impl LoadedAdapter {
    fn close(&self) {
        self.adapter.close();
        // Clean up temp file if it exists (OneLibrary databases)
        if let Some(path) = &self.temp_file {
            let _ = std::fs::remove_file(path);
        }
    }
}

struct DatabaseItem {
    /// The unique identifier of the database.
    id: String,
    /// The media device plugged into the device.
    media: MediaSlotInfo,
    /// The CDJ the media is plugged into.
    device: Device,
    /// The slot the media is plugged into.
    slot: MediaSlot,
    /// The active database.
    active: RwLock<LoadedAdapter>,
    /// The other database format on the same media, loaded on demand when
    /// the active database disagrees with what the player reports (see
    /// [`LocalDatabase::find_track`]).
    ///
    /// Outer `None` means it has not been tried yet; `Some(None)` means the
    /// media has no such file (or it failed to load) and it will not be tried
    /// again. The mutex also serialises loading the alternate.
    alternate: tokio::sync::Mutex<Option<Option<LoadedAdapter>>>,
}

impl DatabaseItem {
    fn active(&self) -> LoadedAdapter {
        self.active.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// What the player knows about the track it is asking us to look up, used
/// to check that the row a database hands back is really that track.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrackLookupHint {
    /// The track's BPM as reported in the player's status packet (unpitched,
    /// two decimals). `None` when the player did not report one.
    pub track_bpm: Option<f64>,
}

/// The outcome of [`LocalDatabase::find_track`].
#[derive(Clone)]
pub struct TrackLookup {
    /// The slot's database, or `None` when the slot has none loaded.
    pub adapter: Option<SharedAdapter>,
    /// The track, or `None` when the database holds no such row.
    pub track: Option<Track>,
    /// Set when answering meant switching the slot to its other database
    /// format: the type the slot is now served from.
    pub switched_to: Option<DatabaseType>,
}

/// A row is only trusted when the player's reported BPM agrees with it.
/// Either side missing (unanalysed track, or a player that reports no BPM)
/// is inconclusive and counts as agreement.
fn track_agrees_with_player(track: &Track, hint: &TrackLookupHint) -> bool {
    match hint.track_bpm {
        Some(reported) if track.tempo != 0.0 => (track.tempo - reported).abs() < 0.05,
        _ => true,
    }
}

/// Compute the identifier for a media device in a CDJ. This is used to
/// determine if we have already hydrated the device or not into our local
/// database.
pub fn get_media_id(info: &MediaSlotInfo) -> String {
    let inputs = [
        info.device_id.to_string(),
        info.slot.as_u8().to_string(),
        info.name.clone(),
        info.free_bytes.to_string(),
        info.total_bytes.to_string(),
        info.track_count.to_string(),
        info.created_date.map(|d| d.to_rfc3339()).unwrap_or_else(|| "Invalid Date".into()),
    ];
    let mut hasher = Sha256::new();
    hasher.update(inputs.join(".").as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The well-known paths of the two database formats on rekordbox media.
pub const ONE_LIBRARY_PATH: &str = "PIONEER/rekordbox/exportLibrary.db";
/// See [`ONE_LIBRARY_PATH`].
pub const PDB_PATH: &str = "PIONEER/rekordbox/export.pdb";

/// Fetch a file from a device, trying both dotted and non-dotted paths.
pub(crate) async fn fetch_file_with_fallback(
    device: &Device,
    slot: MediaSlot,
    base_path: &str,
    fetch_progress: &Emitter<DownloadProgressEvent>,
) -> Result<Vec<u8>> {
    let dotted = format!(".{base_path}");
    let attempt_order: [&str; 2] = if cfg!(windows) { [base_path, &dotted] } else { [&dotted, base_path] };

    let mut last_err = None;
    for path in attempt_order {
        let progress_device = device.clone();
        let mut on_progress = move |progress: FetchProgress| {
            fetch_progress.emit(DownloadProgressEvent { device: progress_device.clone(), slot, progress });
        };
        match fetch_file(device, slot, path, FetchFileOptions { on_progress: Some(&mut on_progress), chunk_size: None }).await {
            Ok(data) => return Ok(data),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| Error::Nfs("no path attempted".into())))
}

/// Try to load the OneLibrary database (exportLibrary.db). Returns the
/// adapter and temp file path, or `None` if not available.
pub(crate) async fn try_load_one_library(
    device: &Device,
    slot: MediaSlot,
    fetch_progress: &Emitter<DownloadProgressEvent>,
) -> Option<LoadedAdapter> {
    let db_data = fetch_file_with_fallback(device, slot, ONE_LIBRARY_PATH, fetch_progress).await.ok()?;

    // Write to a temp file (OneLibrary requires a file path for SQLCipher)
    let temp_file =
        std::env::temp_dir().join(format!("prolink-onelibrary-{}-{}-{}.db", device.id, slot.as_u8(), crate::utils::now_ms()));
    tokio::fs::write(&temp_file, &db_data).await.ok()?;

    let path = temp_file.clone();
    let opened = tokio::task::spawn_blocking(move || OneLibraryAdapter::open(&path)).await.ok()?;
    match opened {
        Ok(adapter) => Some(LoadedAdapter { adapter: Arc::new(adapter), temp_file: Some(temp_file) }),
        Err(e) => {
            tracing::debug!(target: "alphatheta_connect", "OneLibrary database on device {} slot {slot:?} did not open: {e}", device.id);
            let _ = std::fs::remove_file(&temp_file);
            None
        }
    }
}

/// Load the PDB database (export.pdb) and hydrate it into a MetadataORM.
pub(crate) async fn load_pdb_database(
    device: &Device,
    slot: MediaSlot,
    fetch_progress: &Emitter<DownloadProgressEvent>,
    hydration_progress: &Emitter<HydrationProgressEvent>,
) -> Result<LoadedAdapter> {
    let pdb_data = fetch_file_with_fallback(device, slot, PDB_PATH, fetch_progress).await?;

    let orm = MetadataORM::new();
    let progress_device = device.clone();
    let mut on_progress = move |progress: HydrationProgress| {
        hydration_progress.emit(HydrationProgressEvent { device: progress_device.clone(), slot, progress });
    };
    hydrate_database(&orm, &pdb_data, Some(&mut on_progress)).await?;

    Ok(LoadedAdapter { adapter: Arc::new(orm), temp_file: None })
}

struct Inner {
    host_device: SharedDevice,
    device_manager: DeviceManager,
    status_emitter: StatusEmitter,
    fetch_progress: Emitter<DownloadProgressEvent>,
    hydration_progress: Emitter<HydrationProgressEvent>,
    hydration_done: Emitter<HydrationDoneEvent>,
    /// Locks for each device slot: `{device.id}-{slot}`. Used when making
    /// track requests.
    slot_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The current available databases.
    dbs: Mutex<Vec<Arc<DatabaseItem>>>,
    /// Database format preference.
    preference: RwLock<DatabasePreference>,
    disconnect_task: Mutex<Option<JoinHandle<()>>>,
}

/// The local database is responsible for syncing the remote rekordbox
/// databases of media slots on a device into memory.
///
/// This service will attempt to ensure the in-memory databases for each
/// media device that is connected to a CDJ is locally kept in sync, fetching
/// the database for any media slot if it's not already cached.
#[derive(Clone)]
pub struct LocalDatabase {
    inner: Arc<Inner>,
}

impl LocalDatabase {
    pub fn new(
        host_device: SharedDevice,
        device_manager: DeviceManager,
        status_emitter: StatusEmitter,
        preference: DatabasePreference,
    ) -> Self {
        let inner = Arc::new(Inner {
            host_device,
            device_manager: device_manager.clone(),
            status_emitter,
            fetch_progress: Emitter::new(),
            hydration_progress: Emitter::new(),
            hydration_done: Emitter::new(),
            slot_locks: Mutex::new(HashMap::new()),
            dbs: Mutex::new(Vec::new()),
            preference: RwLock::new(preference),
            disconnect_task: Mutex::new(None),
        });

        let task = {
            let inner = Arc::clone(&inner);
            let mut rx = device_manager.disconnected().subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(device) => Self::handle_device_removed(&inner, &device),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
        };
        *inner.disconnect_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);

        Self { inner }
    }

    /// Get the current database preference.
    pub fn preference(&self) -> DatabasePreference {
        *self.inner.preference.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Set the database preference. Only affects newly loaded databases.
    pub fn set_preference(&self, value: DatabasePreference) {
        *self.inner.preference.write().unwrap_or_else(|e| e.into_inner()) = value;
    }

    /// Triggered when we are fetching a database from a CDJ.
    pub fn fetch_progress(&self) -> &Emitter<DownloadProgressEvent> {
        &self.inner.fetch_progress
    }

    /// Triggered when we are hydrating a rekordbox database into memory.
    pub fn hydration_progress(&self) -> &Emitter<HydrationProgressEvent> {
        &self.inner.hydration_progress
    }

    /// Triggered when the database has been fully hydrated.
    pub fn hydration_done(&self) -> &Emitter<HydrationDoneEvent> {
        &self.inner.hydration_done
    }

    pub fn on_fetch_progress<F: FnMut(DownloadProgressEvent) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.fetch_progress.on(f)
    }

    pub fn on_hydration_progress<F: FnMut(HydrationProgressEvent) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.hydration_progress.on(f)
    }

    pub fn on_hydration_done<F: FnMut(HydrationDoneEvent) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.hydration_done.on(f)
    }

    /// Disconnects the local database connection for the specified device.
    pub fn disconnect_for_device(&self, device: &Device) {
        Self::handle_device_removed(&self.inner, device);
    }

    /// Closes the database connection and removes the database entry when a
    /// device is removed.
    fn handle_device_removed(inner: &Inner, device: &Device) {
        let removed: Vec<Arc<DatabaseItem>> = {
            let mut dbs = inner.dbs.lock().unwrap_or_else(|e| e.into_inner());
            let (gone, kept): (Vec<_>, Vec<_>) = dbs.drain(..).partition(|db| db.media.device_id == device.id);
            *dbs = kept;
            gone
        };
        for db in removed {
            db.active().close();
            if let Ok(alt) = db.alternate.try_lock() {
                if let Some(Some(loaded)) = alt.as_ref() {
                    loaded.close();
                }
            }
        }
    }

    /// Downloads and loads a database from a device, respecting the database
    /// preference setting.
    async fn hydrate(inner: &Arc<Inner>, device: &Device, slot: MediaSlot, media: MediaSlotInfo) -> Result<Arc<DatabaseItem>> {
        let preference = *inner.preference.read().unwrap_or_else(|e| e.into_inner());

        let loaded = match preference {
            DatabasePreference::Pdb => load_pdb_database(device, slot, &inner.fetch_progress, &inner.hydration_progress).await?,
            DatabasePreference::OneLibrary => {
                try_load_one_library(device, slot, &inner.fetch_progress).await.ok_or_else(|| {
                    Error::Database("OneLibrary database not found and preference is set to oneLibrary only".into())
                })?
            }
            DatabasePreference::Auto => match try_load_one_library(device, slot, &inner.fetch_progress).await {
                Some(loaded) => loaded,
                None => load_pdb_database(device, slot, &inner.fetch_progress, &inner.hydration_progress).await?,
            },
        };

        inner.hydration_done.emit(HydrationDoneEvent { device: device.clone(), slot });

        let db = Arc::new(DatabaseItem {
            id: get_media_id(&media),
            media,
            device: device.clone(),
            slot,
            active: RwLock::new(loaded),
            alternate: tokio::sync::Mutex::new(None),
        });
        inner.dbs.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::clone(&db));

        Ok(db)
    }

    /// Loads the database format the slot is *not* currently served from, so
    /// a lookup can be checked against it. Only meaningful under the 'auto'
    /// preference: a forced format has no alternate. The result (or its
    /// absence) is remembered so the media is never downloaded twice.
    async fn load_alternate(inner: &Arc<Inner>, db: &DatabaseItem) -> Option<LoadedAdapter> {
        let mut alternate = db.alternate.lock().await;
        if let Some(existing) = alternate.as_ref() {
            return existing.clone();
        }

        if *inner.preference.read().unwrap_or_else(|e| e.into_inner()) != DatabasePreference::Auto {
            *alternate = Some(None);
            return None;
        }

        let loaded = match db.active().adapter.database_type() {
            DatabaseType::OneLibrary => {
                load_pdb_database(&db.device, db.slot, &inner.fetch_progress, &inner.hydration_progress).await.ok()
            }
            DatabaseType::Pdb => try_load_one_library(&db.device, db.slot, &inner.fetch_progress).await,
        };

        *alternate = Some(loaded.clone());
        loaded
    }

    /// Looks a track up in the databases of a device slot, checking the
    /// answer against what the player reports.
    ///
    /// A Device Library Plus export carries both `exportLibrary.db` and the
    /// legacy `export.pdb`, and their track IDs are different number spaces.
    /// A player that reads one while we loaded the other resolves most IDs
    /// to a different track and some to nothing at all (NP3-399). So when the
    /// active database has no such row, or its row's BPM is not the BPM the
    /// player is showing, the other format is loaded and asked. If it agrees
    /// with the player, the slot switches to it for every later lookup —
    /// artwork, analysis and the next track all follow the database the
    /// player is using.
    ///
    /// When neither database can be confirmed, the active database's row (if
    /// any) is returned as before.
    pub async fn find_track(
        &self,
        device_id: DeviceId,
        slot: MediaSlot,
        track_id: u32,
        hint: TrackLookupHint,
    ) -> Result<TrackLookup> {
        let Some(db) = self.get_item(device_id, slot).await? else {
            return Ok(TrackLookup { adapter: None, track: None, switched_to: None });
        };

        let active_loaded = db.active();
        let active = active_loaded.adapter.find_track(track_id)?;
        if let Some(track) = &active {
            if track_agrees_with_player(track, &hint) {
                return Ok(TrackLookup { adapter: Some(active_loaded.adapter), track: active, switched_to: None });
            }
        }

        if let Some(alternate) = Self::load_alternate(&self.inner, &db).await {
            if let Some(other) = alternate.adapter.find_track(track_id)? {
                if track_agrees_with_player(&other, &hint) {
                    let switched_to = alternate.adapter.database_type();
                    let previous = {
                        let mut active_slot = db.active.write().unwrap_or_else(|e| e.into_inner());
                        std::mem::replace(&mut *active_slot, alternate.clone())
                    };
                    *db.alternate.lock().await = Some(Some(previous));
                    return Ok(TrackLookup {
                        adapter: Some(alternate.adapter),
                        track: Some(other),
                        switched_to: Some(switched_to),
                    });
                }
            }
        }

        Ok(TrackLookup { adapter: Some(active_loaded.adapter), track: active, switched_to: None })
    }

    /// Gets the database adapter for the media metadata in the provided
    /// device slot.
    ///
    /// If the database has not already been loaded this will first fetch and
    /// load the database, which may take some time depending on the size of
    /// the database.
    ///
    /// Returns `None` if no rekordbox media is present.
    pub async fn get(&self, device_id: DeviceId, slot: MediaSlot) -> Result<Option<SharedAdapter>> {
        Ok(self.get_item(device_id, slot).await?.map(|db| db.active().adapter))
    }

    /// The loaded database entry for a device slot, hydrating it first if needed.
    async fn get_item(&self, device_id: DeviceId, slot: MediaSlot) -> Result<Option<Arc<DatabaseItem>>> {
        if !slot.is_database_slot() {
            return Err(Error::State("Expected USB or SD slot for local database query".into()));
        }

        let lock = {
            let mut locks = self.inner.slot_locks.lock().unwrap_or_else(|e| e.into_inner());
            Arc::clone(
                locks.entry(format!("{device_id}-{}", slot.as_u8())).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };

        let Some(device) = self.inner.device_manager.device(device_id) else {
            return Ok(None);
        };

        if device.device_type != DeviceType::Cdj || device.id < MIN_CDJ_DEVICE_ID || device.id > MAX_CDJ_DEVICE_ID {
            return Ok(None);
        }

        let host = device_snapshot(&self.inner.host_device);
        let media = match self.inner.status_emitter.query_media_slot(&host, &device, slot).await {
            Ok(media) => media,
            // Timeout or other error - treat as no media
            Err(_) => return Ok(None),
        };

        if media.tracks_type != TrackType::Rb {
            return Ok(None);
        }

        let id = get_media_id(&media);

        // Acquire a lock for this device slot that will not release until
        // we've guaranteed the existence of the database.
        let _guard = lock.lock().await;
        let cached = self.inner.dbs.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|db| db.id == id).cloned();
        if let Some(db) = cached {
            return Ok(Some(db));
        }
        Ok(Some(Self::hydrate(&self.inner, &device, slot, media).await?))
    }

    /// Get the database type for an already-loaded device slot. Returns
    /// `None` if no database is loaded for that device/slot.
    pub fn get_database_type(&self, device_id: DeviceId, slot: MediaSlot) -> Option<DatabaseType> {
        self.inner
            .dbs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|db| db.media.device_id == device_id && db.media.slot == slot)
            .map(|db| db.active().adapter.database_type())
    }

    /// Preload the databases for all connected devices.
    pub async fn preload(&self) -> Result<()> {
        let cdjs: Vec<Device> = self
            .inner
            .device_manager
            .devices()
            .into_values()
            .filter(|d| d.device_type == DeviceType::Cdj && d.id >= MIN_CDJ_DEVICE_ID && d.id <= MAX_CDJ_DEVICE_ID)
            .collect();

        for device in cdjs {
            let (usb, sd) = tokio::join!(self.get(device.id, MediaSlot::Usb), self.get(device.id, MediaSlot::Sd));
            usb?;
            sd?;
        }
        Ok(())
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(t) = self.disconnect_task.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            t.abort();
        }
        for db in self.dbs.get_mut().unwrap_or_else(|e| e.into_inner()).drain(..) {
            db.active().close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_id_is_stable_and_distinct() {
        let a = MediaSlotInfo {
            device_id: 2,
            slot: MediaSlot::Usb,
            name: "STICK".into(),
            color: crate::types::MediaColor::Default,
            created_date: None,
            free_bytes: 1,
            total_bytes: 2,
            tracks_type: TrackType::Rb,
            track_count: 3,
            playlist_count: 0,
            has_settings: false,
        };
        let mut b = a.clone();
        b.track_count = 4;
        assert_eq!(get_media_id(&a), get_media_id(&a));
        assert_ne!(get_media_id(&a), get_media_id(&b));
        assert_eq!(get_media_id(&a).len(), 64);
    }

    #[test]
    fn bpm_agreement() {
        let track = Track { tempo: 128.0, ..Default::default() };
        assert!(track_agrees_with_player(&track, &TrackLookupHint { track_bpm: Some(128.0) }));
        assert!(track_agrees_with_player(&track, &TrackLookupHint { track_bpm: Some(128.04) }));
        assert!(!track_agrees_with_player(&track, &TrackLookupHint { track_bpm: Some(130.0) }));
        assert!(track_agrees_with_player(&track, &TrackLookupHint { track_bpm: None }));
        let unanalysed = Track { tempo: 0.0, ..Default::default() };
        assert!(track_agrees_with_player(&unanalysed, &TrackLookupHint { track_bpm: Some(130.0) }));
    }
}
