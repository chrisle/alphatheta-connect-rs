//! CDJ status from captured status packets.

use std::sync::{Arc, Mutex};

use tokio::task::JoinHandle;

use crate::emitter::{Emitter, Listener};
use crate::passive::pcap_adapter::PcapAdapter;
use crate::status::types::{MixerState, OnAirStatus, State};
use crate::status::utils::{media_slot_from_packet, on_air_from_packet, status_from_packet};
use crate::types::MediaSlotInfo;

struct Inner {
    status: Emitter<State>,
    media_slot: Emitter<MediaSlotInfo>,
    on_air: Emitter<OnAirStatus>,
    /// Part of the shared status event API (matching the active status
    /// emitter) so the two are interchangeable to consumers; passive capture
    /// does not currently produce this event.
    mixer_state: Emitter<MixerState>,
    feed_task: Mutex<Option<JoinHandle<()>>>,
}

/// Reports CDJ status updates received via passive packet capture instead of
/// UDP sockets.
///
/// It provides the same event API as the active
/// [`StatusEmitter`](crate::status::StatusEmitter), but does NOT support
/// media slot queries since that requires sending packets. In passive mode,
/// media slot information is received when CDJs broadcast it naturally
/// (e.g., when media is inserted or at startup).
#[derive(Clone)]
pub struct PassiveStatusEmitter {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PassiveStatusEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassiveStatusEmitter").finish()
    }
}

impl PassiveStatusEmitter {
    pub fn new(adapter: &PcapAdapter) -> Self {
        let inner = Arc::new(Inner {
            status: Emitter::new(),
            media_slot: Emitter::new(),
            on_air: Emitter::new(),
            mixer_state: Emitter::new(),
            feed_task: Mutex::new(None),
        });

        let task = {
            let inner = Arc::clone(&inner);
            let mut rx = adapter.status().subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(datagram) => Self::handle_status(&inner, &datagram.data),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
        };
        *inner.feed_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);

        Self { inner }
    }

    pub fn status(&self) -> &Emitter<State> {
        &self.inner.status
    }

    pub fn media_slot(&self) -> &Emitter<MediaSlotInfo> {
        &self.inner.media_slot
    }

    pub fn on_air(&self) -> &Emitter<OnAirStatus> {
        &self.inner.on_air
    }

    pub fn mixer_state(&self) -> &Emitter<MixerState> {
        &self.inner.mixer_state
    }

    pub fn on_status<F: FnMut(State) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.status.on(f)
    }

    pub fn on_media_slot<F: FnMut(MediaSlotInfo) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.media_slot.on(f)
    }

    pub fn on_on_air<F: FnMut(OnAirStatus) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.on_air.on(f)
    }

    /// Stop listening to the pcap adapter.
    pub fn stop(&self) {
        if let Some(t) = self.inner.feed_task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            t.abort();
        }
    }

    fn handle_status(inner: &Inner, message: &[u8]) {
        // Malformed packets are ignored.
        if let Ok(Some(status)) = status_from_packet(message) {
            inner.status.emit(status);
            return;
        }

        // Media slot status is also reported on this socket
        if let Ok(Some(media_slot)) = media_slot_from_packet(message) {
            inner.media_slot.emit(media_slot);
            return;
        }

        // On-air status from mixer is also reported on this socket
        if let Some(on_air) = on_air_from_packet(message) {
            inner.on_air.emit(on_air);
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(t) = self.feed_task.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            t.abort();
        }
    }
}
