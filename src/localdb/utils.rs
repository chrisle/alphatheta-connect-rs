//! Cue helpers shared by the remotedb and ANLZ converters.

use crate::entities::{CueAndLoop, HotcueButton};

/// Create a CueAndLoop entry given common parameters. `None` when the entry
/// is neither a cue nor a loop (likely a leftover that was removed).
pub fn make_cue_loop_entry(
    is_cue: bool,
    is_loop: bool,
    offset: f64,
    length: f64,
    button: Option<HotcueButton>,
) -> Option<CueAndLoop> {
    match (button, is_loop, is_cue) {
        (Some(button), true, _) => Some(CueAndLoop::HotLoop { offset, length, button, label: None, color: None }),
        (Some(button), false, _) => Some(CueAndLoop::HotCue { offset, button, label: None, color: None }),
        (None, true, _) => Some(CueAndLoop::Loop { offset, length, label: None, color: None }),
        (None, false, true) => Some(CueAndLoop::CuePoint { offset, label: None, color: None }),
        (None, false, false) => None,
    }
}
