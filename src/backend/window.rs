//! The baseview [`WindowHandler`] that embeds a vizia application, plus the
//! skia render-surface helper. This is the ported core of
//! `vizia_baseview::window::ViziaWindow`, re-typed against the main published
//! **`baseview` (0.4.x)** handler model (`draw`/`resized`/`on_event`).

use std::cell::RefCell;

use baseview::{
    Event as BaseviewEvent, EventStatus, WindowContext, WindowHandler, WindowSize,
};
use gl::types::GLint;
use skia_safe::gpu::gl::FramebufferInfo;
use skia_safe::gpu::{
    self, ContextOptions, SurfaceOrigin, backend_render_targets, ganesh::context_options,
};
use skia_safe::{ColorSpace, ColorType, PixelGeometry, Surface, SurfaceProps, SurfacePropsFlags};
use vizia_core::backend::*;
use vizia_core::prelude::*;

use super::application::ApplicationRunner;

/// The baseview window handler for a vizia editor window. baseview invokes the
/// handler methods through `&self`, so the inner vizia state lives behind a
/// `RefCell`.
pub(crate) struct ViziaWindowHandle {
    application: RefCell<ApplicationRunner>,
    on_idle: Option<Box<dyn Fn(&mut Context, bool) + Send>>,
}

impl ViziaWindowHandle {
    /// Create the inner vizia [`ApplicationRunner`] for a window that baseview
    /// has just created. This mirrors `vizia_baseview::ViziaWindow::new`:
    /// initialises the reactive runtime, builds the GL/skia renderer, and runs
    /// the user's app builder against the freshly created vizia `Context`.
    pub(crate) fn new(
        mut cx: BackendContext,
        win_desc: WindowDescription,
        window: WindowContext,
        builder: Option<Box<dyn FnOnce(&mut Context) + Send>>,
        on_idle: Option<Box<dyn Fn(&mut Context, bool) + Send>>,
        is_parented: bool,
    ) -> Result<ViziaWindowHandle, baseview::HandlerError> {
        // Reactive runtime setup — mirrors what `vizia_winit::Application::new`
        // does. Mark this thread as the UI thread so off-thread `SyncSignal`
        // writes correctly enqueue effects on `SYNC_RUNTIME`; the frame loop
        // later drains them via `Runtime::drain_pending_work`.
        Runtime::init_on_ui_thread();

        // Register a no-op sync waker. We keep the window redrawing at the
        // host compositor's frame rate, so signal changes from off-UI writes
        // are observed on the next frame (at most one frame of latency).
        Runtime::set_sync_effect_waker(|| {});

        let context = window
            .gl_context()
            .ok_or_else(|| baseview::HandlerError::from_boxed("no OpenGL context".into()))?;

        let get_proc = |symbol: &str| context.get_proc_address_from_str(symbol);

        // SAFETY: we are on the window/UI thread and hold the only reference
        // to the GL context at init time.
        unsafe { context.make_current()? };
        gl::load_with(get_proc);
        let interface = skia_safe::gpu::gl::Interface::new_load_with(|name| {
            if name == "eglGetCurrentDisplay" {
                return std::ptr::null();
            }
            context.get_proc_address_from_str(name)
        })
        .ok_or_else(|| baseview::HandlerError::from_boxed("Could not create GL interface".into()))?;

        // https://github.com/rust-skia/rust-skia/issues/476
        let mut context_options = ContextOptions::new();
        context_options.skip_gl_error_checks = context_options::Enable::Yes;

        let mut gr_context = skia_safe::gpu::direct_contexts::make_gl(interface, &context_options)
            .ok_or_else(|| {
                baseview::HandlerError::from_boxed("Could not create skia direct context".into())
            })?;

        let fb_info = {
            let mut fboid: GLint = 0;
            // SAFETY: the GL context is current.
            unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid) };

