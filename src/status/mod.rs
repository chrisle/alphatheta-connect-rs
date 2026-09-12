//! CDJ status reporting: the status socket (50002) parsed into events.

pub mod media;
pub mod position;
pub mod types;
pub mod utils;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::constants::STATUS_PORT;
use crate::emitter::{Emitter, Listener};
use crate::status::types::{MixerState, OnAirStatus, State};
use crate::types::{Device, MediaSlot, MediaSlotInfo};
use crate::utils::udp::{Datagram, UdpFeed};
use crate::{Error, Result};

pub use media::make_media_slot_request;
pub use utils::{
    media_slot_from_packet, mixer_state_from_packet, on_air_from_packet, position_from_packet, status_from_packet, vu_from_packet,
};

/// How long to wait for a media slot response.
const MEDIA_SLOT_TIMEOUT: Duration = Duration::from_millis(10_000);

struct Inner {
    /// Fired each time the CDJ reports its status.
    status: Emitter<State>,
    /// Fired when the CDJ reports its media slot status.
    media_slot: Emitter<MediaSlotInfo>,
    /// Fired when the mixer broadcasts on-air channel status.
    on_air: Emitter<OnAirStatus>,
    /// Fired when the Stagehand-connected mixer reports fader/EQ/control positions.
    mixer_state: Emitter<MixerState>,
    /// Lock used to avoid media slot query races.
    media_slot_query_lock: Mutex<()>,
    /// Whether the experimental Stagehand mixer-state (0x39) parsing is
    /// active. Only true when the network was brought online in 'stagehand'
    /// mode.
    stagehand_mode: bool,
}

/// The status emitter will report every time a device status is received.
///
/// Cloning yields another handle on the same emitter.
#[derive(Clone)]
pub struct StatusEmitter {
    inner: Arc<Inner>,
    feed: Arc<UdpFeed>,
    _feed_task: Arc<JoinHandle<()>>,
}

impl std::fmt::Debug for StatusEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusEmitter").field("stagehand_mode", &self.inner.stagehand_mode).finish()
    }
}

impl StatusEmitter {
    /// Listen on the status socket.
    ///
    /// `stagehand_mode` enables Stagehand-only mixer-state parsing. Off by
    /// default so 'active' and passive connections behave exactly as they did
    /// before Stagehand was added — the 0x39 layout is reverse-engineered
    /// from a DJM-A9 and must not be applied to other mixers (e.g. XDJ-AZ)
    /// that were never captured.
    pub fn new(status_feed: Arc<UdpFeed>, stagehand_mode: bool) -> Self {
        let inner = Arc::new(Inner {
            status: Emitter::new(),
            media_slot: Emitter::new(),
            on_air: Emitter::new(),
            mixer_state: Emitter::new(),
            media_slot_query_lock: Mutex::new(()),
            stagehand_mode,
        });

        let feed_task = {
            let inner = Arc::clone(&inner);
            let mut rx = status_feed.packets().subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(datagram) => Self::handle_status(&inner, &datagram),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
        };

        Self { inner, feed: status_feed, _feed_task: Arc::new(feed_task) }
    }

    /// Fired each time a CDJ reports its status.
    pub fn status(&self) -> &Emitter<State> {
        &self.inner.status
    }

    /// Fired when a CDJ reports its media slot status.
    pub fn media_slot(&self) -> &Emitter<MediaSlotInfo> {
        &self.inner.media_slot
    }

    /// Fired when the mixer broadcasts on-air channel status.
    pub fn on_air(&self) -> &Emitter<OnAirStatus> {
        &self.inner.on_air
    }

    /// Fired when the Stagehand-connected mixer reports fader/EQ/control positions.
    pub fn mixer_state(&self) -> &Emitter<MixerState> {
        &self.inner.mixer_state
    }

    /// A receiver of every status event.
    pub fn subscribe_status(&self) -> tokio::sync::broadcast::Receiver<State> {
        self.inner.status.subscribe()
    }

    /// Register a callback for every status event.
    pub fn on_status<F: FnMut(State) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.status.on(f)
    }

    /// Register a callback for every media slot event.
    pub fn on_media_slot<F: FnMut(MediaSlotInfo) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.media_slot.on(f)
    }

    /// Register a callback for every on-air event.
    pub fn on_on_air<F: FnMut(OnAirStatus) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.on_air.on(f)
    }

    /// Register a callback for every mixer-state event.
    pub fn on_mixer_state<F: FnMut(MixerState) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.mixer_state.on(f)
    }

    fn handle_status(inner: &Inner, datagram: &Datagram) {
        let message: &[u8] = &datagram.data;

        // Stagehand mixer state (type 0x39). Only parsed in Stagehand mode —
        // the layout is DJM-A9-specific and applying it to other mixers
        // misreads their faders (NP3-327). In 'active'/passive mode we fall
        // back to the on-air flag, which is how on-air detection worked
        // before Stagehand.
        if inner.stagehand_mode && message.len() >= 11 && message[10] == 0x39 {
            if let Some(mixer_state) = mixer_state_from_packet(message) {
                inner.mixer_state.emit(mixer_state);
                return;
            }
        }

        match status_from_packet(message) {
            Ok(Some(status)) => {
                inner.status.emit(status);
                return;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::trace!(target: "alphatheta_connect", "ignoring status packet: {e}");
                return;
            }
        }

        // Media slot status is also reported on this socket
        match media_slot_from_packet(message) {
            Ok(Some(media_slot)) => {
                inner.media_slot.emit(media_slot);
                return;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::trace!(target: "alphatheta_connect", "ignoring media slot packet: {e}");
                return;
            }
        }

        // On-air status from mixer is also reported on this socket
        if let Some(on_air) = on_air_from_packet(message) {
            inner.on_air.emit(on_air);
        }
    }

    /// Retrieve media slot status information.
    ///
    /// - `host_device`: the device asking for media info.
    /// - `device`: the target device to query.
    /// - `slot`: the specific slot.
    pub async fn query_media_slot(&self, host_device: &Device, device: &Device, slot: MediaSlot) -> Result<MediaSlotInfo> {
        let request = make_media_slot_request(host_device, device, slot);

        let _guard = self.inner.media_slot_query_lock.lock().await;

        // Subscribe before sending so the reply cannot slip past us.
        let mut rx = self.inner.media_slot.subscribe();
        self.feed.send_to(&request, device.ip, STATUS_PORT).await?;

        let device_id = device.id;
        let wait = async {
            loop {
                match rx.recv().await {
                    // Only resolve if this is for our device and slot
                    Ok(info) if info.device_id == device_id && info.slot == slot => return Ok(info),
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(Error::State("status emitter closed".into()))
                    }
                }
            }
        };

        match tokio::time::timeout(MEDIA_SLOT_TIMEOUT, wait).await {
            Ok(r) => r,
            Err(_) => Err(Error::Timeout(format!("Timeout waiting for media slot response from device {device_id}"))),
        }
    }
}
