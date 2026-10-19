//! The [`ProxyEvent`] queue and its [`EventProxy`] implementation, ported from
//! `vizia_baseview::proxy`. Since the audio/host thread can't touch vizia's
//! UI-thread-bound `Context` directly, synchronous signal writes enqueue
//! [`ProxyEvent`]s here and the frame loop drains them via [`queue_get`].

use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};

use vizia_core::context::EventProxy;
use vizia_core::events::ProxyEvent;

/// A queued [`ProxyEvent`] that another thread posted for the UI thread to act
/// on during the next frame. vizia's synchronous signal writes enqueue these;
/// the frame loop drains them via [`queue_get`] then forwards them to the
/// reactive context.
pub(crate) static PROXY_QUEUE: LazyLock<Mutex<VecDeque<ProxyEvent>>> =
    LazyLock::new(Mutex::default);

pub(crate) fn queue_put(event: ProxyEvent) {
    PROXY_QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(event)
}

pub(crate) fn queue_get() -> Option<ProxyEvent> {
    PROXY_QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop_front()
}

/// The [`EventProxy`] installed on vizia's `BackendContext` so that
/// off-UI-thread (e.g. audio thread) `SyncSignal` writes can enqueue work for
/// the UI thread.
#[derive(Clone)]
pub(crate) struct BaseviewProxy;

impl EventProxy for BaseviewProxy {
    fn send(&self, event: ProxyEvent) -> Result<(), ()> {
        queue_put(event);
        Ok(())
    }

    fn make_clone(&self) -> Box<dyn EventProxy> {
        Box::new(self.clone())
    }
}
