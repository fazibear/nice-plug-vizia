//! The reactive frame loop and event translation behind a vizia window.
//!
//! This is the ported core of `vizia_baseview::ApplicationRunner`, re-typed
//! against the **main published `baseview` (0.4.x)** API. It owns the vizia
//! `BackendContext` and skia render surfaces, drains the
//! [`proxy`][super::proxy] event queue each frame, feeds platform events into
//! vizia, and recreates the GL surfaces on resize / scale change.

use baseview::{Event as BaseviewEvent, MouseEvent, ScrollDelta, WindowContext, WindowSize};
use gl::types::GLint;
use keyboard_types::{KeyState, Modifiers as InputModifiers};
use skia_safe::gpu::gl::FramebufferInfo;
use vizia_core::backend::*;
use vizia_core::events::EventManager;
use vizia_core::prelude::*;

use super::proxy::queue_get;
use super::window::create_surface;

/// The state + event loop behind a vizia window. This is the ported core of
/// `vizia_baseview::ApplicationRunner`.
pub(crate) struct ApplicationRunner {
    cx: BackendContext,
    event_manager: EventManager,
    pub window_context: WindowContext,
    pub gr_context: skia_safe::gpu::DirectContext,
    should_redraw: bool,

    /// The scale factor of the window itself, as reported by baseview. Used to
    /// convert logical coordinates to physical ones; for vizia the DPI factor
    /// is this multiplied by the user scale factor.
    window_scale_factor: f64,
    pub surface: skia_safe::Surface,
    pub dirty_surface: skia_safe::Surface,
    window_description: WindowDescription,
    is_initialized: bool,
    /// `true` when the window is embedded as a child of a host (a DAW). Gates
    /// lifecycle decisions that should be left to the host — e.g. not
    /// interpreting Cmd+Q as a close request.
    is_parented: bool,
}

