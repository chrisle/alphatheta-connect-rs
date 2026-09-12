//! XDR encoding for the ONC-RPC, portmap, mount and NFSv2 messages this crate
//! speaks.
//!
//! Upstream builds these with `js-xdr` type declarations; here each message
//! is a small struct with explicit `encode` / `decode` so the byte layout is
//! visible.

use crate::{Error, Result};

/// Calculate padding needed to align to 4-byte boundary (XDR requirement).
fn padding(length: usize) -> usize {
    (4 - length % 4) % 4
}

/// Writes XDR primitives.
#[derive(Debug, Default)]
pub struct XdrWriter {
    buf: Vec<u8>,
}

impl XdrWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// Fixed-length opaque data, padded to 4 bytes.
    pub fn opaque_fixed(&mut self, data: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(data);
        self.buf.extend(std::iter::repeat_n(0u8, padding(data.len())));
        self
    }

    /// Variable-length opaque data: length, bytes, padding.
    pub fn opaque_var(&mut self, data: &[u8]) -> &mut Self {
        self.u32(data.len() as u32);
        self.opaque_fixed(data)
    }

    /// An ASCII/UTF-8 XDR string.
    pub fn string(&mut self, s: &str) -> &mut Self {
        self.opaque_var(s.as_bytes())
    }

    /// In the standard NFS protocol, strings are typically ASCII. For Pioneer
    /// players, it is an UTF-16LE encoded string.
    pub fn string_utf16le(&mut self, s: &str) -> &mut Self {
        let mut data = Vec::with_capacity(s.len() * 2);
        for unit in s.encode_utf16() {
            data.extend_from_slice(&unit.to_le_bytes());
        }
        self.opaque_var(&data)
    }

    /// Raw bytes with no length or padding (the rest of a message).
    pub fn raw(&mut self, data: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(data);
        self
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// Reads XDR primitives.
#[derive(Debug)]
pub struct XdrReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> XdrReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(Error::Nfs("attempt to read outside the boundary of the buffer".into()));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u32()? != 0)
    }

    pub fn opaque_fixed(&mut self, n: usize) -> Result<&'a [u8]> {
        let out = self.take(n)?;
        self.take(padding(n))?;
        Ok(out)
    }

    pub fn opaque_var(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.opaque_fixed(n)
    }

    pub fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.opaque_var()?).into_owned())
    }

    pub fn string_utf16le(&mut self) -> Result<String> {
        let data = self.opaque_var()?;
        let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Ok(String::from_utf16_lossy(&units))
    }

    /// Everything left in the buffer.
    pub fn rest(&mut self) -> &'a [u8] {
        let out = &self.data[self.pos..];
        self.pos = self.data.len();
        out
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

// --------------------------------------------------------------------------
// ONC RPC
// --------------------------------------------------------------------------

/// RPC XDR data types. This implements nearly the entire XDR spec for the
/// ONC-RPC protocol.
pub mod rpc {
    use super::*;

    pub const VERSION: u32 = 2;

    pub const MSG_REQUEST: u32 = 0;
    pub const MSG_RESPONSE: u32 = 1;

    pub const REPLY_ACCEPTED: u32 = 0;
    pub const REPLY_DENIED: u32 = 1;

    /// Accept status values.
    pub mod accept_status {
        pub const SUCCESS: u32 = 0;
        pub const PROGRAM_UNAVAILABLE: u32 = 1;
        pub const PROGRAM_MISMATCH: u32 = 2;
        pub const PROCESS_UNAVAILABLE: u32 = 3;
        pub const GARBAGE_ARGUMENTS: u32 = 4;
        pub const SYSTEM_ERROR: u32 = 5;
    }

