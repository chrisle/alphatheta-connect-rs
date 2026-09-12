//! Rekordbox databases on devices, reached over NFS from passive capture.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use tokio::task::JoinHandle;

use crate::constants::{MAX_CDJ_DEVICE_ID, MIN_CDJ_DEVICE_ID};
use crate::emitter::{Emitter, Listener};
use crate::localdb::{
    get_media_id, load_pdb_database, try_load_one_library, DatabasePreference, DownloadProgressEvent, HydrationDoneEvent,
    HydrationProgressEvent, LoadedAdapter, SharedAdapter,
};
use crate::passive::devices::PassiveDeviceManager;
use crate::passive::status::PassiveStatusEmitter;
use crate::types::{Device, DeviceId, DeviceType, MediaColor, MediaSlot, MediaSlotInfo, TrackType};
use crate::{Error, Result};

struct DatabaseItem {
    id: String,
    media: MediaSlotInfo,
    loaded: LoadedAdapter,
}

struct Inner {
    device_manager: PassiveDeviceManager,
    fetch_progress: Emitter<DownloadProgressEvent>,
    hydration_progress: Emitter<HydrationProgressEvent>,
    hydration_done: Emitter<HydrationDoneEvent>,
    slot_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    dbs: Mutex<Vec<Arc<DatabaseItem>>>,
    /// Cache of media slot info received from broadcast packets.
    media_cache: Mutex<HashMap<String, MediaSlotInfo>>,
    preference: RwLock<DatabasePreference>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

/// Provides access to rekordbox databases on devices using passive packet
/// capture.
///
/// Unlike the active [`LocalDatabase`](crate::localdb::LocalDatabase), this
/// version cannot query for media slot info; it listens for media slot
/// broadcasts to cache media info and offers
/// [`get_with_media`](Self::get_with_media) when media info is known. NFS
/// access to fetch rekordbox databases works without announcing a VCDJ.
#[derive(Clone)]
pub struct PassiveLocalDatabase {
    inner: Arc<Inner>,
}

impl PassiveLocalDatabase {
    pub fn new(
        device_manager: PassiveDeviceManager,
        status_emitter: PassiveStatusEmitter,
        preference: DatabasePreference,
    ) -> Self {
        let inner = Arc::new(Inner {
            device_manager: device_manager.clone(),
            fetch_progress: Emitter::new(),
            hydration_progress: Emitter::new(),
            hydration_done: Emitter::new(),
            slot_locks: Mutex::new(HashMap::new()),
            dbs: Mutex::new(Vec::new()),
            media_cache: Mutex::new(HashMap::new()),
            preference: RwLock::new(preference),
            tasks: Mutex::new(Vec::new()),
        });

        let removed = {
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
        let media = {
            let inner = Arc::clone(&inner);
            let mut rx = status_emitter.media_slot().subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(info) => {
                            inner
                                .media_cache
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(Self::key(info.device_id, info.slot), info);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
        };
        *inner.tasks.lock().unwrap_or_else(|e| e.into_inner()) = vec![removed, media];

        Self { inner }
    }

    fn key(device_id: DeviceId, slot: MediaSlot) -> String {
        format!("{device_id}-{}", slot.as_u8())
    }

    pub fn preference(&self) -> DatabasePreference {
        *self.inner.preference.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Set the database preference. Only affects newly loaded databases.
    pub fn set_preference(&self, value: DatabasePreference) {
        *self.inner.preference.write().unwrap_or_else(|e| e.into_inner()) = value;
    }

    pub fn fetch_progress(&self) -> &Emitter<DownloadProgressEvent> {
        &self.inner.fetch_progress
    }

    pub fn hydration_progress(&self) -> &Emitter<HydrationProgressEvent> {
        &self.inner.hydration_progress
    }

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

    /// Get cached media slot info for a device and slot. `None` if no media
    /// slot info has been received.
    pub fn get_cached_media(&self, device_id: DeviceId, slot: MediaSlot) -> Option<MediaSlotInfo> {
        self.inner.media_cache.lock().unwrap_or_else(|e| e.into_inner()).get(&Self::key(device_id, slot)).cloned()
    }

    /// Get all cached media slot info.
    pub fn get_all_cached_media(&self) -> Vec<MediaSlotInfo> {
        self.inner.media_cache.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }

    /// Disconnects the local database connection for the specified device.
    pub fn disconnect_for_device(&self, device: &Device) {
        Self::handle_device_removed(&self.inner, device);
    }

    /// Stop listening to events and clean up.
    pub fn stop(&self) {
        for t in self.inner.tasks.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            t.abort();
        }
        for db in self.inner.dbs.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            close_loaded(&db.loaded);
        }
        self.inner.media_cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn handle_device_removed(inner: &Inner, device: &Device) {
        let removed: Vec<Arc<DatabaseItem>> = {
            let mut dbs = inner.dbs.lock().unwrap_or_else(|e| e.into_inner());
            let (gone, kept): (Vec<_>, Vec<_>) = dbs.drain(..).partition(|db| db.media.device_id == device.id);
            *dbs = kept;
            gone
        };
        for db in removed {
            close_loaded(&db.loaded);
        }

        // Clear cached media for this device
        let prefix = format!("{}-", device.id);
        inner.media_cache.lock().unwrap_or_else(|e| e.into_inner()).retain(|k, _| !k.starts_with(&prefix));
    }

    async fn hydrate(inner: &Arc<Inner>, device: &Device, slot: MediaSlot, media: MediaSlotInfo) -> Result<Arc<DatabaseItem>> {
        let preference = *inner.preference.read().unwrap_or_else(|e| e.into_inner());

        let loaded = match preference {
            DatabasePreference::Pdb => load_pdb_database(device, slot, &inner.fetch_progress, &inner.hydration_progress).await?,
            DatabasePreference::OneLibrary => {
                try_load_one_library(device, slot, &inner.fetch_progress).await.ok_or_else(|| {
                    Error::Database("OneLibrary database not found and preference is set to oneLibrary only".into())
                })?
            }
            // Auto: Try OneLibrary first, fall back to PDB
            DatabasePreference::Auto => match try_load_one_library(device, slot, &inner.fetch_progress).await {
                Some(loaded) => loaded,
                None => load_pdb_database(device, slot, &inner.fetch_progress, &inner.hydration_progress).await?,
            },
        };

        inner.hydration_done.emit(HydrationDoneEvent { device: device.clone(), slot });

        let db = Arc::new(DatabaseItem { id: get_media_id(&media), media, loaded });
        inner.dbs.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::clone(&db));
        Ok(db)
    }

    fn lock_for(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.inner.slot_locks.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(locks.entry(key).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))))
    }

    fn is_cdj(device: &Device) -> bool {
        device.device_type == DeviceType::Cdj && device.id >= MIN_CDJ_DEVICE_ID && device.id <= MAX_CDJ_DEVICE_ID
    }

    /// Gets the database adapter for the media metadata in the provided
    /// device slot, using cached media info.
    ///
    /// This method uses cached media slot info that was received from
    /// broadcast packets. If no media info is cached for this slot, it will
    /// attempt to fetch the database without media info (useful for
    /// all-in-one units like XDJ-XZ that don't broadcast media slot packets).
    ///
    /// Returns `None` if no rekordbox media is present or the fetch fails.
    pub async fn get(&self, device_id: DeviceId, slot: MediaSlot) -> Result<Option<SharedAdapter>> {
        let Some(device) = self.inner.device_manager.device(device_id) else {
            return Ok(None);
        };

        match self.get_cached_media(device_id, slot) {
            Some(media) => self.get_with_media(&device, slot, media).await,
            // No cached media - try fetching without media info. This is
            // needed for all-in-one units (XDJ-XZ, XDJ-RX) that don't
            // broadcast media slot info packets.
            None => Ok(self.get_without_media(&device, slot).await),
        }
    }

    /// Gets the database adapter for the media metadata using provided media
    /// slot info. Use this when you have media slot info from another source
    /// (e.g., parsed from status packets or provided manually).
    ///
    /// Returns `None` if no rekordbox media is present.
    pub async fn get_with_media(&self, device: &Device, slot: MediaSlot, media: MediaSlotInfo) -> Result<Option<SharedAdapter>> {
        let lock = self.lock_for(Self::key(device.id, slot));

        if !Self::is_cdj(device) {
            return Ok(None);
        }

        if media.tracks_type != TrackType::Rb {
            return Ok(None);
        }

        let id = get_media_id(&media);

        let _guard = lock.lock().await;
        let cached = self.inner.dbs.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|db| db.id == id).cloned();
        let db = match cached {
            Some(db) => db,
            None => Self::hydrate(&self.inner, device, slot, media).await?,
        };

        Ok(Some(Arc::clone(&db.loaded.adapter)))
    }

    /// Attempts to get/hydrate a database without cached media slot info.
    /// This is used for all-in-one units (XDJ-XZ, XDJ-RX, etc.) that don't
    /// broadcast media slot info packets.
    ///
    /// The method will try to fetch the rekordbox database directly via NFS.
    /// If successful, the database is hydrated and cached.
    ///
    /// Returns `None` if no rekordbox database is found or the fetch fails.
    pub async fn get_without_media(&self, device: &Device, slot: MediaSlot) -> Option<SharedAdapter> {
        let lock = self.lock_for(format!("{}-{}-nomedia", device.id, slot.as_u8()));

        if !Self::is_cdj(device) {
            return None;
        }

        let find_existing = || {
            self.inner
                .dbs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|db| db.media.device_id == device.id && db.media.slot == slot)
                .cloned()
        };

        // Check if we already have a cached database for this device/slot
        if let Some(existing) = find_existing() {
            return Some(Arc::clone(&existing.loaded.adapter));
        }

        let _guard = lock.lock().await;
        // Double-check cache inside lock
        if let Some(existing) = find_existing() {
            return Some(Arc::clone(&existing.loaded.adapter));
        }

        // Create synthetic media info - we assume rekordbox since we're
        // attempting to fetch a rekordbox database
        let synthetic = MediaSlotInfo {
            device_id: device.id,
            slot,
            name: "Unknown Media".into(),
            color: MediaColor::Default,
            created_date: Some(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH),
            free_bytes: 0,
            total_bytes: 0,
            tracks_type: TrackType::Rb,
            track_count: 0,
            playlist_count: 0,
            has_settings: false,
        };

        match Self::hydrate(&self.inner, device, slot, synthetic).await {
            Ok(db) => Some(Arc::clone(&db.loaded.adapter)),
            Err(e) => {
                tracing::debug!(target: "alphatheta_connect", "passive database load failed for device {} slot {slot:?}: {e}", device.id);
                None
            }
        }
    }

    /// Preload the databases for all connected devices using cached media info.
    pub async fn preload(&self) -> Result<()> {
        let cdjs: Vec<Device> = self.inner.device_manager.devices().into_values().filter(Self::is_cdj).collect();
        for device in cdjs {
            let (usb, sd) = tokio::join!(self.get(device.id, MediaSlot::Usb), self.get(device.id, MediaSlot::Sd));
            usb?;
            sd?;
        }
        Ok(())
    }
}

fn close_loaded(loaded: &LoadedAdapter) {
    loaded.adapter.close();
    if let Some(path) = &loaded.temp_file {
        let _ = std::fs::remove_file(path);
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        for t in self.tasks.get_mut().unwrap_or_else(|e| e.into_inner()).drain(..) {
            t.abort();
        }
    }
}
