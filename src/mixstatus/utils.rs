//! Play-state classification helpers.

use crate::status::types::{PlayState, State};

/// Returns true if the status reports a playing state.
pub fn is_playing(s: &State) -> bool {
    matches!(s.play_state, PlayState::Playing | PlayState::Looping)
}

/// Returns true if the status reports a stopping state.
pub fn is_stopping(s: &State) -> bool {
    matches!(s.play_state, PlayState::Cued | PlayState::Ended | PlayState::Loading)
}
