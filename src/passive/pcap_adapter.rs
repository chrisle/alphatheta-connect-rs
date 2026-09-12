//! Packet capture of the Pro DJ Link ports.
//!
//! [`PcapAdapter`] captures Pro DJ Link packets via libpcap, extracts UDP
//! payloads and emits them to subscribers based on destination port. This
//! allows passive monitoring of the network without binding to UDP ports or
//! announcing a virtual CDJ.
//!
//! Capture needs the `passive` cargo feature (which links libpcap) and
//! root/sudo privileges. Without the feature, [`PcapAdapter::start`] returns
//! [`Error::Unsupported`]; the adapter can still be fed packets by hand
//! through [`PcapAdapter::inject`], which is how the rest of the passive
//! stack is tested.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::constants::{ANNOUNCE_PORT, BEAT_PORT, STATUS_PORT};
use crate::emitter::Emitter;
use crate::utils::udp::Datagram;
use crate::{Error, Result};

/// Packet info including the source IP (useful for short-format packets
/// where the IP may not be fully present in the payload). This is the
/// [`Datagram`] type shared with the socket path.
pub type PacketInfo = Datagram;

/// Configuration for [`PcapAdapter`].
#[derive(Debug, Clone)]
pub struct PcapAdapterConfig {
    /// Network interface name (e.g., 'en0', 'eth0', 'en15').
    pub iface: String,
    /// Buffer size for packet capture in bytes. Default: 10 MB.
    pub buffer_size: Option<usize>,
}

struct Inner {
    iface: String,
    #[allow(dead_code)]
    buffer_size: usize,
    started: AtomicBool,
    /// Fired when a device announcement packet is received (port 50000).
    announce: Emitter<Datagram>,
    /// Fired when a status packet is received (port 50002).
    status: Emitter<Datagram>,
    /// Fired when a beat/position packet is received (port 50001).
    beat: Emitter<Datagram>,
    /// Fired when an error occurs.
    error: Emitter<String>,
    capture: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// Captures Pro DJ Link packets and re-emits their payloads by port.
#[derive(Clone)]
pub struct PcapAdapter {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PcapAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PcapAdapter").field("iface", &self.inner.iface).field("capturing", &self.is_capturing()).finish()
    }
}

