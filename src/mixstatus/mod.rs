//! The mix status processor: turns the stream of player states into
//! "now playing" / "stopped" / set lifecycle events.

pub mod utils;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::emitter::{Emitter, Listener};
use crate::status::types::State;
use crate::types::{DeviceId, MixstatusMode};
use crate::utils::{bpm_to_seconds, now_ms};

pub use utils::{is_playing, is_stopping};

/// Configuration for the [`MixstatusProcessor`].
#[derive(Debug, Clone, PartialEq)]
pub struct MixstatusConfig {
    /// Selects the mixstatus reporting mode.
    pub mode: MixstatusMode,
    /// Specifies the duration in seconds that no tracks must be on air. This
    /// can be thought of as how long 'air silence' is reasonable in a set
    /// before a separate one is considered to have begun.
    ///
    /// Default: 30 (half a minute).
    pub time_between_sets: f64,
    /// Indicates if the status objects reported should have their on-air
    /// flag read. Setting this to false will degrade the functionality of the
    /// processor such that it will not consider the value of `is_on_air` and
    /// always assume CDJs are live.
    ///
    /// Default: true.
    pub use_on_air_status: bool,
    /// Configures how many beats a track may not be live or playing for it to
    /// still be considered active.
    ///
    /// Default: 8 (two bars).
    pub allowed_interrupt_beats: f64,
    /// Configures how many beats the track must consecutively be playing for
    /// (since the beat it was cued at) until the track is considered to be
    /// active. Used for [`MixstatusMode::SmartTiming`].
    ///
    /// Default: 128 (2 phrases).
    pub beats_until_reported: f64,
}

impl Default for MixstatusConfig {
    fn default() -> Self {
        Self {
            mode: MixstatusMode::SmartTiming,
            time_between_sets: 30.0,
            allowed_interrupt_beats: 8.0,
            beats_until_reported: 128.0,
            use_on_air_status: true,
        }
    }
}

/// A partial configuration, for [`MixstatusProcessor::configure`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MixstatusConfigUpdate {
    pub mode: Option<MixstatusMode>,
    pub time_between_sets: Option<f64>,
    pub use_on_air_status: Option<bool>,
    pub allowed_interrupt_beats: Option<f64>,
    pub beats_until_reported: Option<f64>,
}

/// Payload of the `stopped` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped {
    pub device_id: DeviceId,
}

struct ProcessorState {
    /// Records the most recent state of each player.
    last_state: HashMap<DeviceId, State>,
    /// Records when each device last started playing a track.
    last_start_time: HashMap<DeviceId, u64>,
    /// Records when a device entered a 'may stop' state. If it's in the state
    /// for long enough it will be reported as stopped.
    last_stopped_times: HashMap<DeviceId, u64>,
    /// Records which players have been reported as 'live'.
    live_players: HashSet<DeviceId>,
    /// Records the last track id emitted as 'now playing' for each device.
    /// Used to suppress duplicate emissions when a deck is briefly
    /// cued/loaded and then resumed on the same track (a common pattern for
    /// House/Techno DJs who cue-juggle mid-track). The entry is replaced when
    /// a different track id is emitted on the same deck, so subsequent track
    /// loads still emit normally.
    last_emitted_track_id: HashMap<DeviceId, u32>,
    /// Indicates if we're currently in an active DJ set.
    is_set_active: bool,
    /// When we are waiting for a set to end, use this to cancel the timer.
    set_ending: Option<JoinHandle<()>>,
    config: MixstatusConfig,
}

struct Inner {
    state: Mutex<ProcessorState>,
    /// Fired when a track is considered to be on-air and is being heard by
    /// the audience.
    now_playing: Emitter<State>,
    /// Fired when a track has stopped and is completely off-air.
    stopped: Emitter<Stopped>,
    /// Fired when a DJ set first starts.
    set_started: Emitter<()>,
    /// Fired when tracks have been stopped.
    set_ended: Emitter<()>,
}

