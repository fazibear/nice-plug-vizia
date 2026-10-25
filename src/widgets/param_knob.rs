//! Rotary parameter control with host gestures and optional Alt-click value entry.
use super::param_base::ParamWidgetBase;
use nice_plug_core::params::Param;
use vizia::prelude::*;
mod events;

/// A parameter-aware rotary control. Drag vertically; hold Shift for fine edits.
/// Wheel and arrow keys step the value. Double-click/right-click/command-click
/// reset to default. Alt-click opens formatted value entry (Enter accepts,
/// Escape cancels). Display follows modulation; edits start from the base value.
/// Style `param-knob knob` and its normal Vizia knob children to customize artwork.
pub struct ParamKnob {
    base: ParamWidgetBase,
    entry: SyncSignal<bool>,
    allow_entry: bool,
    scroll: bool,
    drag: Option<Drag>,
    scrolled_lines: f32,
}
#[derive(Clone, Copy)]
struct Drag {
    y: f32,
    value: f32,
}
impl Drag {
    fn update(&mut self, y: f32, scale: f32, fine: bool) -> f32 {
        let delta = (self.y - y) / scale / 200.0 * if fine { 0.1 } else { 1.0 };
        self.y = y;
        self.value = (self.value + delta).clamp(0.0, 1.0);
        self.value
    }
}
#[derive(Debug)]
enum KnobEvent {
    Submit(String),
    Cancel,
}
impl ParamKnob {
    pub fn new<'c, 'p, P>(cx: &'c mut Context, param: &'p P) -> Handle<'c, Self>
    where
        'p: 'c,
        P: Param + 'static,
    {
        let base = ParamWidgetBase::new(cx, param);
        let entry = SyncSignal::new(false);
        let value = base.modulated_signal(cx);
        let unmodulated = base.unmodulated_signal(cx);
        Self {
            base,
            entry,
            allow_entry: true,
            scroll: true,
            drag: None,
            scrolled_lines: 0.0,
        }
        .build(
            cx,
            ParamWidgetBase::build_view(param, move |cx, _| {
                Binding::new(cx, entry, move |cx| {
                    if entry.get() {
                        let display = Memo::new(move |_| {
                            base.normalized_value_to_string(unmodulated.get(), true)
                        });
                        Textbox::new(cx, display)
                            .class("value-entry")
                            .on_submit(|cx, text, success| {
                                cx.emit(if success {
                                    KnobEvent::Submit(text)
                                } else {
                                    KnobEvent::Cancel
                                });
                            })
                            .on_cancel(|cx| cx.emit(KnobEvent::Cancel))
                            .on_build(|cx| {
                                cx.emit(TextEvent::StartEdit);
                                cx.emit(TextEvent::SelectAll);
                            })
                            .width(Stretch(1.0))
                            .height(Stretch(1.0));
                    } else {
                        // All interaction belongs to the outer parameter view, not
                        // the decorative knob, so every edit gets a host gesture.
                        Knob::new(cx, base.default_normalized_value(), value, false)
                            .hoverable(false)
                            .focusable(false)
                            .navigable(false)
                            .width(Stretch(1.0))
                            .height(Stretch(1.0));
                    }
                });
            }),
        )
        .focusable(true)
        .navigable(true)
        .role(Role::Slider)
        .name(base.name().to_owned())
        .numeric_value(value.map(|value| f64::from(*value) * 100.0))
    }
    fn end_drag(&mut self, cx: &mut EventContext) {
        if self.drag.take().is_some() {
            self.base.end_set_parameter(cx);
            cx.release();
            cx.set_active(false);
        }
    }
    fn set_discrete(&mut self, cx: &mut EventContext, value: f32) {
        self.end_drag(cx);
        self.base.begin_set_parameter(cx);
        self.base.set_normalized_value(cx, value.clamp(0.0, 1.0));
        self.base.end_set_parameter(cx);
    }
}

pub trait ParamKnobExt {
    /// Disable wheel changes when the knob is inside a scrollable container.
    fn disable_scroll_wheel(self) -> Self;
    /// Disable Alt-click text entry, leaving normal drag/reset interaction intact.
    fn disable_text_entry(self) -> Self;
}
impl ParamKnobExt for Handle<'_, ParamKnob> {
    fn disable_scroll_wheel(self) -> Self {
        self.modify(|knob| knob.scroll = false)
    }
    fn disable_text_entry(self) -> Self {
        self.modify(|knob| knob.allow_entry = false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_drag_clamps_and_is_scale_independent() {
        let mut normal = Drag {
            y: 100.0,
            value: 0.5,
        };
        assert!((normal.update(80.0, 1.0, false) - 0.6).abs() < 1e-6);
        let mut hidpi = Drag {
            y: 200.0,
            value: 0.5,
        };
        assert_eq!(normal.value, hidpi.update(160.0, 2.0, false));
        assert_eq!(normal.update(-1000.0, 1.0, false), 1.0);
        assert!(normal.update(-990.0, 1.0, false) < 1.0);
        assert_eq!(normal.update(1000.0, 1.0, false), 0.0);
    }
    #[test]
    fn widget_builds_with_parameter_registry_and_options() {
        use crate::widgets::param_registry::ParamRegistry;
        use nice_plug_core::params::{FloatParam, range::FloatRange};
        let parameter = FloatParam::new("Test", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 });
        let mut cx = Context::new();
        let registry = ParamRegistry::new();
        registry.clone().build(&mut cx);
        let value = registry.unmodulated(parameter.as_ptr());
        ParamKnob::new(&mut cx, &parameter)
            .disable_scroll_wheel()
            .disable_text_entry()
            .modify(|knob| {
                assert!(!knob.scroll);
                assert!(!knob.allow_entry);
                assert_eq!(knob.base.default_normalized_value(), 0.5);
                assert!(knob.drag.is_none());
            });
        // SAFETY: simulate a host update while the parameter and registry are alive.
        unsafe {
            parameter.as_ptr()._internal_set_normalized_value(0.75);
        }
        registry.flush_all();
        assert_eq!(value.get(), 0.75);
    }
    #[test]
    fn shift_can_change_mid_drag_without_a_jump() {
        let mut drag = Drag {
            y: 100.0,
            value: 0.5,
        };
        let value = drag.update(80.0, 1.0, false);
        assert_eq!(drag.update(80.0, 1.0, true), value);
        assert!((drag.update(60.0, 1.0, true) - 0.61).abs() < 1e-6);
    }
}
