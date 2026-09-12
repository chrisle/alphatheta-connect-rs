//! A minimal ONC-RPC client over UDP.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use crate::nfs::xdr::rpc;
use crate::{Error, Result};

/// The RPC auth stamp passed by the CDJs. It's unclear if this is actually
/// important, but the rpc calls stay as close to CDJ calls as they can.
const CDJ_AUTH_STAMP: u32 = 0x967b8703;

fn rpc_auth_message() -> rpc::UnixAuth {
    rpc::UnixAuth { stamp: CDJ_AUTH_STAMP, name: String::new(), uid: 0, gid: 0, gids: Vec::new() }
}

/// One RPC call.
#[derive(Debug, Clone)]
pub struct RpcCall {
    pub port: u16,
    pub program: u32,
    pub version: u32,
    pub procedure: u32,
    pub data: Vec<u8>,
}

/// Configuration for the retry strategy to use when making RPC calls.
///
/// Mirrors the `promise-retry` / `retry` options upstream uses, with the same
/// defaults: 10 retries, exponential factor 2 from 1 s, uncapped.
#[derive(Debug, Clone, PartialEq)]
pub struct RetryConfig {
    /// The maximum amount of times to retry the operation. Default: 10.
    pub retries: u32,
    /// The exponential factor to use. Default: 2.
    pub factor: f64,
    /// The time to wait before the first retry. Default: 1 s.
    pub min_timeout: Duration,
    /// The maximum time to wait between retries. Default: unbounded.
    pub max_timeout: Duration,
    /// Randomize the timeouts by multiplying with a factor between 1 and 2.
    /// Default: false.
    pub randomize: bool,
    /// Time to wait before a RPC transaction should time out. Default: 1 s.
    pub transaction_timeout: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            retries: 10,
            factor: 2.0,
            min_timeout: Duration::from_millis(1000),
            max_timeout: Duration::MAX,
            randomize: false,
            transaction_timeout: Duration::from_millis(1000),
        }
    }
}

impl RetryConfig {
    /// The wait before retry number `attempt` (1-based), as `retry` computes it.
    fn backoff(&self, attempt: u32) -> Duration {
        let random = if self.randomize { 1.0 + rand::random::<f64>() } else { 1.0 };
        let ms = random * self.min_timeout.as_secs_f64() * 1000.0 * self.factor.powi(attempt.saturating_sub(1) as i32);
        let ms = ms.min(self.max_timeout.as_secs_f64() * 1000.0);
        Duration::from_secs_f64((ms / 1000.0).max(0.0))
    }
}

/// Generic RPC connection. Can be used to make RPC 2 calls to any program
/// specified in the [`RpcCall`].
pub struct RpcConnection {
    pub address: Ipv4Addr,
    retry_config: RwLock<RetryConfig>,
    socket: UdpSocket,
    mutex: Mutex<()>,
    xid: AtomicU32,
    connected: AtomicBool,
}

impl std::fmt::Debug for RpcConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcConnection").field("address", &self.address).finish()
    }
}

