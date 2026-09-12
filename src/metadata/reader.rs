//! File reader implementations.

use crate::metadata::types::FileReader;
use crate::Result;

/// A [`FileReader`] over an in-memory buffer (for testing or in-memory data).
#[derive(Debug, Clone)]
pub struct BufferReader {
    data: Vec<u8>,
    extension: String,
}

impl BufferReader {
    pub fn new(data: Vec<u8>, extension: impl Into<String>) -> Self {
        Self { data, extension: extension.into() }
    }
}

impl FileReader for BufferReader {
    fn size(&self) -> u64 {
        self.data.len() as u64
    }

    fn extension(&self) -> &str {
        &self.extension
    }

    async fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>> {
        let start = (offset as usize).min(self.data.len());
        let end = ((offset + length) as usize).min(self.data.len());
        Ok(self.data[start..end].to_vec())
    }
}

/// Create a [`FileReader`] from a buffer.
pub fn create_buffer_reader(data: Vec<u8>, extension: impl Into<String>) -> BufferReader {
    BufferReader::new(data, extension)
}
