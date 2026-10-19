//! The [`Event`] queue and its [`EventProxy`] implementation, ported from
//! `vizia_baseview::proxy`. Since the audio/host thread can't touch vizia's
//! UI-thread-bound `Context` directly, background context proxies enqueue
//! [`Event`]s here and the frame loop drains them via [`queue_get`].

use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};

use vizia_core::context::EventProxy;
use vizia_core::prelude::Event;

/// A queued [`Event`] that another thread posted for the UI thread to act
/// on during the next frame. vizia's background context proxies enqueue these;
/// the frame loop drains them via [`queue_get`] then forwards them to the
/// reactive context.
pub(crate) static PROXY_QUEUE: LazyLock<Mutex<VecDeque<Event>>> =
    LazyLock::new(Mutex::default);

pub(crate) fn queue_put(event: Event) {
    PROXY_QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(event)
}

pub(crate) fn queue_get() -> Option<Event> {
    PROXY_QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop_front()
}

/// The [`EventProxy`] installed on vizia's `BackendContext` so that
/// off-UI-thread context proxies can enqueue events for
/// the UI thread.
#[derive(Clone)]
pub(crate) struct BaseviewProxy;

impl EventProxy for BaseviewProxy {
    fn send(&self, event: Event) -> Result<(), ()> {
        queue_put(event);
        Ok(())
    }

    fn make_clone(&self) -> Box<dyn EventProxy> {
        Box::new(self.clone())
    }
}
