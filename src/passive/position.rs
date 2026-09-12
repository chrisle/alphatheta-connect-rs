//! Absolute playhead position from captured beat packets.

use crate::passive::pcap_adapter::PcapAdapter;
use crate::status::position::PositionEmitter;

/// Reports absolute playhead position updates from CDJ-3000+ devices using
/// passive packet capture. Position packets provide precise track position
/// independent of beat grids, enabling accurate timecode, lighting cue, and
/// video synchronization even during scratching, reverse play, loops, and
/// needle jumps.
///
/// This is the active [`PositionEmitter`] fed from the capture's beat
/// packets; the VU (Stagehand) parsing stays off.
pub type PassivePositionEmitter = PositionEmitter;

/// Create a position emitter over the adapter's beat packets.
pub fn passive_position_emitter(adapter: &PcapAdapter) -> PassivePositionEmitter {
    PositionEmitter::from_packets(adapter.beat(), false)
}
