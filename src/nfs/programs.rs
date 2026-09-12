//! The portmap, mount and NFS procedures this crate calls.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::nfs::rpc::{RpcCall, RpcConnection, RpcProgram};
use crate::nfs::xdr::{mount, nfs, portmap};
use crate::nfs::FetchProgress;
use crate::{Error, Result};

/// How many bytes of a file should we read at once.
pub const READ_SIZE: u32 = 8192;

/// Standard portmapper port used by CDJs and other hardware.
pub const STANDARD_PORTMAP_PORT: u16 = 111;

/// Non-standard portmapper port used by rekordbox software. Rekordbox
/// registers its RPC services on this port instead of 111.
pub const REKORDBOX_PORTMAP_PORT: u16 = 50111;

/// A program id and version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Program {
    pub id: u32,
    pub version: u32,
}

/// Queries for the listening port of a RPC program.
pub async fn make_program_client(conn: Arc<RpcConnection>, program: Program, portmap_port: u16) -> Result<RpcProgram> {
    let get_port = portmap::GetPort {
        program: program.id,
        version: program.version,
        // UDP protocol
        protocol: 17,
        port: 0,
    };

    let data = conn
        .call(RpcCall {
            port: portmap_port,
            program: portmap::PROGRAM,
            version: portmap::VERSION,
            procedure: portmap::procedure::GET_PORT,
            data: get_port.to_xdr(),
        })
        .await?;

    if data.len() < 4 {
        return Err(Error::Nfs("portmap GETPORT reply too short".into()));
    }
    let port = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);

    Ok(RpcProgram::new(conn, program.id, program.version, port as u16))
}

/// An NFS export on a remote system.
pub type Export = mount::ExportEntry;

/// A file handle.
pub type FileHandle = [u8; nfs::FILEHANDLE_SIZE];

/// Attributes of a remote file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    pub handle: FileHandle,
    pub name: String,
    pub size: u32,
    #[serde(rename = "type")]
    pub file_type: nfs::FileType,
}

/// Request a list of export entries.
pub async fn get_exports(conn: &RpcProgram) -> Result<Vec<Export>> {
    let data = conn.call(mount::procedure::EXPORT, Vec::new()).await?;
    mount::decode_export_list(&data)
}

/// Mount the specified export, returning the file handle.
pub async fn mount_filesystem(conn: &RpcProgram, export: &Export) -> Result<FileHandle> {
    let resp = conn.call(mount::procedure::MOUNT, mount::encode_mount_request(&export.filesystem)).await?;
    mount::decode_fh_status(&resp)?.ok_or_else(|| Error::Nfs("Failed to mount filesystem".into()))
}

/// Lookup a file within the directory of the provided file handle, returning
/// the [`FileInfo`] if the file can be located.
pub async fn lookup_file(conn: &RpcProgram, handle: &FileHandle, filename: &str) -> Result<FileInfo> {
    let resp = conn.call(nfs::procedure::LOOKUP, nfs::encode_directory_op_args(handle, filename)).await?;
    let body =
        nfs::decode_directory_op_response(&resp)?.ok_or_else(|| Error::Nfs(format!("Failed file lookup of {filename}")))?;

    Ok(FileInfo {
        name: filename.to_string(),
        handle: body.handle,
        size: body.attributes.size,
        file_type: body.attributes.file_type,
    })
}

/// Lookup the absolute path to a file, given the root file handle and path.
pub async fn lookup_path(conn: &RpcProgram, root_handle: &FileHandle, filepath: &str) -> Result<FileInfo> {
    // There are times when the path includes a leading slash, sanitize that
    let path = filepath.strip_prefix('/').unwrap_or(filepath);

    let mut handle = *root_handle;
    let mut info: Option<FileInfo> = None;

    for filename in path.split('/') {
        let file_info = lookup_file(conn, &handle, filename).await?;
        handle = file_info.handle;
        info = Some(file_info);
    }

    info.ok_or_else(|| Error::Nfs(format!("empty path: {filepath:?}")))
}

/// Fetch the specified file from the remote NFS server. This will read the
/// entire file into memory.
pub async fn fetch_file(
    conn: &RpcProgram,
    file: &FileInfo,
    mut on_progress: Option<&mut (dyn FnMut(FetchProgress) + Send)>,
    read_size: Option<u32>,
) -> Result<Vec<u8>> {
    let read_size = read_size.filter(|s| *s > 0).unwrap_or(READ_SIZE);
    if read_size > READ_SIZE {
        return Err(Error::Nfs(format!("Maximum read size for NFS is {READ_SIZE}. You specified: {read_size}")));
    }

    let size = file.size;
    let mut data = vec![0u8; size as usize];
    let mut bytes_read: u32 = 0;

    while bytes_read < size {
        let resp = conn.call(nfs::procedure::READ, nfs::encode_read_args(&file.handle, bytes_read, read_size, 0)).await?;
        let body = nfs::decode_read_response(&resp)?
            .ok_or_else(|| Error::Nfs(format!("Failed to read file at offset {bytes_read} / {size}")))?;

        if body.data.is_empty() {
            return Err(Error::Nfs(format!("Short read at offset {bytes_read} / {size}")));
        }

        let end = (bytes_read as usize + body.data.len()).min(data.len());
        let n = end - bytes_read as usize;
        data[bytes_read as usize..end].copy_from_slice(&body.data[..n]);
        bytes_read += n as u32;

        if let Some(cb) = on_progress.as_deref_mut() {
            cb(FetchProgress { read: bytes_read, total: size });
        }
    }

    Ok(data)
}

/// Fetch a range of bytes from a file on the remote NFS server. Unlike
/// [`fetch_file`], this only reads the specified range.
pub async fn fetch_file_range(conn: &RpcProgram, file: &FileInfo, offset: u32, length: u32) -> Result<Vec<u8>> {
    let mut data = Vec::with_capacity(length as usize);
    let mut bytes_read: u32 = 0;

    while bytes_read < length {
        let chunk_size = READ_SIZE.min(length - bytes_read);
        let resp =
            conn.call(nfs::procedure::READ, nfs::encode_read_args(&file.handle, offset + bytes_read, chunk_size, 0)).await?;
        let body = nfs::decode_read_response(&resp)?
            .ok_or_else(|| Error::Nfs(format!("Failed to read file at offset {}", offset + bytes_read)))?;

        if body.data.is_empty() {
            break;
        }

        let take = body.data.len().min((length - bytes_read) as usize);
        data.extend_from_slice(&body.data[..take]);
        bytes_read += take as u32;
    }

    Ok(data)
}
