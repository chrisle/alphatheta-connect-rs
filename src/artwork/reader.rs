//! File readers backed by NFS and by memory.

use crate::metadata::types::FileReader;
use crate::nfs::{fetch_file_range, get_file_info};
use crate::types::{Device, MediaSlot};
use crate::Result;

pub use crate::metadata::reader::{create_buffer_reader, BufferReader};

/// A [`FileReader`] backed by NFS that reads from a device's media slot.
#[derive(Debug, Clone)]
pub struct NfsFileReader {
    device: Device,
    slot: MediaSlot,
    path: String,
    size: u64,
    extension: String,
}

impl NfsFileReader {
    pub fn new(device: Device, slot: MediaSlot, path: String, size: u64) -> Self {
        let extension = path.rsplit('.').next().unwrap_or("").to_lowercase();
        Self { device, slot, path, size, extension }
    }
}

impl FileReader for NfsFileReader {
    fn size(&self) -> u64 {
        self.size
    }

    fn extension(&self) -> &str {
        &self.extension
    }

    async fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>> {
        let offset = u32::try_from(offset).unwrap_or(u32::MAX);
        let length = u32::try_from(length).unwrap_or(u32::MAX);
        fetch_file_range(&self.device, self.slot, &self.path, offset, length).await
    }
}

/// Create a [`FileReader`] backed by NFS that reads from a device's media slot.
pub fn create_nfs_file_reader(device: &Device, slot: MediaSlot, path: &str, file_size: u64) -> NfsFileReader {
    NfsFileReader::new(device.clone(), slot, path.to_string(), file_size)
}

/// Create a [`FileReader`] backed by NFS, automatically fetching the file size.
pub async fn create_nfs_file_reader_with_info(device: &Device, slot: MediaSlot, path: &str) -> Result<NfsFileReader> {
    let info = get_file_info(device, slot, path).await?;
    Ok(create_nfs_file_reader(device, slot, path, u64::from(info.size)))
}
