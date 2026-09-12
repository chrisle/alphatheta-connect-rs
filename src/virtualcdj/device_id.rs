//! Virtual CDJ device-ID selection.

use std::collections::HashSet;

use crate::types::DeviceId;

/// The highest device ID a CDJ will answer remotedb queries from.
///
/// This limit applies to the device-ID byte inside the remotedb protocol (the
/// Introduce message and every query header), NOT to the ID we announce on
/// the network. Hardware-verified on a CDJ-3000 (2026-08-30, two players +
/// DJM-V5, announced as VCDJ 7 throughout): queries carrying 1-6 are all
/// answered — including IDs of absent players and the target's own ID — while
/// 7 and above are silently ignored (the TCP connection and Introduce succeed,
/// the query never gets a response). See [`pick_remote_db_query_id`].
pub const REMOTEDB_MAX_DEVICE_ID: DeviceId = 6;

/// The highest device ID the prolink protocol allows.
pub const MAX_DEVICE_ID: DeviceId = 32;

const MIXER_DEVICE_TYPE: u8 = 0x03;
const CDJ_DEVICE_TYPE: u8 = 0x01;

/// How many player numbers each mixer can hand out.
///
/// Players take their number from the mixer channel they are plugged into, so
/// an N-channel mixer only ever produces players 1..N — and N+1 is the lowest
/// number that no player on that rig can claim.
///
/// These are physical channel counts, deliberately NOT the `channelCount` in
/// the app's own capability table, which caps the DJM-V10 at 4 for its mixer
/// signal model. Capping here would put us on player 5 of a 6-channel rig,
/// which is the collision this table exists to avoid.
pub const MIXER_PLAYER_CEILING: &[(&str, u8)] = &[
    // 6-channel flagships — no player number in 1-6 is safe on these
    ("DJM-V10", 6),
    ("DJM-V10-LF", 6),
    // 4-channel
    ("DJM-A9", 4),
    ("DJM-900NXS2", 4),
    ("DJM-900NXS", 4),
    ("DJM-900SRT", 4),
    ("DJM-850", 4),
    ("DJM-800", 4),
    ("DJM-750MK2", 4),
    ("DJM-750", 4),
    ("DJM-TOUR1", 4),
    ("DJM-2000NXS", 4),
    ("DJM-2000", 4),
    ("EUPHONIA", 4),
    // 3-channel (2026)
    ("DJM-V5", 3),
    // 2-channel
    ("DJM-450", 2),
    ("DJM-350", 2),
    ("DJM-250MK2", 2),
    // All-in-ones: their built-in players occupy numbers too, so the ceiling
    // is however many decks the unit announces rather than its mixer channel
    // count
    ("XDJ-XZ", 4),
    ("XDJ-AZ", 4),
    ("XDJ-RX3", 2),
    ("XDJ-RX2", 2),
    ("XDJ-RX", 2),
    ("XDJ-AN", 2),
    ("OPUS-QUAD", 4),
];

/// What to assume about a mixer whose model we do not recognise.
///
/// Four is both the most common layout and the choice that keeps remotedb
/// metadata working: assuming six would push us to player 7 on every mixer
/// released after this table was written, silently losing metadata for
/// unanalyzed and streaming tracks. A six-channel unit we failed to recognise
/// may later claim the number we took, which the announcer's conflict
/// handling resolves by moving us.
pub const DEFAULT_MIXER_PLAYER_CEILING: u8 = 4;

/// The fields [`player_number_ceiling`] reads off a discovered device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLike {
    pub name: String,
    pub device_type: u8,
}

impl From<&crate::types::Device> for DeviceLike {
    fn from(d: &crate::types::Device) -> Self {
        Self { name: d.name.clone(), device_type: d.device_type.as_u8() }
    }
}

