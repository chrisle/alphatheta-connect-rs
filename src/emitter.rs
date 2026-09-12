//! A small typed event emitter.
//!
//! Upstream uses Node's `EventEmitter` with `on` / `off` / `once` per event
//! name. Each event here is its own [`Emitter<T>`] backed by a `tokio`
//! broadcast channel: consumers either [`subscribe`](Emitter::subscribe) and
//! receive events on a channel, or register a callback with
//! [`on`](Emitter::on), which runs it on a background task until the returned
//! [`Listener`] is [turned off](Listener::off).

use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Events queued per subscriber before the oldest are dropped. Status packets
/// arrive at ~5 Hz per player, so this is many seconds of slack.
const DEFAULT_CAPACITY: usize = 256;

/// A typed event source. Cloning an emitter yields another handle on the same
/// channel: events emitted through either reach every subscriber.
#[derive(Debug, Clone)]
pub struct Emitter<T> {
    tx: broadcast::Sender<T>,
}

impl<T: Clone + Send + 'static> Default for Emitter<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Send + 'static> Emitter<T> {
    /// A new emitter with the default per-subscriber capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// A new emitter whose subscribers buffer up to `capacity` events.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Deliver an event to every current subscriber. Silently does nothing
    /// when nobody is listening.
    pub fn emit(&self, event: T) {
        let _ = self.tx.send(event);
    }

    /// A receiver for every event emitted from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.tx.subscribe()
    }

    /// Run `f` for every event on a background task. The callback keeps
    /// running after the handle is dropped; call [`Listener::off`] to stop
    /// it, mirroring `emitter.off(event, handler)`.
    ///
    /// Must be called from within a tokio runtime.
    pub fn on<F>(&self, mut f: F) -> Listener
    where
        F: FnMut(T) + Send + 'static,
    {
        let mut rx = self.subscribe();
        let handle = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => f(event),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Listener { handle }
    }

    /// Wait for the next event, or for the first event `pred` accepts.
    pub async fn once<P>(&self, mut pred: P) -> Option<T>
    where
        P: FnMut(&T) -> bool,
    {
        let mut rx = self.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) if pred(&event) => return Some(event),
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Number of live subscribers.
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// A registered callback. Dropping it does not stop the callback; call
/// [`off`](Listener::off) for that.
#[derive(Debug)]
pub struct Listener {
    handle: JoinHandle<()>,
}

impl Listener {
    /// Stop delivering events to the callback.
    pub fn off(self) {
        self.handle.abort();
    }
}
