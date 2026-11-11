//! Per-window event queues. Proxies stop accepting events when their editor closes.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};
use vizia_core::context::EventProxy;
use vizia_core::events::ProxyEvent;

#[derive(Default)]
pub(crate) struct ProxyQueue(Arc<Mutex<VecDeque<ProxyEvent>>>);

impl ProxyQueue {
    pub(crate) fn proxy(&self) -> BaseviewProxy {
        BaseviewProxy { queue: Arc::downgrade(&self.0) }
    }

    pub(crate) fn take_events(&self) -> Vec<ProxyEvent> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
    }
}

#[derive(Clone)]
pub(crate) struct BaseviewProxy {
    queue: Weak<Mutex<VecDeque<ProxyEvent>>>,
}

impl EventProxy for BaseviewProxy {
    fn send(&self, event: ProxyEvent) -> Result<(), ()> {
        let queue = self.queue.upgrade().ok_or(())?;
        queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(event);
        // Continuous rendering drains this queue on the next frame.
        Ok(())
    }

    fn make_clone(&self) -> Box<dyn EventProxy> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queues_are_isolated_and_proxies_do_not_keep_them_alive() {
        let first = ProxyQueue::default();
        let second = ProxyQueue::default();
        let proxy = first.proxy();
        proxy.send(ProxyEvent::new(1u32)).unwrap();
        second.proxy().send(ProxyEvent::new(2u32)).unwrap();
        assert_eq!(first.take_events().len(), 1);
        assert!(first.take_events().is_empty());
        assert_eq!(second.take_events().len(), 1);
        let clone = proxy.make_clone();
        drop(first);
        assert!(proxy.send(ProxyEvent::new(3u32)).is_err());
        assert!(clone.send(ProxyEvent::new(4u32)).is_err());
        assert!(second.proxy().send(ProxyEvent::new(5u32)).is_ok());
    }
}
