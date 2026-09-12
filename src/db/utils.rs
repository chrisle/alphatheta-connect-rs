//! Helpers for the database service.

use std::future::Future;
use std::pin::Pin;

use crate::nfs::{fetch_file, FetchFileOptions};
use crate::types::{Device, MediaSlot};
use crate::Result;

/// An ANLZ resolver that fetches analysis files from a device over NFS.
pub fn anlz_loader<'a>(
    device: &'a Device,
    slot: MediaSlot,
) -> impl Fn(String) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> + Send + Sync + 'a {
    move |path: String| Box::pin(async move { fetch_file(device, slot, &path, FetchFileOptions::default()).await })
}
