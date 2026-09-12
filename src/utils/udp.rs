//! UDP socket plumbing.
//!
//! A tokio [`UdpSocket`] has one reader, but several services listen on each
//! Pro DJ Link socket (the device manager and the announcer both watch the
//! announce socket, for example). A [`UdpFeed`] owns the socket, reads it on
//! a background task and re-broadcasts every datagram to any number of
//! subscribers, mirroring Node's `socket.on('message', …)`.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use crate::emitter::Emitter;
use crate::Result;

/// One received datagram. Cheap to clone: the payload is shared.
#[derive(Debug, Clone)]
pub struct Datagram {
    /// The UDP payload.
    pub data: Arc<[u8]>,
    /// Who sent it.
    pub from: SocketAddr,
}

impl Datagram {
    /// A datagram from a bare payload and source, for tests and for the
    /// passive capture path.
    pub fn new(data: impl Into<Arc<[u8]>>, from: SocketAddr) -> Self {
        Self { data: data.into(), from }
    }

    /// The source IPv4 address, `0.0.0.0` when the source was IPv6.
    pub fn src_ipv4(&self) -> Ipv4Addr {
        match self.from {
            SocketAddr::V4(a) => *a.ip(),
            SocketAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
        }
    }
}

/// A bound UDP socket whose incoming datagrams are broadcast to subscribers.
#[derive(Debug)]
pub struct UdpFeed {
    socket: Arc<UdpSocket>,
    packets: Emitter<Datagram>,
    reader: JoinHandle<()>,
}

impl UdpFeed {
    /// Bind `port` on all interfaces with `SO_REUSEADDR`, matching upstream's
    /// `dgram.createSocket({type: 'udp4', reuseAddr: true})`, and start
    /// reading.
    pub async fn bind(port: u16) -> Result<Self> {
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port));
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;
        #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
        socket.set_reuse_port(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&addr.into())?;
        let socket = UdpSocket::from_std(socket.into())?;
        Ok(Self::from_socket(socket))
    }

    /// Wrap an already-bound socket.
    pub fn from_socket(socket: UdpSocket) -> Self {
        let socket = Arc::new(socket);
        let packets: Emitter<Datagram> = Emitter::with_capacity(1024);
        let reader = {
            let socket = Arc::clone(&socket);
            let packets = packets.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65_535];
                loop {
                    match socket.recv_from(&mut buf).await {
                        Ok((n, from)) => {
                            packets.emit(Datagram { data: Arc::from(&buf[..n]), from });
                        }
                        Err(e) => {
                            tracing::debug!(target: "alphatheta_connect", "udp read failed: {e}");
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                    }
                }
            })
        };
        Self { socket, packets, reader }
    }

    /// Allow sending to the subnet broadcast address.
    pub fn set_broadcast(&self, on: bool) -> Result<()> {
        self.socket.set_broadcast(on)?;
        Ok(())
    }

    /// The socket, for sending.
    pub fn socket(&self) -> &Arc<UdpSocket> {
        &self.socket
    }

    /// Every datagram received from now on.
    pub fn packets(&self) -> &Emitter<Datagram> {
        &self.packets
    }

    /// Send `data` to `ip:port`.
    pub async fn send_to(&self, data: &[u8], ip: Ipv4Addr, port: u16) -> Result<usize> {
        Ok(self.socket.send_to(data, SocketAddr::V4(SocketAddrV4::new(ip, port))).await?)
    }

    /// The local address the socket is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    /// Stop reading. The socket closes when the last `Arc` to it is dropped.
    pub fn close(&self) {
        self.reader.abort();
    }
}

impl Drop for UdpFeed {
    fn drop(&mut self) {
        self.reader.abort();
    }
}