impl RpcConnection {
    pub async fn new(address: Ipv4Addr, retry_config: Option<RetryConfig>) -> Result<Self> {
        let socket = UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))).await?;
        Ok(Self {
            address,
            retry_config: RwLock::new(retry_config.unwrap_or_default()),
            socket,
            mutex: Mutex::new(()),
            xid: AtomicU32::new(1),
            connected: AtomicBool::new(true),
        })
    }

    /// Whether the connection is believed to be usable.
    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub fn retry_config(&self) -> RetryConfig {
        self.retry_config.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_retry_config(&self, config: RetryConfig) {
        *self.retry_config.write().unwrap_or_else(|e| e.into_inner()) = config;
    }

    fn setup_request(&self, xid: u32, call: &RpcCall) -> Vec<u8> {
        let auth = rpc::Auth { flavor: 1, body: rpc_auth_message().to_xdr() };
        let verifier = rpc::Auth { flavor: 0, body: Vec::new() };

        let request = rpc::Request {
            rpc_version: rpc::VERSION,
            program: call.program,
            program_version: call.version,
            procedure: call.procedure,
            auth,
            verifier,
            data: call.data.clone(),
        };

        rpc::encode_request(xid, &request)
    }

    /// Execute a RPC transaction (call and response).
    ///
    /// If a transaction does not complete after the configured timeout it
    /// will be retried with the retry configuration.
    pub async fn call(&self, call: RpcCall) -> Result<Vec<u8>> {
        let xid = self.xid.fetch_add(1, Ordering::SeqCst) + 1;
        let call_data = self.setup_request(xid, &call);
        let target = SocketAddr::V4(SocketAddrV4::new(self.address, call.port));
        let config = self.retry_config();

        // Execute the transaction exclusively to avoid async call races
        let _guard = self.mutex.lock().await;

        let mut attempt = 0u32;
        let reply = loop {
            let execute = async {
                self.socket.send_to(&call_data, target).await?;
                let mut buf = vec![0u8; 65_535];
                loop {
                    let (n, _) = self.socket.recv_from(&mut buf).await?;
                    let reply = rpc::decode_reply(&buf[..n])?;
                    // A late answer to an earlier (timed out) attempt is not ours.
                    if reply.xid == xid {
                        return Ok::<_, Error>(reply);
                    }
                }
            };

            match tokio::time::timeout(config.transaction_timeout, execute).await {
                Ok(result) => break result?,
                Err(_) if attempt < config.retries => {
                    attempt += 1;
                    tokio::time::sleep(config.backoff(attempt)).await;
                }
                Err(_) => {
                    return Err(Error::Timeout(format!(
                        "RPC call to {target} (program {} procedure {}) timed out after {} attempts",
                        call.program,
                        call.procedure,
                        attempt + 1
                    )))
                }
            }
        };

        match reply.body {
            rpc::ReplyBody::Success(data) => Ok(data),
            rpc::ReplyBody::Denied => Err(Error::Nfs("RPC request was denied".into())),
            rpc::ReplyBody::Failed { status, mismatch } => Err(Error::Nfs(format!(
                "RPC did not successfully return data (accept status {status}{})",
                mismatch.map(|(lo, hi)| format!(", versions {lo}-{hi} supported")).unwrap_or_default()
            ))),
        }
    }

    pub fn disconnect(&self) {
        self.connected.store(false, Ordering::SeqCst);
    }
}

/// [`RpcProgram`] is constructed with specialization details for a specific
/// RPC program. This should be used to avoid having to repeat yourself for
/// calls made using the [`RpcConnection`].
#[derive(Debug, Clone)]
pub struct RpcProgram {
    pub program: u32,
    pub version: u32,
    pub port: u16,
    pub conn: Arc<RpcConnection>,
}

impl RpcProgram {
    pub fn new(conn: Arc<RpcConnection>, program: u32, version: u32, port: u16) -> Self {
        Self { conn, program, version, port }
    }

    pub async fn call(&self, procedure: u32, data: Vec<u8>) -> Result<Vec<u8>> {
        self.conn.call(RpcCall { program: self.program, version: self.version, port: self.port, procedure, data }).await
    }

    pub fn disconnect(&self) {
        self.conn.disconnect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_exponential_from_min_timeout() {
        let c = RetryConfig::default();
        assert_eq!(c.backoff(1), Duration::from_millis(1000));
        assert_eq!(c.backoff(2), Duration::from_millis(2000));
        assert_eq!(c.backoff(3), Duration::from_millis(4000));
        let capped = RetryConfig { max_timeout: Duration::from_millis(1500), ..Default::default() };
        assert_eq!(capped.backoff(3), Duration::from_millis(1500));
    }

    #[tokio::test]
    async fn calls_time_out_and_retry() {
        let conn = RpcConnection::new(
            Ipv4Addr::LOCALHOST,
            Some(RetryConfig {
                retries: 1,
                min_timeout: Duration::from_millis(1),
                transaction_timeout: Duration::from_millis(20),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        // Nothing listens on this port, so both attempts time out.
        let err = conn.call(RpcCall { port: 1, program: 1, version: 1, procedure: 1, data: vec![] }).await.unwrap_err();
        assert!(matches!(err, Error::Timeout(_)), "{err:?}");
    }

    #[tokio::test]
    async fn round_trips_against_a_fake_server() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1024];
            let (n, from) = server.recv_from(&mut buf).await.unwrap();
            let xid = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
            let mut w = crate::nfs::xdr::XdrWriter::new();
            w.u32(xid).u32(1).u32(0).u32(0).opaque_var(&[]).u32(0).u32(0x2049);
            server.send_to(&w.into_bytes(), from).await.unwrap();
            let _ = n;
        });

        let conn = RpcConnection::new(Ipv4Addr::LOCALHOST, None).await.unwrap();
        let data = conn.call(RpcCall { port, program: 100_000, version: 2, procedure: 3, data: vec![] }).await.unwrap();
        assert_eq!(data, vec![0, 0, 0x20, 0x49]);
    }
}
