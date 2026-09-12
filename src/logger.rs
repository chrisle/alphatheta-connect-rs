//! Pluggable logging.
//!
//! Consumers can supply their own [`Logger`] implementation. If none is
//! provided a no-op logger is used (silent). [`TracingLogger`] forwards to the
//! `tracing` crate for applications that already use it.

use std::sync::Arc;

/// Logger interface for the crate. Consumers can supply their own
/// implementation.
pub trait Logger: Send + Sync {
    fn trace(&self, msg: &str);
    fn debug(&self, msg: &str);
    fn info(&self, msg: &str);
    fn warn(&self, msg: &str);
    fn error(&self, msg: &str);
}

/// A logger handle that can be cloned into every service.
pub type SharedLogger = Arc<dyn Logger>;

/// Discards everything.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopLogger;

impl Logger for NoopLogger {
    fn trace(&self, _msg: &str) {}
    fn debug(&self, _msg: &str) {}
    fn info(&self, _msg: &str) {}
    fn warn(&self, _msg: &str) {}
    fn error(&self, _msg: &str) {}
}

/// Forwards to the `tracing` crate under the `alphatheta_connect` target.
#[derive(Debug, Default, Clone, Copy)]
pub struct TracingLogger;

impl Logger for TracingLogger {
    fn trace(&self, msg: &str) {
        tracing::trace!(target: "alphatheta_connect", "{msg}");
    }
    fn debug(&self, msg: &str) {
        tracing::debug!(target: "alphatheta_connect", "{msg}");
    }
    fn info(&self, msg: &str) {
        tracing::info!(target: "alphatheta_connect", "{msg}");
    }
    fn warn(&self, msg: &str) {
        tracing::warn!(target: "alphatheta_connect", "{msg}");
    }
    fn error(&self, msg: &str) {
        tracing::error!(target: "alphatheta_connect", "{msg}");
    }
}

/// The logger used when a consumer supplies none.
pub fn noop_logger() -> SharedLogger {
    Arc::new(NoopLogger)
}