/// `MixstatusProcessor` is a configurable processor which when fed device
/// state will attempt to accurately determine events that happen within the
/// DJ set.
///
/// The following events are fired:
///
/// - now_playing: The track is considered playing and on air to the audience.
/// - stopped:     The track was stopped / paused / went off-air.
///
/// Additionally the following non-track status are reported:
///
/// - set_started: The first track has begun playing.
/// - set_ended:   The time_between_sets has passed since any tracks were live.
///
/// Config options may be changed after the processor has been created and is
/// actively receiving state updates.
#[derive(Clone)]
pub struct MixstatusProcessor {
    inner: Arc<Inner>,
}

impl Default for MixstatusProcessor {
    fn default() -> Self {
        Self::new(None)
    }
}

impl MixstatusProcessor {
    pub fn new(config: Option<MixstatusConfig>) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(ProcessorState {
                    last_state: HashMap::new(),
                    last_start_time: HashMap::new(),
                    last_stopped_times: HashMap::new(),
                    live_players: HashSet::new(),
                    last_emitted_track_id: HashMap::new(),
                    is_set_active: false,
                    set_ending: None,
                    config: config.unwrap_or_default(),
                }),
                now_playing: Emitter::new(),
                stopped: Emitter::new(),
                set_started: Emitter::new(),
                set_ended: Emitter::new(),
            }),
        }
    }

    /// Update the configuration.
    pub fn configure(&self, update: MixstatusConfigUpdate) {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = update.mode {
            st.config.mode = v;
        }
        if let Some(v) = update.time_between_sets {
            st.config.time_between_sets = v;
        }
        if let Some(v) = update.use_on_air_status {
            st.config.use_on_air_status = v;
        }
        if let Some(v) = update.allowed_interrupt_beats {
            st.config.allowed_interrupt_beats = v;
        }
        if let Some(v) = update.beats_until_reported {
            st.config.beats_until_reported = v;
        }
    }

    /// The current configuration.
    pub fn config(&self) -> MixstatusConfig {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner()).config.clone()
    }

    pub fn now_playing(&self) -> &Emitter<State> {
        &self.inner.now_playing
    }

    pub fn stopped(&self) -> &Emitter<Stopped> {
        &self.inner.stopped
    }

    pub fn set_started(&self) -> &Emitter<()> {
        &self.inner.set_started
    }

    pub fn set_ended(&self) -> &Emitter<()> {
        &self.inner.set_ended
    }

    pub fn on_now_playing<F: FnMut(State) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.now_playing.on(f)
    }

    pub fn on_stopped<F: FnMut(Stopped) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.stopped.on(f)
    }

    pub fn on_set_started<F: FnMut(()) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.set_started.on(f)
    }

    pub fn on_set_ended<F: FnMut(()) + Send + 'static>(&self, f: F) -> Listener {
        self.inner.set_ended.on(f)
    }

    /// Helper to account for the use_on_air_status config. If not configured
    /// with this flag the state will always be determined as on air.
    fn on_air(st: &ProcessorState, state: &State) -> bool {
        if st.config.use_on_air_status {
            state.is_on_air
        } else {
            true
        }
    }

    /// Report a player as 'live'. Will not report the state if the player has
    /// already previously been reported as live.
    fn promote_player(inner: &Arc<Inner>, st: &mut ProcessorState, state: &State) {
        let device_id = state.device_id;

        if !Self::on_air(st, state) || !is_playing(state) {
            return;
        }

        if st.live_players.contains(&device_id) {
            return;
        }

        // Suppress duplicate now_playing for the same track on the same deck.
        // This can happen when a DJ cues mid-track and resumes (Cued /
        // Loading / Ended are all stopping states that drop the deck from
        // live_players, so the next play of the same track id would otherwise
        // re-emit). When a different track is loaded, the track id differs
        // and the new emission proceeds normally.
        if st.last_emitted_track_id.get(&device_id) == Some(&state.track_id) {
            return;
        }

        if !st.is_set_active {
            st.is_set_active = true;
            inner.set_started.emit(());
        }

        if let Some(t) = st.set_ending.take() {
            t.abort();
        }

        st.live_players.insert(device_id);
        st.last_emitted_track_id.insert(device_id, state.track_id);

        inner.now_playing.emit(state.clone());
    }

    /// Locate the player that has been playing for the longest time and is on
    /// air, and report that device as now playing.
    fn promote_next_player(inner: &Arc<Inner>, st: &mut ProcessorState) {
        let longest_playing_id = st
            .last_start_time
            .iter()
            .filter(|(id, _)| !st.live_players.contains(id))
            .filter(|(id, _)| st.last_state.get(id).is_some_and(is_playing))
            .min_by_key(|(_, started_at)| **started_at)
            .map(|(id, _)| *id);

        // No other players currently playing?
        let Some(id) = longest_playing_id else {
            Self::set_may_stop(inner, st);
            return;
        };

        // We know this value is available since we have a live player playing
        let next_state = st.last_state.get(&id).cloned();
        if let Some(next_state) = next_state {
            Self::promote_player(inner, st, &next_state);
        }
    }

    fn mark_player_stopped(inner: &Arc<Inner>, st: &mut ProcessorState, state: &State) {
        let device_id = state.device_id;
        st.last_stopped_times.remove(&device_id);
        st.last_start_time.remove(&device_id);
        st.live_players.remove(&device_id);

        Self::promote_next_player(inner, st);
        inner.stopped.emit(Stopped { device_id });
    }

    fn set_may_stop(inner: &Arc<Inner>, st: &mut ProcessorState) {
        // We handle the set ending as an async timeout as in the case with a
        // set ending, the DJ may immediately turn off the CDJs, stopping state
        // packets meaning we can't process on a heartbeat.
        if !st.is_set_active {
            return;
        }

        // If any tracks are still playing the set has not ended
        if st.last_state.values().any(|s| is_playing(s) && Self::on_air(st, s)) {
            return;
        }

        if let Some(t) = st.set_ending.take() {
            t.abort();
        }

        let wait = Duration::from_secs_f64(st.config.time_between_sets.max(0.0));
        let task_inner = Arc::clone(inner);
        st.set_ending = Some(tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let mut st = task_inner.state.lock().unwrap_or_else(|e| e.into_inner());
            st.set_ending = None;
            if !st.is_set_active {
                return;
            }
            task_inner.set_ended.emit(());
        }));
    }

    /// Called to indicate that we think this player may be the first one to
    /// start playing. Will check if no other players are playing, if so it
    /// will report the player as now playing.
    fn player_may_be_first(inner: &Arc<Inner>, st: &mut ProcessorState, state: &State) {
        let other_players_playing = st
            .last_state
            .values()
            .filter(|other| other.device_id != state.device_id)
            .any(|other| Self::on_air(st, other) && is_playing(other));

        if other_players_playing {
            return;
        }

        Self::promote_player(inner, st, state);
    }

    /// Called when the player is in a state where it is no longer playing,
    /// but may come back on air. Examples are slip pause, or 'cutting' a
    /// track on the mixer taking it off air.
    fn player_may_stop(st: &mut ProcessorState, state: &State) {
        st.last_stopped_times.insert(state.device_id, now_ms());
    }

    /// Called to indicate that a device has reported a different play state
    /// than it had previously reported.
    fn handle_playstate_change(inner: &Arc<Inner>, st: &mut ProcessorState, last_state: &State, state: &State) {
        let device_id = state.device_id;

        let is_following_master = st.config.mode == MixstatusMode::FollowsMaster && state.is_master;

        let now_playing = is_playing(state);
        let was_playing = is_playing(last_state);

        let is_now_playing = now_playing && !was_playing;

        // Was this device in a 'may stop' state and it has begun on-air
        // playing again?
        if st.last_stopped_times.contains_key(&device_id) && now_playing && Self::on_air(st, state) {
            st.last_stopped_times.remove(&device_id);
            return;
        }

        if is_now_playing && is_following_master {
            Self::promote_player(inner, st, state);
        }

        if is_now_playing {
            st.last_start_time.insert(device_id, now_ms());
            Self::player_may_be_first(inner, st, state);
            return;
        }

        if was_playing && is_stopping(state) {
            Self::mark_player_stopped(inner, st, state);
            return;
        }

        if was_playing && !now_playing {
            Self::player_may_stop(st, state);
        }
    }

    fn handle_onair_change(inner: &Arc<Inner>, st: &mut ProcessorState, state: &State) {
        let device_id = state.device_id;

        // Player may have just been brought on with nothing else playing
        Self::player_may_be_first(inner, st, state);

        if !st.live_players.contains(&device_id) {
            return;
        }

        if !Self::on_air(st, state) {
            Self::player_may_stop(st, state);
            return;
        }

        // Play has come back on air
        st.last_stopped_times.remove(&device_id);
    }

    /// Feed a CDJ status state to the mix state processor.
    pub fn handle_state(&self, state: State) {
        let inner = &self.inner;
        let mut st = inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let st = &mut *st;

        let device_id = state.device_id;
        let play_state = state.play_state;

        let last_state = st.last_state.insert(device_id, state.clone());

        // If this is the first time we've heard from this CDJ, and it is on
        // air and playing, report it immediately. This is different from
        // reporting the first playing track, as the CDJ will have already sent
        // many states.
        if last_state.is_none() && Self::on_air(st, &state) && is_playing(&state) {
            st.last_start_time.insert(device_id, now_ms());
            Self::player_may_be_first(inner, st, &state);
            return;
        }

        // Play state has changed since this player last reported
        if let Some(last) = &last_state {
            if last.play_state != play_state {
                Self::handle_playstate_change(inner, st, last, &state);
            }
        }

        if let Some(last) = &last_state {
            if Self::on_air(st, last) != Self::on_air(st, &state) {
                Self::handle_onair_change(inner, st, &state);
            }
        }

        // Are we simply following master?
        if st.config.mode == MixstatusMode::FollowsMaster && last_state.as_ref().is_some_and(|l| !l.is_master) && state.is_master
        {
            Self::promote_player(inner, st, &state);
            return;
        }

        let now = now_ms();
        let seconds_per_beat = bpm_to_seconds(state.track_bpm.unwrap_or(f64::NAN), state.effective_pitch);

        // If a device has been playing for the required number of beats, we
        // may be able to report it as live
        let required_play_time = st.config.beats_until_reported * seconds_per_beat * 1000.0;
        if st.config.mode == MixstatusMode::SmartTiming {
            if let Some(started_at) = st.last_start_time.get(&device_id).copied() {
                if required_play_time <= now.saturating_sub(started_at) as f64 {
                    Self::promote_player(inner, st, &state);
                }
            }
        }

        // If a device has been in a 'potentially stopped' state for long
        // enough, we can mark the track as truly stopped.
        let required_stop_time = st.config.allowed_interrupt_beats * seconds_per_beat * 1000.0;
        if let Some(stopped_at) = st.last_stopped_times.get(&device_id).copied() {
            if required_stop_time <= now.saturating_sub(stopped_at) as f64 {
                Self::mark_player_stopped(inner, st, &state);
            }
        }
    }

    /// Manually reports the track that has been playing the longest which has
    /// not yet been reported as live.
    pub fn trigger_next_track(&self) {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::promote_next_player(&self.inner, &mut st);
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(t) = self.state.get_mut().unwrap_or_else(|e| e.into_inner()).set_ending.take() {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::types::PlayState;
    use crate::types::{MediaSlot, TrackType};

    fn state(device_id: u8, track_id: u32, play_state: PlayState, on_air: bool) -> State {
        State {
            device_id,
            track_id,
            track_device_id: device_id,
            track_slot: MediaSlot::Usb,
            track_type: TrackType::Rb,
            play_state,
            is_on_air: on_air,
            is_sync: false,
            is_bpm_sync: false,
            is_master: false,
            is_emergency_mode: false,
            track_bpm: Some(128.0),
            effective_pitch: 0.0,
            slider_pitch: 0.0,
            beat_in_measure: 1,
            beats_until_cue: None,
            beat: Some(1),
            device_type: 0x0f,
            packet_num: 0,
        }
    }

    #[tokio::test]
    async fn first_on_air_playing_player_is_now_playing() {
        let p = MixstatusProcessor::new(None);
        let mut np = p.now_playing().subscribe();
        let mut started = p.set_started().subscribe();
        p.handle_state(state(1, 10, PlayState::Playing, true));
        assert_eq!(np.try_recv().unwrap().track_id, 10);
        assert!(started.try_recv().is_ok());
    }

    #[tokio::test]
    async fn second_player_waits_while_first_plays() {
        let p = MixstatusProcessor::new(None);
        let mut np = p.now_playing().subscribe();
        p.handle_state(state(1, 10, PlayState::Playing, true));
        assert!(np.try_recv().is_ok());
        p.handle_state(state(2, 20, PlayState::Cued, true));
        p.handle_state(state(2, 20, PlayState::Playing, true));
        assert!(np.try_recv().is_err());

        // Player 1 cues: player 2 is promoted and player 1 reports stopped.
        let mut stopped = p.stopped().subscribe();
        p.handle_state(state(1, 10, PlayState::Cued, true));
        assert_eq!(np.try_recv().unwrap().device_id, 2);
        assert_eq!(stopped.try_recv().unwrap().device_id, 1);
    }

    #[tokio::test]
    async fn same_track_resume_does_not_re_emit() {
        let p = MixstatusProcessor::new(None);
        let mut np = p.now_playing().subscribe();
        p.handle_state(state(1, 10, PlayState::Playing, true));
        assert!(np.try_recv().is_ok());
        p.handle_state(state(1, 10, PlayState::Cued, true));
        p.handle_state(state(1, 10, PlayState::Playing, true));
        assert!(np.try_recv().is_err());
        p.handle_state(state(1, 11, PlayState::Cued, true));
        p.handle_state(state(1, 11, PlayState::Playing, true));
        assert_eq!(np.try_recv().unwrap().track_id, 11);
    }

    #[tokio::test]
    async fn follows_master_mode() {
        let p = MixstatusProcessor::new(Some(MixstatusConfig { mode: MixstatusMode::FollowsMaster, ..Default::default() }));
        let mut np = p.now_playing().subscribe();
        p.handle_state(state(1, 10, PlayState::Playing, true));
        assert!(np.try_recv().is_ok());
        p.handle_state(state(2, 20, PlayState::Cued, true));
        p.handle_state(state(2, 20, PlayState::Playing, true));
        assert!(np.try_recv().is_err());
        let mut master = state(2, 20, PlayState::Playing, true);
        master.is_master = true;
        p.handle_state(master);
        assert_eq!(np.try_recv().unwrap().device_id, 2);
    }

    #[tokio::test]
    async fn set_ends_after_silence() {
        let p = MixstatusProcessor::new(Some(MixstatusConfig { time_between_sets: 0.05, ..Default::default() }));
        let mut ended = p.set_ended().subscribe();
        p.handle_state(state(1, 10, PlayState::Playing, true));
        p.handle_state(state(1, 10, PlayState::Cued, true));
        let r = tokio::time::timeout(Duration::from_secs(1), ended.recv()).await;
        assert!(r.is_ok());
    }
}
