//! The [`Editor`] trait implementation for VIZIA editors.

use baseview::{Window, WindowContext};
use nice_plug_core::context::gui::GuiContext;
use nice_plug_core::debug::*;
use nice_plug_core::editor::dpi::NativeSize;
use nice_plug_core::editor::{
    Editor, EditorHandle, HostMethods, Modifiers, ParentWindowHandle, SpawnedEditor, VirtualKeyCode,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use keyboard_types::NamedKey;
use vizia::prelude::*;
use vizia::views::TextEvent;

use crate::backend;
use crate::widgets::RawParamEvent;
use crate::widgets::param_registry::ParamRegistry;
use crate::{ViziaState, ViziaTheming, widgets};

/// A key-down event queued by the host-thread
/// `EditorHandle::on_virtual_key_from_host` callback, waiting for the next
/// `on_idle` tick to dispatch on the GUI thread.
///
/// Split between character input (goes through `TextEvent::InsertText`)
/// and non-printable control keys (goes through `WindowEvent::KeyDown`)
/// so the GUI-thread drain stays trivially correct without having to
/// re-guess a key's semantics.
pub(crate) enum KeyInject {
    Char(char),
    ControlKey(Code, Key),
}

/// State shared between the host UI thread
/// (`EditorHandle::on_virtual_key_from_host`) and the vizia `on_idle` callback
/// (invoked on the GUI thread).
pub(crate) struct KeyInjectState {
    text_focused: AtomicBool,
    pending: Mutex<VecDeque<KeyInject>>,
}

impl KeyInjectState {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            text_focused: AtomicBool::new(false),
            pending: Mutex::new(VecDeque::new()),
        })
    }
}

/// An [`Editor`] implementation that calls a vizia draw loop.
///
/// This is the concrete (non-generic) editor type returned by
/// [`crate::create_vizia_editor`], so it can be used directly as
/// `Plugin::Editor`.
pub struct ViziaEditor {
    pub(crate) vizia_state: Arc<ViziaState>,
    /// The user's app function.
    pub(crate) app: Arc<dyn Fn(&mut Context, GuiContext) + 'static + Send + Sync>,
    /// What level of theming to apply.
    pub(crate) theming: ViziaTheming,

    /// Whether to emit a parameters changed event during the next idle callback.
    pub(crate) emit_parameters_changed_event: Arc<AtomicBool>,

    /// Shared registry of `SyncSignal<f32>`s tracking each parameter's live value.
    pub(crate) param_registry: ParamRegistry,

    /// Shared state bridging the host-thread key callback with the GUI thread.
    pub(crate) key_inject: Arc<KeyInjectState>,
}

impl Editor for ViziaEditor {
    type Handle = ViziaEditorHandle;

    fn spawn(
        &self,
        parent: Option<ParentWindowHandle>,
        wait_for_parent: bool,
        fallback_scale_factor: Option<f64>,
        gui_context: GuiContext,
        host: Option<HostMethods>,
    ) -> Result<SpawnedEditor<Self::Handle>, Box<dyn std::error::Error>> {
        let app = self.app.clone();
        let vizia_state = self.vizia_state.clone();
        let theming = self.theming;
        let param_registry = self.param_registry.clone();
        let param_registry_for_build = param_registry.clone();

        // Prevent stale runtime-bound signals from previous editor instances from being reused.
        param_registry.clear_signals();

        let (unscaled_width, unscaled_height) = vizia_state.inner_logical_size();
        let user_scale_factor = vizia_state.user_scale_factor();

        // The size baseview creates the window at. baseview treats the user
        // scale factor as part of the logical size, so fold it in here.
        let logical_width = unscaled_width as f64 * user_scale_factor;
        let logical_height = unscaled_height as f64 * user_scale_factor;

        let emit_parameters_changed_event = self.emit_parameters_changed_event.clone();
        let key_inject = self.key_inject.clone();

        // Runs every frame on the GUI thread after events have been handled:
        // emits `ParametersChanged` when the host reported a value change, and
        // drains keys queued by the host-thread virtual-key callback.
        let on_idle: Box<dyn Fn(&mut Context) + Send> = Box::new(move |cx| {
            if emit_parameters_changed_event
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                cx.emit_custom(
                    Event::new(RawParamEvent::ParametersChanged)
                        .propagate(Propagation::Subtree),
                );
            }

            // Keep `text_focused` in sync so the host-thread key callback can
            // decide synchronously whether to claim a key.
            key_inject
                .text_focused
                .store(cx.focused_element() == Some("textbox"), Ordering::Release);

            let drained: Vec<KeyInject> = {
                let mut q = key_inject.pending.lock().unwrap_or_else(|e| e.into_inner());
                q.drain(..).collect()
            };
            if !drained.is_empty() {
                let mut ec = EventContext::new(cx);
                let target = ec.focused();
                for entry in drained {
                    match entry {
                        KeyInject::Char(c) => {
                            ec.emit_to(target, TextEvent::InsertText(c.to_string()));
                        }
                        KeyInject::ControlKey(code, key) => {
                            ec.emit_to(target, WindowEvent::KeyDown(code, Some(key)));
                        }
                    }
                }
            }
        });