    /// AUTH_UNIX credentials.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UnixAuth {
        pub stamp: u32,
        pub name: String,
        pub uid: u32,
        pub gid: u32,
        pub gids: Vec<u32>,
    }

    impl UnixAuth {
        pub fn to_xdr(&self) -> Vec<u8> {
            let mut w = XdrWriter::new();
            w.u32(self.stamp).string(&self.name).u32(self.uid).u32(self.gid);
            w.u32(self.gids.len() as u32);
            for g in &self.gids {
                w.u32(*g);
            }
            w.into_bytes()
        }
    }

    /// An auth field: flavor and opaque body.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Auth {
        pub flavor: u32,
        pub body: Vec<u8>,
    }

    impl Auth {
        pub fn encode(&self, w: &mut XdrWriter) {
            w.u32(self.flavor).opaque_var(&self.body);
        }

        pub fn decode(r: &mut XdrReader<'_>) -> Result<Self> {
            Ok(Self { flavor: r.u32()?, body: r.opaque_var()?.to_vec() })
        }
    }

    /// A call.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Request {
        pub rpc_version: u32,
        pub program: u32,
        pub program_version: u32,
        pub procedure: u32,
        pub auth: Auth,
        pub verifier: Auth,
        pub data: Vec<u8>,
    }

    /// A whole packet: xid plus a request.
    pub fn encode_request(xid: u32, request: &Request) -> Vec<u8> {
        let mut w = XdrWriter::new();
        w.u32(xid).u32(MSG_REQUEST);
        w.u32(request.rpc_version).u32(request.program).u32(request.program_version).u32(request.procedure);
        request.auth.encode(&mut w);
        request.verifier.encode(&mut w);
        w.raw(&request.data);
        w.into_bytes()
    }

    /// The decoded reply of an RPC.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Reply {
        pub xid: u32,
        pub body: ReplyBody,
    }

    /// What the server answered.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ReplyBody {
        /// The call was accepted and succeeded; here are the result bytes.
        Success(Vec<u8>),
        /// The call was accepted but failed with the given accept status
        /// (with low/high versions for a program mismatch).
        Failed { status: u32, mismatch: Option<(u32, u32)> },
        /// The call was denied.
        Denied,
    }

    /// Decode a reply packet.
    pub fn decode_reply(data: &[u8]) -> Result<Reply> {
        let mut r = XdrReader::new(data);
        let xid = r.u32()?;
        let msg_type = r.u32()?;
        if msg_type != MSG_RESPONSE {
            return Err(Error::Nfs(format!("expected an RPC response, got message type {msg_type}")));
        }
        let reply_stat = r.u32()?;
        if reply_stat != REPLY_ACCEPTED {
            return Ok(Reply { xid, body: ReplyBody::Denied });
        }
        let _verifier = Auth::decode(&mut r)?;
        let status = r.u32()?;
        let body = match status {
            accept_status::SUCCESS => ReplyBody::Success(r.rest().to_vec()),
            accept_status::PROGRAM_MISMATCH => ReplyBody::Failed { status, mismatch: Some((r.u32()?, r.u32()?)) },
            other => ReplyBody::Failed { status: other, mismatch: None },
        };
        Ok(Reply { xid, body })
    }
}

// --------------------------------------------------------------------------
// Portmap
// --------------------------------------------------------------------------

/// Portmap RPC XDR types.
pub mod portmap {
    use super::*;

    pub const PROGRAM: u32 = 100_000;
    pub const VERSION: u32 = 2;

    pub mod procedure {
        pub const GET_PORT: u32 = 3;
    }

    /// GETPORT arguments.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct GetPort {
        pub program: u32,
        pub version: u32,
        pub protocol: u32,
        pub port: u32,
    }

    impl GetPort {
        pub fn to_xdr(&self) -> Vec<u8> {
            let mut w = XdrWriter::new();
            w.u32(self.program).u32(self.version).u32(self.protocol).u32(self.port);
            w.into_bytes()
        }
    }
}

// --------------------------------------------------------------------------
// Mount
// --------------------------------------------------------------------------

/// Mount RPC XDR types.
pub mod mount {
    use super::*;

    pub const PROGRAM: u32 = 100_005;
    pub const VERSION: u32 = 1;

    pub mod procedure {
        pub const MOUNT: u32 = 1;
        pub const EXPORT: u32 = 5;
    }

    /// The size of a file handle.
    pub const FILEHANDLE_SIZE: usize = 32;

    /// MNT arguments.
    pub fn encode_mount_request(filesystem: &str) -> Vec<u8> {
        let mut w = XdrWriter::new();
        w.string_utf16le(filesystem);
        w.into_bytes()
    }

