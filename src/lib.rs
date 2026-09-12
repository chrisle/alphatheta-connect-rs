//! AlphaTheta / Pioneer PRO DJ LINK protocol: consume CDJ state and retrieve
//! complete track metadata.
//!
//! This crate is a Rust port of the TypeScript library
//! [alphatheta-connect](https://github.com/chrisle/alphatheta-connect). The
//! module layout mirrors the upstream `src/` tree so changes can be followed
//! file by file.
//!
//! # Connecting to the network
//!
//! ```no_run
//! use alphatheta_connect::{bring_online, NetworkConfig};
//!
//! #[tokio::main]
//! async fn main() -> alphatheta_connect::Result<()> {
//!     // Open the announce / beat / status sockets.
//!     let network = bring_online(None).await?;
//!
//!     // React to devices appearing on the network.
//!     let _listener = network.device_manager().on_connected(|device| {
//!         println!("New device on network: {} [id {}]", device.name, device.id);
//!     });
//!
//!     // Wait for a peer so the right interface can be chosen, then join as a
//!     // virtual CDJ (device id 7 by default, outside the player range).
//!     network.autoconfig_from_peers().await?;
//!     network.connect().await?;
//!
//!     let status = network.status_emitter().expect("connected");
//!     let mut rx = status.subscribe_status();
//!     while let Ok(state) = rx.recv().await {
//!         println!("player {} play state {:?}", state.device_id, state.play_state);
//!     }
//!     Ok(())
//! }
//! ```

pub mod artwork;
pub mod constants;
pub mod control;
pub mod db;
pub mod devices;
pub mod emitter;
pub mod entities;
mod error;
pub mod localdb;
pub mod logger;
pub mod metadata;
pub mod mixstatus;
pub mod network;
pub mod nfs;
pub mod passive;
pub mod remotedb;
pub mod status;
pub mod types;
pub mod utils;
pub mod virtualcdj;

pub use emitter::{Emitter, Listener};
pub use entities::*;
pub use error::{Error, Result};
pub use logger::{Logger, NoopLogger, SharedLogger, TracingLogger};
pub use mixstatus::{MixstatusConfig, MixstatusProcessor};
pub use network::{bring_online, bring_online_stagehand, ConnectMethod, NetworkConfig, ProlinkNetwork};
pub use status::position::PositionEmitter;
pub use types::*;

// Passive mode (pcap-based monitoring without announcing a VCDJ)
pub use passive::*;

// Artwork extraction
pub use artwork::{
    extract_artwork, extract_artwork_from_device, is_artwork_extraction_supported, ExtractedArtwork,
    FileReader as ArtworkFileReader, PictureType,
};

// Full metadata extraction (title, artist, album, BPM, key, genre, artwork)
pub use metadata::{extract_full_metadata, extract_metadata_from_device, is_metadata_extraction_supported, ExtractedMetadata};

// ANLZ file loading (for analysis data: beat grid, cues, phrases, waveforms)
pub use localdb::rekordbox::{
    load_anlz, AnlzKind, AnlzResolver, AnlzResponse, AnlzResponse2EX, AnlzResponseDAT, AnlzResponseEXT,
};
pub use nfs::fetch_file;

// Database adapters
pub use localdb::database_adapter::{DatabaseAdapter, DatabasePreference, DatabaseType, PlaylistQueryResult};
pub use localdb::onelibrary::{
    Category, DeviceProperty, HistorySession, HotCueBankList, MenuItem, MyTag, OneLibraryAdapter, SortOption,
};

// Virtual CDJ device-ID selection, for consumers that choose their own ID
pub use virtualcdj::device_id::{
    pick_available_device_id, pick_remote_db_query_id, player_number_ceiling, DeviceLike, PickDeviceIdOptions, QueryIdDeviceLike,
    DEFAULT_MIXER_PLAYER_CEILING, MAX_DEVICE_ID, MIXER_PLAYER_CEILING, REMOTEDB_MAX_DEVICE_ID,
};
