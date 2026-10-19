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
//! The upstream `vizia_baseview` is tied to baseview 0.2.x's different handler
//! model (`on_frame`) and its own git-fork of baseview; we instead drive vizia
//! from the **main published `baseview` (0.4.x)** directly so the editor's
//! `EditorHandle::Window` can be a real `baseview::Window` exposing the
//! window-control API nice-plug needs.

mod application;
mod proxy;
mod window;

pub(crate) use proxy::BaseviewProxy;
pub(crate) use window::ViziaWindowHandle;