    /// One entry of the EXPORT reply.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ExportEntry {
        /// The name of the exported filesystem.
        pub filesystem: String,
        /// The groups allowed to mount this filesystem.
        pub groups: Vec<String>,
    }

    /// Decode the EXPORT reply: a linked list of entries, each with a linked
    /// list of groups.
    pub fn decode_export_list(data: &[u8]) -> Result<Vec<ExportEntry>> {
        let mut r = XdrReader::new(data);
        let mut entries = Vec::new();
        while r.bool()? {
            let filesystem = r.string_utf16le()?;
            let mut groups = Vec::new();
            while r.bool()? {
                groups.push(r.string()?);
            }
            entries.push(ExportEntry { filesystem, groups });
        }
        Ok(entries)
    }

    /// Decode the MNT reply (FHStatus): the root file handle on success.
    pub fn decode_fh_status(data: &[u8]) -> Result<Option<[u8; FILEHANDLE_SIZE]>> {
        let mut r = XdrReader::new(data);
        if r.u32()? != 0 {
            return Ok(None);
        }
        let mut fh = [0u8; FILEHANDLE_SIZE];
        fh.copy_from_slice(r.opaque_fixed(FILEHANDLE_SIZE)?);
        Ok(Some(fh))
    }
}

// --------------------------------------------------------------------------
// NFS v2
// --------------------------------------------------------------------------

/// NFS RPC XDR types.
pub mod nfs {
    use super::*;

    pub const PROGRAM: u32 = 100_003;
    pub const VERSION: u32 = 2;

    pub mod procedure {
        pub const LOOKUP: u32 = 4;
        pub const READ: u32 = 6;
    }

    pub const FILEHANDLE_SIZE: usize = 32;
    /// The largest read a v2 server will return.
    pub const MAX_DATA: usize = 8192;