        // Builds the vizia context the first time the window is opened.
        let builder: Box<dyn FnOnce(&mut Context) + Send> = Box::new(move |cx| {
            // Apply the custom stylesheet + register the bundled widget themes.
            if let Err(err) = cx.add_stylesheet(include_style!("src/assets/theme.css")) {
                nice_error!("Failed to load stylesheet: {err:?}");
                panic!();
            }

            widgets::register_theme(cx);

            // Install the parameter signal registry so widgets can find it via
            // `cx.data::<ParamRegistry>()`.
            param_registry_for_build.clone().build(cx);

            // Any widget can change the parameters by emitting `ParamEvent`
            // events. This model handles them automatically.
            widgets::ParamModel {
                context: gui_context.clone(),
            }
            .build(cx);

            // Link `WindowEvent::ResizeWindow` / `WindowEvent::SetScale` to the
            // `ViziaState` and notify the host when they change.
            let current_inner_window_size = EventContext::new(cx).cache.get_bounds(Entity::root());
            widgets::WindowModel {
                context: gui_context.clone(),
                vizia_state: vizia_state.clone(),
                last_inner_window_size: crossbeam::atomic::AtomicCell::new((
                    current_inner_window_size.width() as u32,
                    current_inner_window_size.height() as u32,
                )),
            }
            .build(cx);

            app(cx, gui_context.clone())
        });

        let win_desc = vizia::backend::WindowDescription {
            title: String::from("VIZIA"),
            inner_size: (unscaled_width, unscaled_height).into(),
            user_scale_factor,
            ..Default::default()
        };

        let mut settings = baseview::WindowSettings::new()
            .with_title(win_desc.title.clone())
            .with_size(baseview::dpi::LogicalSize::new(logical_width, logical_height))
            .with_fallback_scale_factor(fallback_scale_factor)
            .with_gl_config(baseview::gl::GlConfig {
                ..Default::default()
            })
            .with_wait_for_parent(wait_for_parent);

        if let Some(parent) = &parent {
            settings = settings.with_parent(parent);
        }

        // Adapt nice-plug's `HostMethods` (its re-implementation of baseview's
        // `HostCallbacks` / `HostMainThreadCaller`) to baseview's own host API.
        let host = host.map(|host| {
            struct HostCallbackAdapter {
                host: Box<dyn nice_plug_core::editor::HostCallbacks>,
            }
            impl baseview::host::HostCallbacks for HostCallbackAdapter {
                fn request_resize(
                    &mut self,
                    new_size: baseview::WindowSize,
                ) -> Result<(), baseview::HandlerError> {
                    self.host
                        .request_resize(new_size.physical.into(), new_size.scale_factor)
                        .map_err(baseview::HandlerError::from_boxed)
                }

                fn destroyed(&mut self) {
                    self.host.destroyed();
                }
            }
            struct HostMainThreadCallerAdapter {
                host: Box<dyn nice_plug_core::editor::HostMainThreadCaller>,
            }
            impl baseview::host::HostMainThreadCaller for HostMainThreadCallerAdapter {
                fn call_main_thread(&mut self) {
                    self.host.call_main_thread();
                }
            }
            baseview::host::Host::new()
                .with_callbacks(HostCallbackAdapter {
                    host: host.callbacks,
                })
                .with_main_thread(HostMainThreadCallerAdapter {
                    host: host.main_thread_caller,
                })
        });

        let window = Window::create_with_host(
            settings,
            move |ctx: WindowContext| {
                let mut cx = Context::new();
                cx.ignore_default_theme = theming == ViziaTheming::None;
                cx.add_built_in_styles();

                let mut cx = vizia::backend::BackendContext::new(cx);
                cx.set_event_proxy(Box::new(backend::BaseviewProxy));
                backend::ViziaWindowHandle::new(
                    cx,
                    win_desc,
                    ctx,
                    Some(builder),
                    Some(on_idle),
                    true, // is_parented
                )
            },
            host,
        )?;