impl ApplicationRunner {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cx: BackendContext,
        gr_context: skia_safe::gpu::DirectContext,
        window_scale_factor: f64,
        surface: skia_safe::Surface,
        dirty_surface: skia_safe::Surface,
        window_description: WindowDescription,
        is_parented: bool,
        window_context: WindowContext,
    ) -> Self {
        ApplicationRunner {
            should_redraw: true,
            gr_context,
            event_manager: EventManager::new(),
            window_context,
            window_scale_factor,
            cx,
            surface,
            dirty_surface,
            window_description,
            is_initialized: false,
            is_parented,
        }
    }

    /// Apply a new user scale factor: resize the embedded window, update
    /// vizia's tracked DPI factor + root viewport bounds, and recreate the
    /// skia render surfaces at the new physical size.
    fn apply_user_scale(&mut self, new_user_scale: f64) {
        self.window_description.user_scale_factor = new_user_scale;

        // baseview treats the user scale factor as part of its logical size.
        let base_logical_w = self.window_description.inner_size.width as f64;
        let base_logical_h = self.window_description.inner_size.height as f64;
        let scaled_logical_w = base_logical_w * new_user_scale;
        let scaled_logical_h = base_logical_h * new_user_scale;
        let _ = self
            .window_context
            .resize(baseview::dpi::LogicalSize::new(scaled_logical_w, scaled_logical_h));

        let new_dpi_factor = self.window_scale_factor * new_user_scale;
        self.cx.set_scale_factor(new_dpi_factor);

        let new_physical_w = (scaled_logical_w * self.window_scale_factor) as f32;
        let new_physical_h = (scaled_logical_h * self.window_scale_factor) as f32;
        self.cx.set_window_size(Entity::root(), new_physical_w, new_physical_h);

        if new_physical_w > 0.0 && new_physical_h > 0.0 {
            let fb_info = {
                let mut fboid: GLint = 0;
                // SAFETY: the GL context is current (the frame loop calls us).
                unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid) };

                FramebufferInfo {
                    fboid: fboid.try_into().unwrap(),
                    format: skia_safe::gpu::gl::Format::RGBA8.into(),
                    ..Default::default()
                }
            };

            self.surface = create_surface(
                (new_physical_w as i32, new_physical_h as i32),
                fb_info,
                &mut self.gr_context,
            );

            self.dirty_surface = self
                .surface
                .new_surface_with_dimensions((
                    new_physical_w.max(1.0) as i32,
                    new_physical_h.max(1.0) as i32,
                ))
                .unwrap();
        }

        self.cx.needs_refresh(Entity::root());
    }

    /// Handle all reactivity within a frame.
    pub fn on_frame_update(&mut self) {
        // Pick up effects enqueued by off-UI-thread `SyncSignal` writes.
        Runtime::drain_pending_work();

        while let Some(event) = queue_get() {
            self.cx.send_event(event.into_event());
        }

        // Events. The flush callback can't borrow `&mut self`, so size / scale
        // change requests get latched into locals here and applied afterwards.
        let mut pending_user_scale: Option<f64> = None;
        self.event_manager.flush_events(self.cx.context(), |window_event| {
            match window_event {
                WindowEvent::FocusIn => {
                    #[cfg(not(target_os = "linux"))] // not implemented for linux yet
                    if !self.window_context.has_focus() {
                        let _ = self.window_context.focus();
                    }
                }
                WindowEvent::SetUserScale(factor) => {
                    pending_user_scale = Some(*factor);
                }
                _ => {}
            }
        });

        if let Some(new_user_scale) = pending_user_scale {
            self.apply_user_scale(new_user_scale);
        }

        // We need to resize the window to make sure the new size is applied.
        // This is a workaround for the fact that baseview does not resize the
        // window when the scale factor changes.
        if !self.is_initialized {
            // Resizing the window doesn't apply unless the size has actually
            // changed, so we resize it slightly larger and then back again.
            let _ = self.window_context.resize(baseview::dpi::LogicalSize::new(
                self.window_description.inner_size.width as f64 + 1.0,
                self.window_description.inner_size.height as f64 + 1.0,
            ));
            let _ = self.window_context.resize(baseview::dpi::LogicalSize::new(
                self.window_description.inner_size.width as f64,
                self.window_description.inner_size.height as f64,
            ));
            self.is_initialized = true;
        }

        let context = self
            .window_context
            .gl_context()
            .expect("Window was created without OpenGL support");
        // SAFETY: we're on the UI thread and no other GL usage is in flight.
        unsafe { let _ = context.make_current(); };
        self.cx.process_style_updates();
        unsafe { let _ = context.make_not_current(); };

        self.cx.process_animations();
        self.cx.process_visual_updates();

        if self.cx.0.windows.iter().any(|(_, window_state)| !window_state.redraw_list.is_empty()) {
            self.should_redraw = true;
        }

        self.cx.process_timers();
    }

    pub fn render(&mut self) {
        if self.should_redraw {
            let context = self
                .window_context
                .gl_context()
                .expect("Window was created without OpenGL support");
            // SAFETY: we're on the UI thread.
            unsafe { let _ = context.make_current(); };
            self.cx.draw(Entity::root(), &mut self.surface, &mut self.dirty_surface);
            self.gr_context.flush_and_submit();
            self.should_redraw = false;
            let _ = context.swap_buffers();
            unsafe { let _ = context.make_not_current(); };
        }
    }

    pub fn handle_event(&mut self, event: BaseviewEvent) {
        if requests_exit(&event, self.is_parented) {
            self.cx.send_event(Event::new(WindowEvent::WindowClose));
            self.window_context.request_close();
        }

        let mut update_modifiers = |modifiers: InputModifiers| {
            self.cx
                .modifiers()
                .set(Modifiers::SHIFT, modifiers.contains(InputModifiers::SHIFT));
            self.cx
                .modifiers()
                .set(Modifiers::CTRL, modifiers.contains(InputModifiers::CONTROL));
            self.cx
                .modifiers()
                .set(Modifiers::SUPER, modifiers.contains(InputModifiers::META));
            self.cx
                .modifiers()
                .set(Modifiers::ALT, modifiers.contains(InputModifiers::ALT));
        };

        match event {
            BaseviewEvent::Mouse(event) => match event {
                MouseEvent::CursorMoved { position, modifiers } => {
                    update_modifiers(modifiers);

                    // baseview delivers cursor coordinates in physical pixels,
                    // so no additional DPI scaling is applied here.
                    let cursor_x = position.x as f32;
                    let cursor_y = position.y as f32;
                    self.cx.emit_origin(WindowEvent::MouseMove(cursor_x, cursor_y));
                }
                MouseEvent::ButtonPressed { button, modifiers } => {
                    update_modifiers(modifiers);
                    let b = translate_mouse_button(button);
                    self.cx.emit_origin(WindowEvent::MouseDown(b));
                }
                MouseEvent::ButtonReleased { button, modifiers } => {
                    update_modifiers(modifiers);
                    let b = translate_mouse_button(button);
                    self.cx.emit_origin(WindowEvent::MouseUp(b));
                }
                MouseEvent::WheelScrolled { delta, modifiers } => {
                    update_modifiers(modifiers);

                    let (lines_x, lines_y) = match delta {
                        ScrollDelta::Lines { x, y } => (x, y),
                        ScrollDelta::Pixels { x, y } => (
                            if x < 0.0 {
                                -1.0
                            } else if x > 1.0 {
                                1.0
                            } else {
                                0.0
                            },
                            if y < 0.0 {
                                -1.0
                            } else if y > 1.0 {
                                1.0
                            } else {
                                0.0
                            },
                        ),
                    };

                    self.cx.emit_origin(WindowEvent::MouseScroll(lines_x, lines_y));
                }
                MouseEvent::CursorEntered => {
                    self.cx.emit_origin(WindowEvent::MouseEnter);
                }
                MouseEvent::CursorLeft => {
                    self.cx.emit_origin(WindowEvent::MouseLeave);
                }
                _ => {}
            },
            BaseviewEvent::Keyboard(event) => {
                let pressed = match event.state {
                    KeyState::Down => true,
                    KeyState::Up => false,
                };

                match event.code {
                    Code::ShiftLeft | Code::ShiftRight => {
                        self.cx.modifiers().set(Modifiers::SHIFT, pressed)
                    }
                    Code::ControlLeft | Code::ControlRight => {
                        self.cx.modifiers().set(Modifiers::CTRL, pressed)
                    }
                    Code::AltLeft | Code::AltRight => {
                        self.cx.modifiers().set(Modifiers::ALT, pressed)
                    }
                    Code::MetaLeft | Code::MetaRight => {
                        self.cx.modifiers().set(Modifiers::SUPER, pressed)
                    }
                    _ => (),
                }

                if pressed {
                    if let Key::Character(written) = &event.key {
                        for chr in written.chars() {
                            self.cx.emit_origin(WindowEvent::CharInput(chr));
                        }
                    }

                    self.cx.emit_origin(WindowEvent::KeyDown(event.code, Some(event.key)));
                } else {
                    self.cx.emit_origin(WindowEvent::KeyUp(event.code, Some(event.key)));
                }
            }
            BaseviewEvent::Window(event) => match event {
                baseview::WindowEvent::Focused => self.cx.needs_refresh(Entity::root()),
                baseview::WindowEvent::WillClose => {
                    self.cx.send_event(Event::new(WindowEvent::WindowClose));
                }
                _ => {}
            },
            _ => {}
        }
    }

    pub fn handle_resized(&mut self, new_size: WindowSize) {
        let context = self
            .window_context
            .gl_context()
            .expect("Window was created without OpenGL support");
        // SAFETY: we're on the UI thread.
        unsafe { let _ = context.make_current(); };

        let fb_info = {
            let mut fboid: GLint = 0;
            // SAFETY: context is current.
            unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid) };

            FramebufferInfo {
                fboid: fboid.try_into().unwrap(),
                format: skia_safe::gpu::gl::Format::RGBA8.into(),
                ..Default::default()
            }
        };

        let width = new_size.physical.width.max(1) as i32;
        let height = new_size.physical.height.max(1) as i32;

        self.surface = create_surface((width, height), fb_info, &mut self.gr_context);

        self.dirty_surface = self.surface.new_surface_with_dimensions((width, height)).unwrap();

        unsafe { let _ = context.make_not_current(); };

        self.window_scale_factor = new_size.scale_factor;

        self.cx
            .set_scale_factor(self.window_scale_factor * self.window_description.user_scale_factor);

        self.cx.set_window_size(
            Entity::root(),
            new_size.physical.width as f32,
            new_size.physical.height as f32,
        );

        self.cx.needs_refresh(Entity::root());
    }

    pub fn handle_idle(&mut self, on_idle: &Option<Box<dyn Fn(&mut Context) + Send>>) {
        if let Some(idle_callback) = on_idle {
            self.cx.set_current(Entity::root());
            (idle_callback)(self.cx.context());
        }
    }

    /// Element name of the view that currently has keyboard focus.
    pub fn focused_element(&self) -> Option<&'static str> {
        self.cx.focused_element()
    }
}