    /// The type of a file.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum FileType {
        Null,
        Regular,
        Directory,
        Block,
        Char,
        Link,
        Other(u32),
    }

    impl FileType {
        pub const fn from_u32(v: u32) -> Self {
            match v {
                0 => FileType::Null,
                1 => FileType::Regular,
                2 => FileType::Directory,
                3 => FileType::Block,
                4 => FileType::Char,
                5 => FileType::Link,
                other => FileType::Other(other),
            }
        }
    }

    /// The `fattr` structure.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct FileAttributes {
        pub file_type: FileType,
        pub mode: u32,
        pub nlink: u32,
        pub uid: u32,
        pub gid: u32,
        pub size: u32,
        pub blocksize: u32,
        pub rdev: u32,
        pub blocks: u32,
        pub fsid: u32,
        pub fileid: u32,
        pub atime: (u32, u32),
        pub mtime: (u32, u32),
        pub ctime: (u32, u32),
    }

    impl FileAttributes {
        pub fn decode(r: &mut XdrReader<'_>) -> Result<Self> {
            Ok(Self {
                file_type: FileType::from_u32(r.u32()?),
                mode: r.u32()?,
                nlink: r.u32()?,
                uid: r.u32()?,
                gid: r.u32()?,
                size: r.u32()?,
                blocksize: r.u32()?,
                rdev: r.u32()?,
                blocks: r.u32()?,
                fsid: r.u32()?,
                fileid: r.u32()?,
                atime: (r.u32()?, r.u32()?),
                mtime: (r.u32()?, r.u32()?),
                ctime: (r.u32()?, r.u32()?),
            })
        }
    }

    /// LOOKUP arguments.
    pub fn encode_directory_op_args(handle: &[u8; FILEHANDLE_SIZE], filename: &str) -> Vec<u8> {
        let mut w = XdrWriter::new();
        w.opaque_fixed(handle).string_utf16le(filename);
        w.into_bytes()
    }

    /// The body of a successful LOOKUP reply.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct DirectoryOpResponse {
        pub handle: [u8; FILEHANDLE_SIZE],
        pub attributes: FileAttributes,
    }

    /// Decode a LOOKUP reply. `None` when the status is not success.
    pub fn decode_directory_op_response(data: &[u8]) -> Result<Option<DirectoryOpResponse>> {
        let mut r = XdrReader::new(data);
        if r.u32()? != 0 {
            return Ok(None);
        }
        let mut handle = [0u8; FILEHANDLE_SIZE];
        handle.copy_from_slice(r.opaque_fixed(FILEHANDLE_SIZE)?);
        let attributes = FileAttributes::decode(&mut r)?;
        Ok(Some(DirectoryOpResponse { handle, attributes }))
    }

    /// READ arguments.
    pub fn encode_read_args(handle: &[u8; FILEHANDLE_SIZE], offset: u32, count: u32, total_count: u32) -> Vec<u8> {
        let mut w = XdrWriter::new();
        w.opaque_fixed(handle).u32(offset).u32(count).u32(total_count);
        w.into_bytes()
    }

    /// The body of a successful READ reply.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ReadResponse {
        pub attributes: FileAttributes,
        pub data: Vec<u8>,
    }

    /// Decode a READ reply. `None` when the status is not success.
    pub fn decode_read_response(data: &[u8]) -> Result<Option<ReadResponse>> {
        let mut r = XdrReader::new(data);
        if r.u32()? != 0 {
            return Ok(None);
        }
        let attributes = FileAttributes::decode(&mut r)?;
        let data = r.opaque_var()?.to_vec();
        Ok(Some(ReadResponse { attributes, data }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16le_strings_round_trip_with_padding() {
        let mut w = XdrWriter::new();
        w.string_utf16le("abc");
        let bytes = w.into_bytes();
        // 4 length + 6 data + 2 pad
        assert_eq!(bytes.len(), 12);
        assert_eq!(&bytes[..4], &[0, 0, 0, 6]);
        let mut r = XdrReader::new(&bytes);
        assert_eq!(r.string_utf16le().unwrap(), "abc");
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn rpc_request_and_reply() {
        let req = rpc::Request {
            rpc_version: 2,
            program: 100_000,
            program_version: 2,
            procedure: 3,
            auth: rpc::Auth { flavor: 1, body: vec![1, 2, 3] },
            verifier: rpc::Auth { flavor: 0, body: vec![] },
            data: vec![9, 9, 9, 9],
        };
        let bytes = rpc::encode_request(5, &req);
        assert_eq!(&bytes[..8], &[0, 0, 0, 5, 0, 0, 0, 0]);
        assert_eq!(bytes.len(), 8 + 16 + (4 + 4 + 4) + (4 + 4) + 4);

        let mut w = XdrWriter::new();
        w.u32(5).u32(1).u32(0).u32(0).opaque_var(&[]).u32(0).u32(0x1234);
        let reply = rpc::decode_reply(&w.into_bytes()).unwrap();
        assert_eq!(reply.xid, 5);
        assert_eq!(reply.body, rpc::ReplyBody::Success(vec![0, 0, 0x12, 0x34]));

        let mut w = XdrWriter::new();
        w.u32(5).u32(1).u32(1);
        assert_eq!(rpc::decode_reply(&w.into_bytes()).unwrap().body, rpc::ReplyBody::Denied);
    }

    #[test]
    fn export_list_decodes_linked_lists() {
        let mut w = XdrWriter::new();
        w.u32(1).string_utf16le("/C/").u32(1).string("*").u32(0);
        w.u32(1).string_utf16le("/B/").u32(0);
        w.u32(0);
        let exports = mount::decode_export_list(&w.into_bytes()).unwrap();
        assert_eq!(exports.len(), 2);
        assert_eq!(exports[0].filesystem, "/C/");
        assert_eq!(exports[0].groups, vec!["*"]);
        assert_eq!(exports[1].filesystem, "/B/");
        assert!(exports[1].groups.is_empty());
    }

    #[test]
    fn nfs_replies_decode() {
        let attrs = |w: &mut XdrWriter| {
            w.u32(1).u32(0).u32(1).u32(0).u32(0).u32(1234).u32(0).u32(0).u32(0).u32(0).u32(0);
            w.u32(0).u32(0).u32(0).u32(0).u32(0).u32(0);
        };
        let mut w = XdrWriter::new();
        w.u32(0).opaque_fixed(&[7u8; 32]);
        attrs(&mut w);
        let resp = nfs::decode_directory_op_response(&w.into_bytes()).unwrap().unwrap();
        assert_eq!(resp.handle, [7u8; 32]);
        assert_eq!(resp.attributes.size, 1234);
        assert_eq!(resp.attributes.file_type, nfs::FileType::Regular);

        let mut w = XdrWriter::new();
        w.u32(0);
        attrs(&mut w);
        w.opaque_var(&[1, 2, 3]);
        let read = nfs::decode_read_response(&w.into_bytes()).unwrap().unwrap();
        assert_eq!(read.data, vec![1, 2, 3]);

        let mut w = XdrWriter::new();
        w.u32(2);
        assert!(nfs::decode_read_response(&w.into_bytes()).unwrap().is_none());
    }
}
