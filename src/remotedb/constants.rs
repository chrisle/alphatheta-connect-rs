//! Remote database protocol constants.

/// All remote database messages include this 4 byte magic value.
pub const REMOTEDB_MAGIC: u32 = 0x872349ae;

/// The consistent port on which we can query the remote db server for the port.
pub const REMOTEDB_SERVER_QUERY_PORT: u16 = 12523;