impl PcapAdapter {
    pub fn new(config: PcapAdapterConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                iface: config.iface,
                buffer_size: config.buffer_size.unwrap_or(10 * 1024 * 1024),
                started: AtomicBool::new(false),
                announce: Emitter::with_capacity(1024),
                status: Emitter::with_capacity(1024),
                beat: Emitter::with_capacity(1024),
                error: Emitter::new(),
                capture: Mutex::new(None),
            }),
        }
    }

    pub fn announce(&self) -> &Emitter<Datagram> {
        &self.inner.announce
    }

    pub fn status(&self) -> &Emitter<Datagram> {
        &self.inner.status
    }

    pub fn beat(&self) -> &Emitter<Datagram> {
        &self.inner.beat
    }

    pub fn error(&self) -> &Emitter<String> {
        &self.inner.error
    }

    /// Feed a captured UDP payload to the right emitter by destination port.
    /// Also the entry point for tests and for other capture backends.
    pub fn inject(&self, dst_port: u16, payload: Vec<u8>, src_addr: Ipv4Addr) {
        let datagram = Datagram::new(payload, SocketAddr::V4(SocketAddrV4::new(src_addr, dst_port)));
        match dst_port {
            ANNOUNCE_PORT => self.inner.announce.emit(datagram),
            BEAT_PORT => self.inner.beat.emit(datagram),
            STATUS_PORT => self.inner.status.emit(datagram),
            _ => {}
        }
    }

    /// Decode an Ethernet frame and feed its UDP payload to
    /// [`inject`](Self::inject). Returns false for frames that are not IPv4
    /// UDP.
    pub fn inject_frame(&self, frame: &[u8]) -> bool {
        let Some((dst_port, src, payload)) = decode_udp_frame(frame) else {
            return false;
        };
        self.inject(dst_port, payload.to_vec(), src);
        true
    }

    /// Start capturing packets on the configured interface. Requires
    /// root/sudo privileges and the `passive` cargo feature.
    pub fn start(&self) -> Result<()> {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        match self.start_capture() {
            Ok(()) => Ok(()),
            Err(e) => {
                self.inner.started.store(false, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    #[cfg(feature = "passive")]
    fn start_capture(&self) -> Result<()> {
        use pcap::{Capture, Device};

        let iface = self.inner.iface.clone();

        // On Windows, translate the interface name to an Npcap device path by
        // matching IPv4 addresses.
        let device_name = if cfg!(windows) && !iface.starts_with("\\Device\\") {
            resolve_windows_interface(&iface).unwrap_or_else(|| iface.clone())
        } else {
            iface.clone()
        };

        let device = Device::list()
            .map_err(|e| Error::Unsupported(format!("Failed to list capture devices: {e}")))?
            .into_iter()
            .find(|d| d.name == device_name)
            .unwrap_or_else(|| Device::from(device_name.as_str()));

        let mut cap = Capture::from_device(device)
            .and_then(|c| c.promisc(true).snaplen(65535).buffer_size(self.inner.buffer_size as i32).timeout(100).open())
            .map_err(|e| {
                Error::Unsupported(format!(
                    "Failed to open interface \"{iface}\" (device: {device_name}) for packet capture. Ensure you have root/sudo privileges and the interface exists.\nOriginal error: {e}"
                ))
            })?;

        // BPF filter for Pro DJ Link UDP ports
        let filter = format!("udp and (port {ANNOUNCE_PORT} or port {BEAT_PORT} or port {STATUS_PORT})");
        cap.filter(&filter, true).map_err(|e| Error::Unsupported(format!("Failed to set capture filter: {e}")))?;

        let adapter = self.clone();
        let handle = std::thread::Builder::new()
            .name("alphatheta-pcap".into())
            .spawn(move || {
                while adapter.inner.started.load(Ordering::SeqCst) {
                    match cap.next_packet() {
                        Ok(packet) => {
                            adapter.inject_frame(packet.data);
                        }
                        Err(pcap::Error::TimeoutExpired) => continue,
                        Err(e) => {
                            adapter.inner.error.emit(e.to_string());
                            break;
                        }
                    }
                }
            })
            .map_err(|e| Error::Other(format!("could not start capture thread: {e}")))?;

        *self.inner.capture.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
        Ok(())
    }

    #[cfg(not(feature = "passive"))]
    fn start_capture(&self) -> Result<()> {
        Err(Error::Unsupported(
            "Packet capture requires the `passive` cargo feature (which links libpcap; needs libpcap-dev on Linux or Npcap on Windows).".into(),
        ))
    }

    /// Stop capturing packets and release resources.
    pub fn stop(&self) {
        self.inner.started.store(false, Ordering::SeqCst);
        if let Some(handle) = self.inner.capture.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = handle.join();
        }
    }

    /// Check if the adapter is currently capturing.
    pub fn is_capturing(&self) -> bool {
        self.inner.started.load(Ordering::SeqCst)
    }

    /// Get the interface name being captured.
    pub fn interface_name(&self) -> &str {
        &self.inner.iface
    }
}

/// Decode an Ethernet II frame carrying IPv4/UDP: (destination port, source
/// address, payload).
pub fn decode_udp_frame(frame: &[u8]) -> Option<(u16, Ipv4Addr, &[u8])> {
    // Ethernet header: 6 dst, 6 src, 2 ethertype (0x0800 = IPv4). Handle one
    // 802.1Q tag as well.
    if frame.len() < 14 {
        return None;
    }
    let mut ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    let mut ip_start = 14;
    if ethertype == 0x8100 {
        if frame.len() < 18 {
            return None;
        }
        ethertype = u16::from_be_bytes([frame[16], frame[17]]);
        ip_start = 18;
    }
    if ethertype != 0x0800 {
        return None;
    }

    let ip = &frame[ip_start..];
    if ip.len() < 20 || ip[0] >> 4 != 4 {
        return None;
    }
    let ihl = usize::from(ip[0] & 0x0f) * 4;
    if ihl < 20 || ip.len() < ihl {
        return None;
    }
    let total_len = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
    // UDP
    if ip[9] != 17 {
        return None;
    }
    let src = Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]);

    let udp = &ip[ihl..ip.len().min(total_len.max(ihl))];
    if udp.len() < 8 {
        return None;
    }
    let dst_port = u16::from_be_bytes([udp[2], udp[3]]);
    let udp_len = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if udp_len < 8 {
        return None;
    }
    let payload_len = (udp_len - 8).min(udp.len() - 8);
    if payload_len == 0 {
        return None;
    }
    Some((dst_port, src, &udp[8..8 + payload_len]))
}