/// The highest player number the discovered gear can hand out, or `None`
/// when nothing on the network implies one.
///
/// `None` means we have no mixer to reason from — players may have been
/// numbered by hand anywhere in 1-6, and there is no better guess available.
pub fn player_number_ceiling<'a>(devices: impl IntoIterator<Item = &'a DeviceLike>) -> Option<u8> {
    let mut ceiling: Option<u8> = None;

    for device in devices {
        let key = device.name.trim().to_uppercase();
        let known = MIXER_PLAYER_CEILING.iter().find(|(n, _)| *n == key).map(|(_, c)| *c);
        let value = known.or(if device.device_type == MIXER_DEVICE_TYPE { Some(DEFAULT_MIXER_PLAYER_CEILING) } else { None });

        if let Some(v) = value {
            if ceiling.is_none_or(|c| v > c) {
                ceiling = Some(v);
            }
        }
    }

    ceiling
}

/// Options for [`pick_available_device_id`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PickDeviceIdOptions {
    /// Prefer an ID in the 1-6 range, the range real players number
    /// themselves in. Purely conventional: remotedb metadata no longer
    /// depends on our announced ID ([`pick_remote_db_query_id`] chooses the
    /// in-protocol query ID independently), but sitting where gear expects
    /// players keeps us legible on link displays and avoids surprising other
    /// tooling.
    pub prefer_player_range: bool,
    /// The highest player number the rig can hand out, from
    /// [`player_number_ceiling`]. The search starts just above it, so the
    /// number we take is one no player on this rig can be assigned.
    ///
    /// When omitted, the 1-6 range is searched downwards instead: players
    /// number themselves from 1 upwards, so the top of the range is the least
    /// likely to be claimed by a CDJ that powers on later.
    pub player_ceiling: Option<u8>,
}

fn descending(from: u8, to: u8) -> Vec<DeviceId> {
    (to..=from).rev().collect()
}

fn ascending(from: u8, to: u8) -> Vec<DeviceId> {
    (from..=to).collect()
}

/// Pick a device ID that no device on the network is currently using.
///
/// Taking an ID a live player already holds knocks that player off the
/// network mid-set, so every caller that chooses its own ID should route
/// through here rather than hardcoding one.
///
/// Returns `None` when every ID from 1 to 32 is occupied, in which case there
/// is no safe ID and the caller must not join the network.
pub fn pick_available_device_id(used_ids: impl IntoIterator<Item = DeviceId>, options: PickDeviceIdOptions) -> Option<DeviceId> {
    let used: HashSet<DeviceId> = used_ids.into_iter().collect();

    let above_the_players = match options.player_ceiling {
        None => descending(REMOTEDB_MAX_DEVICE_ID, 1),
        Some(ceiling) => ascending(ceiling + 1, REMOTEDB_MAX_DEVICE_ID),
    };

    let outside_player_range = ascending(REMOTEDB_MAX_DEVICE_ID + 1, MAX_DEVICE_ID);

    // Numbers a player on this rig could be assigned. Last resort: better to
    // contend for a slot than not to join at all, and the announcer moves us
    // if the player that owns it shows up.
    let among_the_players = match options.player_ceiling {
        None => Vec::new(),
        Some(ceiling) => descending(ceiling.min(REMOTEDB_MAX_DEVICE_ID), 1),
    };

    let search: Vec<DeviceId> = if options.prefer_player_range {
        [above_the_players, outside_player_range, among_the_players].concat()
    } else {
        [outside_player_range, above_the_players, among_the_players].concat()
    };

    search.into_iter().find(|id| !used.contains(id))
}

/// The fields [`pick_remote_db_query_id`] reads off a discovered device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryIdDeviceLike {
    pub id: DeviceId,
    pub device_type: u8,
}

impl From<&crate::types::Device> for QueryIdDeviceLike {
    fn from(d: &crate::types::Device) -> Self {
        Self { id: d.id, device_type: d.device_type.as_u8() }
    }
}

