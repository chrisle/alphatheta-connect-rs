//! OneLibrary (exportLibrary.db) support, inlined from the
//! `onelibrary-connect` sibling library.

pub mod adapter;
pub mod connection;
pub mod encryption;
pub mod schema;

pub use crate::entities::{Category, DeviceProperty, HistorySession, HotCueBankList, MenuItem, MyTag, SortOption};
pub use adapter::OneLibraryAdapter;
pub use connection::open_one_library_db;
pub use encryption::get_encryption_key;