/// Resolve an interface name (e.g. "Ethernet") to an Npcap device path (e.g.
/// `\Device\NPF_{GUID}`) on Windows, by matching IPv4 addresses.
#[cfg(feature = "passive")]
fn resolve_windows_interface(iface_name: &str) -> Option<String> {
    let addresses: Vec<std::net::IpAddr> =
        crate::utils::net::interfaces_by_name(iface_name).into_iter().map(|i| std::net::IpAddr::V4(i.address)).collect();
    if addresses.is_empty() {
        return None;
    }
    pcap::Device::list().ok()?.into_iter().find(|d| d.addresses.iter().any(|a| addresses.contains(&a.addr))).map(|d| d.name)
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// Build an Ethernet/IPv4/UDP frame.
    pub fn udp_frame(src: [u8; 4], dst_port: u16, payload: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&[0xff; 6]);
        f.extend_from_slice(&[0xc8, 0x3d, 0xfc, 1, 2, 3]);
        f.extend_from_slice(&[0x08, 0x00]);
        let udp_len = 8 + payload.len();
        let total = 20 + udp_len;
        let mut ip = vec![0x45, 0, (total >> 8) as u8, (total & 0xff) as u8, 0, 0, 0x40, 0, 64, 17, 0, 0];
        ip.extend_from_slice(&src);
        ip.extend_from_slice(&[255, 255, 255, 255]);
        f.extend_from_slice(&ip);
        f.extend_from_slice(&50000u16.to_be_bytes());
        f.extend_from_slice(&dst_port.to_be_bytes());
        f.extend_from_slice(&(udp_len as u16).to_be_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(payload);
        f
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::udp_frame;
    use super::*;

    #[test]
    fn decodes_udp_frames() {
        let frame = udp_frame([169, 254, 88, 83], STATUS_PORT, b"hello");
        let (port, src, payload) = decode_udp_frame(&frame).unwrap();
        assert_eq!(port, STATUS_PORT);
        assert_eq!(src, Ipv4Addr::new(169, 254, 88, 83));
        assert_eq!(payload, b"hello");
        assert!(decode_udp_frame(&frame[..20]).is_none());
        let mut not_udp = frame.clone();
        not_udp[23] = 6; // tcp
        assert!(decode_udp_frame(&not_udp).is_none());
    }

    #[tokio::test]
    async fn routes_by_destination_port() {
        let adapter = PcapAdapter::new(PcapAdapterConfig { iface: "test0".into(), buffer_size: None });
        let mut announce = adapter.announce().subscribe();
        let mut status = adapter.status().subscribe();
        let mut beat = adapter.beat().subscribe();
        assert!(adapter.inject_frame(&udp_frame([10, 0, 0, 1], ANNOUNCE_PORT, b"a")));
        assert!(adapter.inject_frame(&udp_frame([10, 0, 0, 1], STATUS_PORT, b"s")));
        assert!(adapter.inject_frame(&udp_frame([10, 0, 0, 1], BEAT_PORT, b"b")));
        assert!(adapter.inject_frame(&udp_frame([10, 0, 0, 1], 9999, b"x")));
        assert_eq!(&*announce.try_recv().unwrap().data, b"a");
        assert_eq!(&*status.try_recv().unwrap().data, b"s");
        let b = beat.try_recv().unwrap();
        assert_eq!(&*b.data, b"b");
        assert_eq!(b.src_ipv4(), Ipv4Addr::new(10, 0, 0, 1));
        assert!(!adapter.is_capturing());
        assert_eq!(adapter.interface_name(), "test0");
    }

    #[cfg(not(feature = "passive"))]
    #[test]
    fn start_without_feature_is_unsupported() {
        let adapter = PcapAdapter::new(PcapAdapterConfig { iface: "test0".into(), buffer_size: None });
        assert!(matches!(adapter.start(), Err(Error::Unsupported(_))));
        assert!(!adapter.is_capturing());
    }
}