/// Choose the device ID to carry inside remotedb messages when querying a CDJ.
///
/// CDJs only answer remotedb queries whose in-protocol device-ID byte is in
/// 1-6, but that byte is independent of the ID we announce on the network —
/// so a virtual CDJ announced safely above the player range (say 7 behind a
/// DJM-V10) can still query metadata by carrying an in-range ID here.
/// Verified on a CDJ-3000 (2026-08-30): absent IDs and even the target's own
/// ID are answered; 7+ is silently dropped.
///
/// Older players are documented (dysentery, nexus era) as stricter: the byte
/// had to be 1-4, belong to a player actually on the network, and not be the
/// player being queried. The preference order below satisfies those rules
/// whenever the rig makes it possible, then falls back through choices newer
/// players accept.
pub fn pick_remote_db_query_id(target_id: DeviceId, devices: impl IntoIterator<Item = QueryIdDeviceLike>) -> DeviceId {
    let mut player_ids: HashSet<DeviceId> = HashSet::new();
    for device in devices {
        if device.device_type == CDJ_DEVICE_TYPE && device.id <= REMOTEDB_MAX_DEVICE_ID {
            player_ids.insert(device.id);
        }
    }

    // A live player that isn't the target, lowest first — 1-4 satisfies every
    // documented era of the restriction.
    let mut other_players: Vec<DeviceId> = player_ids.iter().copied().filter(|id| *id != target_id).collect();
    other_players.sort_unstable();
    if let Some(first) = other_players.first() {
        return *first;
    }

    // No other player on the rig: take the lowest 1-6 ID no player holds.
    for id in 1..=REMOTEDB_MAX_DEVICE_ID {
        if id != target_id && !player_ids.contains(&id) {
            return id;
        }
    }

    // Six players and every one of them is the target? Impossible, but the
    // target's own ID is answered too, so it is a safe closing fallback.
    target_id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str, t: u8) -> DeviceLike {
        DeviceLike { name: name.into(), device_type: t }
    }

    #[test]
    fn ceiling_from_known_mixers() {
        assert_eq!(player_number_ceiling(&[dev("DJM-V10", 3)]), Some(6));
        assert_eq!(player_number_ceiling(&[dev("djm-a9 ", 3)]), Some(4));
        assert_eq!(player_number_ceiling(&[dev("Mystery", 3)]), Some(DEFAULT_MIXER_PLAYER_CEILING));
        assert_eq!(player_number_ceiling(&[dev("CDJ-3000", 1)]), None);
        assert_eq!(player_number_ceiling(&[dev("XDJ-RX3", 1), dev("DJM-V5", 3)]), Some(3));
    }

    #[test]
    fn picks_outside_player_range_by_default() {
        assert_eq!(pick_available_device_id([1, 2, 3], PickDeviceIdOptions::default()), Some(7));
        assert_eq!(pick_available_device_id([7, 8], PickDeviceIdOptions::default()), Some(9));
    }

    #[test]
    fn prefers_player_range_above_the_ceiling() {
        let opts = PickDeviceIdOptions { prefer_player_range: true, player_ceiling: Some(4) };
        assert_eq!(pick_available_device_id([1, 2], opts), Some(5));
        assert_eq!(pick_available_device_id([1, 2, 5, 6], opts), Some(7));
        let opts6 = PickDeviceIdOptions { prefer_player_range: true, player_ceiling: Some(6) };
        assert_eq!(pick_available_device_id([1], opts6), Some(7));
        let none = PickDeviceIdOptions { prefer_player_range: true, player_ceiling: None };
        assert_eq!(pick_available_device_id([1, 6], none), Some(5));
    }

    #[test]
    fn falls_back_among_players_and_to_none() {
        let opts = PickDeviceIdOptions { prefer_player_range: true, player_ceiling: Some(4) };
        let used: Vec<u8> = (5..=32).collect();
        assert_eq!(pick_available_device_id(used, opts), Some(4));
        let all: Vec<u8> = (1..=32).collect();
        assert_eq!(pick_available_device_id(all, opts), None);
    }

    #[test]
    fn query_id_prefers_other_live_players() {
        let devs = [
            QueryIdDeviceLike { id: 2, device_type: 1 },
            QueryIdDeviceLike { id: 3, device_type: 1 },
            QueryIdDeviceLike { id: 33, device_type: 3 },
        ];
        assert_eq!(pick_remote_db_query_id(2, devs), 3);
        assert_eq!(pick_remote_db_query_id(3, devs), 2);
        assert_eq!(pick_remote_db_query_id(2, [QueryIdDeviceLike { id: 2, device_type: 1 }]), 1);
        assert_eq!(pick_remote_db_query_id(1, [QueryIdDeviceLike { id: 1, device_type: 1 }]), 2);
        assert_eq!(pick_remote_db_query_id(7, []), 1);
    }
}
