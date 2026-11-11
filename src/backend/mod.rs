//! vizia's baseview rendering + event backend, merged in from the upstream
//! `vizia_baseview` crate and split into the same modules as the original:
//!
//! - [`proxy`] — the [`ProxyEvent`] queue + `EventProxy` installed on vizia's
//!   `BackendContext` so off-UI-thread writes can reach the UI thread.
//! - [`window`] — the baseview [`WindowHandler`][baseview::WindowHandler] that
//!   embeds a vizia application, plus the skia render-surface helper.
//! - [`application`] — the reactive frame loop and baseview→vizia event
//!   translation (the ported `ApplicationRunner`).
//!
//! Like upstream `vizia_baseview`, we use published baseview 0.4.x. We keep
//! this backend local to integrate nice-plug's host callbacks and continuous
//! parameter/key polling, while exposing the editor's
//! `EditorHandle::Window` can be a real `baseview::Window` exposing the
//! window-control API nice-plug needs.

mod application;
mod proxy;
mod window;

pub(crate) use window::ViziaWindowHandle;
