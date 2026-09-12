//! Absolute playhead position reporting from the beat socket (50001).

use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::emitter::{Emitter, Listener};
use crate::status::types::{PositionState, VUState};
use crate::status::utils::{position_from_packet, vu_from_packet};
use crate::utils::udp::{Datagram, UdpFeed};

struct Inner {
    /// Fired when an absolute position packet is received from a CDJ-3000+.
    /// These packets are sent approximately every 30ms while a track is loaded.
    position: Emitter<PositionState>,
    /// Fired when real-time VU levels are received from the mixer under
    /// Stagehand connection.
    vu: Emitter<VUState>,
    /// Whether experimental Stagehand VU-meter (0x58) parsing is active.
    stagehand_mode: bool,
}

/// The position emitter reports absolute playhead position updates from
/// CDJ-3000+ devices. These packets provide precise track position
/// independent of beat grids, enabling accurate timecode, lighting cue, and
/// video synchronization even during scratching, reverse play, loops, and
/// needle jumps.
#[derive(Clone)]
pub struct PositionEmitter {
    inner: Arc<Inner>,
    _feed_task: Arc<JoinHandle<()>>,
}

impl std::fmt::Debug for PositionEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PositionEmitter").field("stagehand_mode", &self.inner.stagehand_mode).finish()
    }
}

impl PositionEmitter {
    /// Listen on the beat socket. `stagehand_mode` enables Stagehand-only
    /// VU-meter parsing. Off by default so non-Stagehand connections behave
    /// as they did before Stagehand.
    pub fn new(beat_feed: &UdpFeed, stagehand_mode: bool) -> Self {
        Self::from_packets(beat_feed.packets(), stagehand_mode)
    }

    /// Listen on any feed of beat-port datagrams.
    pub fn from_packets(packets: &Emitter<Datagram>, stagehand_mode: bool) -> Self {
        let inner = Arc::new(Inner { position: Emitter::new(), vu: Emitter::new(), stagehand_mode });

        let feed_task = {
            let inner = Arc::clone(&inner);
            let mut rx = packets.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(datagram) => Self::handle_position(&inner, &datagram.data),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
        };

        Self { inner, _feed_task: Arc::new(feed_task) }
    }

    /// Fired for every absolute position packet.
    pub fn position(&self) -> &Emitter<PositionState> {
        &self.inner.position
    }

    /// Fired for every VU packet (Stagehand mode only).
    pub fn vu(&self) -> &Emitter<VUState> {
        &self.inner.vu
    }

    /// Register a callback for position events.
    pub fn on_position<F: FnMut(PositionState) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.position.on(f)
    }

    /// Register a callback for VU events.
    pub fn on_vu<F: FnMut(VUState) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.vu.on(f)
    }

    /// Stop listening to the feed.
    pub fn stop(&self) {
        self._feed_task.abort();
    }

    fn handle_position(inner: &Inner, message: &[u8]) {
        // Stagehand VU meter (type 0x58) — only parsed in Stagehand mode.
        if inner.stagehand_mode && message.len() >= 11 && message[10] == 0x58 {
            if let Some(vu) = vu_from_packet(message) {
                inner.vu.emit(vu);
                return;
            }
        }

        if let Some(position) = position_from_packet(message) {
            inner.position.emit(position);
        }
    }
}
