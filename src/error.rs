//! The crate-wide error type.
//!
//! Upstream throws plain `Error` objects with a message. The Rust port keeps
//! one error enum for the whole crate, with a variant per failure class so
//! callers can match on what went wrong rather than on message text.

use std::fmt;

/// Every fallible operation in this crate returns this.
pub type Result<T> = std::result::Result<T, Error>;

/// Failure classes for the crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A socket, file or other OS-level failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// A packet or message did not have the shape the protocol requires.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The library was used before it was ready (for example connecting an
    /// unconfigured network).
    #[error("{0}")]
    State(String),

    /// A request to a device did not complete in time.
    #[error("timeout: {0}")]
    Timeout(String),

    /// An NFS / RPC call was refused or failed on the device.
    #[error("nfs: {0}")]
    Nfs(String),

    /// A remote database (remotedb) request failed.
    #[error("remotedb: {0}")]
    RemoteDb(String),

    /// A rekordbox database (pdb / OneLibrary) could not be read.
    #[error("database: {0}")]
    Database(String),

    /// SQLite failed.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// A binary file (ANLZ, pdb, audio tags) did not parse.
    #[error("parse error: {0}")]
    Parse(String),

    /// A feature was compiled out (for example `passive` without libpcap).
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// Anything else, with a message.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// A protocol error with a formatted message.
    pub fn protocol(msg: impl fmt::Display) -> Self {
        Error::Protocol(msg.to_string())
    }

    /// A parse error with a formatted message.
    pub fn parse(msg: impl fmt::Display) -> Self {
        Error::Parse(msg.to_string())
    }

    /// A generic error with a formatted message.
    pub fn other(msg: impl fmt::Display) -> Self {
        Error::Other(msg.to_string())
    }
}

impl From<tokio::time::error::Elapsed> for Error {
    fn from(e: tokio::time::error::Elapsed) -> Self {
        Error::Timeout(e.to_string())
    }
}
