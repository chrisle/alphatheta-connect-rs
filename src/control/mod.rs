//! Remote control of CDJs on the network.

pub mod stagehand;

use std::sync::Arc;

use rand::RngExt;

use crate::constants::{BEAT_PORT, PROLINK_HEADER, STATUS_PORT};
use crate::status::types::PlayState;
use crate::types::{device_snapshot, Device, DeviceType, SharedDevice};
use crate::utils::build_name;
use crate::utils::udp::UdpFeed;
use crate::{Error, Result};

pub use stagehand::{make_stagehand_pref_write_packet, make_stagehand_transport_packet, OnAirPref, StagehandPreferences};

/// Generates the packet used to control the playstate of CDJs in active
/// connection mode. `play_state` must be [`PlayState::Cued`] or
/// [`PlayState::Playing`].
pub fn make_playstate_packet(host_device: &Device, device: &Device, play_state: PlayState) -> Result<Vec<u8>> {
    let state_byte = match play_state {
        PlayState::Cued => 0x01,
        PlayState::Playing => 0x00,
        other => return Err(Error::State(format!("cannot set play state {other:?}; only Cued and Playing are supported"))),
    };

    let mut p = Vec::with_capacity(0x2c);
    p.extend_from_slice(&PROLINK_HEADER);
    p.push(0x02);
    p.extend_from_slice(&build_name(host_device));
    p.extend_from_slice(&[0x01, 0x00]);
    p.push(host_device.id);
    p.extend_from_slice(&[0x00, 0x04]);
    // One byte per player slot 0-3; upstream indexes by the device id.
    for i in 0..4u8 {
        p.push(if i == device.id { state_byte } else { 0 });
    }
    Ok(p)
}

/// The control service can be used to control the play state of CDJs on the
/// network.
#[derive(Clone)]
pub struct Control {
    host_device: SharedDevice,
    /// The socket used to send control packets.
    beat_feed: Arc<UdpFeed>,
    /// Randomized correlation byte per session for Stagehand commands.
    correlation_byte: u8,
}

impl Control {
    pub fn new(beat_feed: Arc<UdpFeed>, host_device: SharedDevice) -> Self {
        Self { beat_feed, host_device, correlation_byte: rand::rng().random() }
    }

    /// Start or stop a CDJ on the network. Delegates automatically to
    /// Stagehand-specific transport control if connected in Stagehand mode.
    pub async fn set_play_state(&self, device: &Device, play_state: PlayState) -> Result<()> {
        let host = device_snapshot(&self.host_device);
        if host.device_type == DeviceType::Stagehand {
            return if play_state == PlayState::Playing { self.play(device).await } else { self.pause(device).await };
        }

        let packet = make_playstate_packet(&host, device, play_state)?;
        self.beat_feed.send_to(&packet, device.ip, BEAT_PORT).await?;
        Ok(())
    }

    async fn transport(&self, device: &Device, op: u8, press: bool) -> Result<()> {
        let host = device_snapshot(&self.host_device);
        let p = make_stagehand_transport_packet(&host, op, press, self.correlation_byte);
        self.beat_feed.send_to(&p, device.ip, BEAT_PORT).await?;
        Ok(())
    }

    /// Send Stagehand PLAY command (paired 0x0f and 0x14 packets).
    pub async fn play(&self, device: &Device) -> Result<()> {
        self.transport(device, 0x0f, true).await?;
        self.transport(device, 0x14, true).await
    }

    /// Send Stagehand PAUSE command (paired 0x14 packet).
    pub async fn pause(&self, device: &Device) -> Result<()> {
        self.transport(device, 0x14, false).await
    }

    /// Send Stagehand SEEK forward command. `press`: true to start seek,
    /// false to release.
    pub async fn seek_forward(&self, device: &Device, press: bool) -> Result<()> {
        self.transport(device, 0x1a, press).await
    }

    /// Send Stagehand SEEK backward command.
    pub async fn seek_backward(&self, device: &Device, press: bool) -> Result<()> {
        self.transport(device, 0x1b, press).await
    }

    /// Send Stagehand SKIP track command.
    pub async fn skip(&self, device: &Device, press: bool) -> Result<()> {
        self.transport(device, 0x18, press).await
    }

    /// Send Stagehand preference write command (0x6b packet) to port 50002.
    pub async fn set_preference(&self, device: &Device, options: StagehandPreferences) -> Result<()> {
        let host = device_snapshot(&self.host_device);
        let p = make_stagehand_pref_write_packet(&host, options);
        self.beat_feed.send_to(&p, device.ip, STATUS_PORT).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn playstate_packet() {
        let host = Device::new("ProLink-Connect", 5, DeviceType::Cdj, [0; 6], Ipv4Addr::new(10, 0, 0, 1));
        let target = Device::new("CDJ", 2, DeviceType::Cdj, [0; 6], Ipv4Addr::new(10, 0, 0, 2));
        let p = make_playstate_packet(&host, &target, PlayState::Cued).unwrap();
        assert_eq!(p.len(), 40);
        assert_eq!(p[0x0a], 0x02);
        assert_eq!(p[33], 5);
        assert_eq!(&p[36..40], &[0, 0, 1, 0]);
        let play = make_playstate_packet(&host, &target, PlayState::Playing).unwrap();
        assert_eq!(&play[36..40], &[0, 0, 0, 0]);
        assert!(make_playstate_packet(&host, &target, PlayState::Paused).is_err());
    }
}