            FramebufferInfo {
                fboid: fboid.try_into().unwrap(),
                format: skia_safe::gpu::gl::Format::RGBA8.into(),
                ..Default::default()
            }
        };

        let initial_size = window.size();
        let initial_width = initial_size.physical.width.max(1) as i32;
        let initial_height = initial_size.physical.height.max(1) as i32;

        let mut surface = create_surface((initial_width, initial_height), fb_info, &mut gr_context);

        let dirty_surface =
            surface.new_surface_with_dimensions((initial_width, initial_height)).unwrap();

        // baseview 0.4.x always uses the platform's system scaling, with the
        // `fallback_scale_factor` only consulted when the platform can't
        // provide one. vizia's own scale factor is the window's scale factor
        // multiplied by the user scale factor.
        let window_scale_factor = window.scale_factor();
        let dpi_factor = window_scale_factor * win_desc.user_scale_factor;

        cx.add_main_window(Entity::root(), &win_desc, dpi_factor as f32);
        cx.add_window(WindowView {});

        cx.0.windows.insert(
            Entity::root(),
            WindowState { window_description: win_desc.clone(), ..Default::default() },
        );

        cx.context().add_built_in_styles();
        if let Some(builder) = builder {
            (builder)(cx.context());
        }

        let application = ApplicationRunner::new(
            cx,
            gr_context,
            window_scale_factor,
            surface,
            dirty_surface,
            win_desc,
            is_parented,
            window,
        );

        // SAFETY: the GL context is current and only used from this thread.
        unsafe {
            let _ = context.make_not_current();
        }

        Ok(ViziaWindowHandle { application: RefCell::new(application), on_idle })
    }
}

impl WindowHandler for ViziaWindowHandle {
    fn draw(&self) -> Result<(), baseview::HandlerError> {
        // Some hosts dispatch baseview callbacks from a different UI thread
        // than the one that constructed the window; make sure we're
        // registered as the UI thread here too.
        Runtime::init_on_ui_thread();
        let Ok(mut application) = self.application.try_borrow_mut() else {
            return Ok(());
        };

        application.on_frame_update();

        // Run the embedder's idle callback on every frame so work queued from
        // off-UI threads (e.g. a plugin host delivering keys/param changes on
        // the host thread) is drained even when host events are sparse.
        application.handle_idle(&self.on_idle);

        application.render();

        // Keep the frame loop alive: this mirrors baseview 0.2.x's vsync-driven
        // `on_frame` behaviour that the original vizia_baseview relied on for
        // animations, timers and signal-driven repaints.
        application.window_context.request_redraw();

        Ok(())
    }

    fn resized(&self, new_size: WindowSize) -> Result<(), baseview::HandlerError> {
        Runtime::init_on_ui_thread();
        if let Ok(mut application) = self.application.try_borrow_mut() {
            application.handle_resized(new_size);
        }
        Ok(())
    }

    fn on_event(&self, event: BaseviewEvent) -> EventStatus {
        Runtime::init_on_ui_thread();
        let Ok(mut application) = self.application.try_borrow_mut() else {
            return EventStatus::Ignored;
        };

        // If a text input currently holds focus, a native keyboard event is
        // intended for the plugin. Report it as `Captured` so the backend does
        // not forward the platform event further — otherwise the host (e.g. a
        // DAW) also processes the same keystroke.
        let captured = matches!(event, BaseviewEvent::Keyboard(_))
            && application.focused_element() == Some("textbox");

        application.handle_event(event);
        application.handle_idle(&self.on_idle);

        if captured { EventStatus::Captured } else { EventStatus::Ignored }
    }
}

/// vizia's root window view.
pub struct WindowView {}

impl View for WindowView {}

/// Create a skia render surface wrapping the window's current GL framebuffer,
/// including the text properties (gamma + contrast) vizia expects for readable
/// text rendering.
pub fn create_surface(
    size: (i32, i32),
    fb_info: FramebufferInfo,
    gr_context: &mut skia_safe::gpu::DirectContext,
) -> Surface {
    let backend_render_target = backend_render_targets::make_gl(size, None, 8, fb_info);

    let surface_props = SurfaceProps::new_with_text_properties(
        SurfacePropsFlags::default(),
        PixelGeometry::default(),
        0.5,
        0.0,
    );

    gpu::surfaces::wrap_backend_render_target(
        gr_context,
        &backend_render_target,
        SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        ColorSpace::new_srgb(),
        Some(surface_props).as_ref(),
    )
    .expect("Could not create skia surface")
}