        self.vizia_state.open.store(true, Ordering::Release);
        Ok(SpawnedEditor {
            handle: ViziaEditorHandle {
                key_inject: self.key_inject.clone(),
                emit_parameters_changed_event: self.emit_parameters_changed_event.clone(),
                param_registry,
                vizia_state: self.vizia_state.clone(),
            },
            window,
        })
    }

    fn size(&self) -> NativeSize<u32> {
        // This includes the user scale factor if set, but not any HiDPI scaling.
        let (width, height) = self.vizia_state.scaled_logical_size();
        NativeSize { width, height }
    }
}

/// The window handle used for [`ViziaEditor`].
pub struct ViziaEditorHandle {
    vizia_state: Arc<ViziaState>,
    emit_parameters_changed_event: Arc<AtomicBool>,
    param_registry: ParamRegistry,
    key_inject: Arc<KeyInjectState>,
}

impl EditorHandle for ViziaEditorHandle {
    type Window = Window;
    type Error = baseview::Error;

    fn run_until_closed(window: Self::Window) -> Result<(), Self::Error> {
        window.run_until_closed()
    }

    fn set_parent(
        &self,
        parent: ParentWindowHandle,
        window: &Self::Window,
    ) -> Result<(), Self::Error> {
        window.set_parent(baseview::ParentWindowHandle::from_window(&parent))
    }

    fn show(&self, window: &Self::Window) -> Result<(), Self::Error> {
        window.show()
    }

    fn hide(&self, window: &Self::Window) -> Result<(), Self::Error> {
        window.hide()
    }

    fn set_size(
        &self,
        new_size: NativeSize<u32>,
        window: &Self::Window,
    ) -> Result<(), Self::Error> {
        window.resize(new_size)
    }

    fn host_main_thread_callback(&self, window: &Self::Window) {
        window.host_main_thread_callback();
    }

    fn set_fallback_scale_factor(
        &self,
        scale_factor: f64,
        window: &Self::Window,
    ) -> Result<(), Self::Error> {
        window.suggest_fallback_scale_factor(scale_factor)
    }

