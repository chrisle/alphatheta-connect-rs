//! Opening a OneLibrary database with SQLCipher decryption.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::localdb::onelibrary::encryption::get_encryption_key;
use crate::Result;

/// Open a OneLibrary database read-only with SQLCipher decryption.
pub fn open_one_library_db(db_path: &Path) -> Result<Connection> {
    let key = get_encryption_key()?;

    let db = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    db.pragma_update(None, "cipher", "sqlcipher")?;
    db.pragma_update(None, "legacy", 4)?;
    db.pragma_update(None, "key", key)?;

    Ok(db)
}