/// Returns true if the provided event should cause the application to exit.
///
/// `WindowEvent::WillClose` is honoured in both standalone and parented modes.
/// On macOS, Cmd+Q is recognised as a quit shortcut **only** when the
/// application is standalone (`is_parented == false`); when embedded in a host
/// the host owns the application lifecycle.
pub fn requests_exit(
    event: &BaseviewEvent,
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] is_parented: bool,
) -> bool {
    match event {
        BaseviewEvent::Window(baseview::WindowEvent::WillClose) => true,
        #[cfg(target_os = "macos")]
        BaseviewEvent::Keyboard(event) if !is_parented => {
            if event.code == Code::KeyQ
                && event.modifiers == InputModifiers::META
                && event.state == KeyState::Down
            {
                return true;
            }

            false
        }
        _ => false,
    }
}

fn translate_mouse_button(button: baseview::MouseButton) -> MouseButton {
    match button {
        baseview::MouseButton::Left => MouseButton::Left,
        baseview::MouseButton::Right => MouseButton::Right,
        baseview::MouseButton::Middle => MouseButton::Middle,
        baseview::MouseButton::Back => MouseButton::Back,
        baseview::MouseButton::Forward => MouseButton::Forward,
        baseview::MouseButton::Other(id) => MouseButton::Other(id as u16),
        _ => MouseButton::Other(0),
    }
}