    fn on_virtual_key_from_host(
        &self,
        key_code: VirtualKeyCode,
        is_down: bool,
        modifiers: Modifiers,
    ) -> bool {
        // Called from the host's UI thread. Claim the key only when a textbox
        // is currently focused; otherwise the host's accelerator should run.
        if !self.key_inject.text_focused.load(Ordering::Acquire) {
            return false;
        }

        // Modifier-held combinations are claimed by the host or handled by
        // AppKit's normal text-input path; dispatching through our injection
        // queue would double-fire and lose modifier context.
        if !modifiers.is_empty() {
            return false;
        }

        let inject = match key_code {
            VirtualKeyCode::Space => Some(KeyInject::Char(' ')),
            VirtualKeyCode::Numpad0 => Some(KeyInject::Char('0')),
            VirtualKeyCode::Numpad1 => Some(KeyInject::Char('1')),
            VirtualKeyCode::Numpad2 => Some(KeyInject::Char('2')),
            VirtualKeyCode::Numpad3 => Some(KeyInject::Char('3')),
            VirtualKeyCode::Numpad4 => Some(KeyInject::Char('4')),
            VirtualKeyCode::Numpad5 => Some(KeyInject::Char('5')),
            VirtualKeyCode::Numpad6 => Some(KeyInject::Char('6')),
            VirtualKeyCode::Numpad7 => Some(KeyInject::Char('7')),
            VirtualKeyCode::Numpad8 => Some(KeyInject::Char('8')),
            VirtualKeyCode::Numpad9 => Some(KeyInject::Char('9')),
            VirtualKeyCode::NumpadMultiply => Some(KeyInject::Char('*')),
            VirtualKeyCode::NumpadAdd => Some(KeyInject::Char('+')),
            VirtualKeyCode::NumpadSeparator => Some(KeyInject::Char(',')),
            VirtualKeyCode::NumpadSubtract => Some(KeyInject::Char('-')),
            VirtualKeyCode::NumpadDecimal => Some(KeyInject::Char('.')),
            VirtualKeyCode::NumpadDivide => Some(KeyInject::Char('/')),
            VirtualKeyCode::Equals => Some(KeyInject::Char('=')),

            VirtualKeyCode::Backspace => {
                Some(KeyInject::ControlKey(Code::Backspace, Key::Named(NamedKey::Backspace)))
            }
            VirtualKeyCode::Tab => Some(KeyInject::ControlKey(Code::Tab, Key::Named(NamedKey::Tab))),
            VirtualKeyCode::Return => Some(KeyInject::ControlKey(Code::Enter, Key::Named(NamedKey::Enter))),
            VirtualKeyCode::NumpadEnter => {
                Some(KeyInject::ControlKey(Code::NumpadEnter, Key::Named(NamedKey::Enter)))
            }
            VirtualKeyCode::Pause => Some(KeyInject::ControlKey(Code::Pause, Key::Named(NamedKey::Pause))),
            VirtualKeyCode::Escape => Some(KeyInject::ControlKey(Code::Escape, Key::Named(NamedKey::Escape))),
            VirtualKeyCode::End => Some(KeyInject::ControlKey(Code::End, Key::Named(NamedKey::End))),
            VirtualKeyCode::Home => Some(KeyInject::ControlKey(Code::Home, Key::Named(NamedKey::Home))),
            VirtualKeyCode::ArrowLeft => {
                Some(KeyInject::ControlKey(Code::ArrowLeft, Key::Named(NamedKey::ArrowLeft)))
            }
            VirtualKeyCode::ArrowUp => Some(KeyInject::ControlKey(Code::ArrowUp, Key::Named(NamedKey::ArrowUp))),
            VirtualKeyCode::ArrowRight => {
                Some(KeyInject::ControlKey(Code::ArrowRight, Key::Named(NamedKey::ArrowRight)))
            }
            VirtualKeyCode::ArrowDown => {
                Some(KeyInject::ControlKey(Code::ArrowDown, Key::Named(NamedKey::ArrowDown)))
            }
            VirtualKeyCode::PageUp => Some(KeyInject::ControlKey(Code::PageUp, Key::Named(NamedKey::PageUp))),
            VirtualKeyCode::PageDown => Some(KeyInject::ControlKey(Code::PageDown, Key::Named(NamedKey::PageDown))),
            VirtualKeyCode::Insert => Some(KeyInject::ControlKey(Code::Insert, Key::Named(NamedKey::Insert))),
            VirtualKeyCode::Delete => Some(KeyInject::ControlKey(Code::Delete, Key::Named(NamedKey::Delete))),
            VirtualKeyCode::F1 => Some(KeyInject::ControlKey(Code::F1, Key::Named(NamedKey::F1))),
            VirtualKeyCode::F2 => Some(KeyInject::ControlKey(Code::F2, Key::Named(NamedKey::F2))),
            VirtualKeyCode::F3 => Some(KeyInject::ControlKey(Code::F3, Key::Named(NamedKey::F3))),
            VirtualKeyCode::F4 => Some(KeyInject::ControlKey(Code::F4, Key::Named(NamedKey::F4))),
            VirtualKeyCode::F5 => Some(KeyInject::ControlKey(Code::F5, Key::Named(NamedKey::F5))),
            VirtualKeyCode::F6 => Some(KeyInject::ControlKey(Code::F6, Key::Named(NamedKey::F6))),
            VirtualKeyCode::F7 => Some(KeyInject::ControlKey(Code::F7, Key::Named(NamedKey::F7))),
            VirtualKeyCode::F8 => Some(KeyInject::ControlKey(Code::F8, Key::Named(NamedKey::F8))),
            VirtualKeyCode::F9 => Some(KeyInject::ControlKey(Code::F9, Key::Named(NamedKey::F9))),
            VirtualKeyCode::F10 => Some(KeyInject::ControlKey(Code::F10, Key::Named(NamedKey::F10))),
            VirtualKeyCode::F11 => Some(KeyInject::ControlKey(Code::F11, Key::Named(NamedKey::F11))),
            VirtualKeyCode::F12 => Some(KeyInject::ControlKey(Code::F12, Key::Named(NamedKey::F12))),

            _ => None,
        };

        let Some(entry) = inject else {
            return false;
        };

        // Push the queued event on key-down; on key-up just claim the event so
        // the host doesn't pick the release up as a separate accelerator.
        if is_down {
            self.key_inject
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(entry);
        }
        true
    }

    fn state_changed(&self) {
        self.param_registry.flush_all();
        self.emit_parameters_changed_event
            .store(true, Ordering::Relaxed);
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        // Push the new value into the registry's signals — observers bound via
        // `Binding::new` wake up and rebuild. Also flag a `ParametersChanged`
        // idle event for widgets that rely on the older notification path.
        self.param_registry.flush_all();
        self.emit_parameters_changed_event
            .store(true, Ordering::Relaxed);
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
        self.param_registry.flush_all();
        self.emit_parameters_changed_event
            .store(true, Ordering::Relaxed);
    }
}

impl Drop for ViziaEditorHandle {
    fn drop(&mut self) {
        self.vizia_state.open.store(false, Ordering::Release);
        self.param_registry.clear_signals();
    }
}
