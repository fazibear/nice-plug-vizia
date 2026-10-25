use super::*;
use crate::widgets::util::ModifiersExt;

impl View for ParamKnob {
    fn element(&self) -> Option<&'static str> {
        Some("param-knob")
    }
    fn event(&mut self, cx: &mut EventContext, event: &mut Event) {
        if cx.is_disabled() {
            self.end_drag(cx);
            return;
        }
        event.map(|action, meta| match action {
            KnobEvent::Submit(text) => {
                if let Some(value) = self.base.string_to_normalized_value(text) {
                    self.set_discrete(cx, value);
                    self.entry.set(false);
                    cx.focus();
                }
                meta.consume();
            }
            KnobEvent::Cancel => {
                self.entry.set(false);
                cx.focus();
                meta.consume();
            }
        });
        event.map(|window, meta| {
            // Let the textbox handle its own mouse, keyboard and selection events.
            if self.entry.get() {
                return;
            }
            match window {
                WindowEvent::MouseDown(MouseButton::Left)
                | WindowEvent::MouseTripleClick(MouseButton::Left) => {
                    self.end_drag(cx);
                    cx.focus();
                    if self.allow_entry && cx.modifiers().alt() {
                        self.entry.set(true);
                    } else if cx.modifiers().command() {
                        self.set_discrete(cx, self.base.default_normalized_value());
                    } else {
                        self.drag = Some(Drag {
                            y: cx.mouse().cursor_y,
                            value: self.base.unmodulated_normalized_value(),
                        });
                        self.base.begin_set_parameter(cx);
                        cx.capture();
                        cx.set_active(true);
                    }
                    meta.consume();
                }
                WindowEvent::MouseDoubleClick(MouseButton::Left)
                | WindowEvent::MouseDown(MouseButton::Right) => {
                    self.set_discrete(cx, self.base.default_normalized_value());
                    meta.consume();
                }
                WindowEvent::MouseMove(_, y) if self.drag.is_some() => {
                    let value = self.drag.as_mut().unwrap().update(
                        *y,
                        cx.scale_factor(),
                        cx.modifiers().shift(),
                    );
                    self.base.set_normalized_value(cx, value);
                    meta.consume();
                }
                WindowEvent::MouseUp(MouseButton::Left) if self.drag.is_some() => {
                    self.end_drag(cx);
                    meta.consume();
                }
                WindowEvent::MouseScroll(_, y) if self.scroll => {
                    if !y.is_finite() {
                        return;
                    }
                    // Bound work for unusually large platform scroll events.
                    self.scrolled_lines = (self.scrolled_lines + y).clamp(-100.0, 100.0);
                    let mut value = self.base.unmodulated_normalized_value();
                    let mut changed = false;
                    while self.scrolled_lines.abs() >= 1.0 {
                        if self.scrolled_lines > 0.0 {
                            value = self
                                .base
                                .next_normalized_step(value, cx.modifiers().shift());
                            self.scrolled_lines -= 1.0;
                        } else {
                            value = self
                                .base
                                .previous_normalized_step(value, cx.modifiers().shift());
                            self.scrolled_lines += 1.0;
                        }
                        changed = true;
                    }
                    if changed {
                        self.set_discrete(cx, value);
                    }
                    meta.consume();
                }
                WindowEvent::KeyDown(Code::ArrowUp | Code::ArrowRight, _) => {
                    let value = self.base.next_normalized_step(
                        self.base.unmodulated_normalized_value(),
                        cx.modifiers().shift(),
                    );
                    self.set_discrete(cx, value);
                    meta.consume();
                }
                WindowEvent::KeyDown(Code::ArrowDown | Code::ArrowLeft, _) => {
                    let value = self.base.previous_normalized_step(
                        self.base.unmodulated_normalized_value(),
                        cx.modifiers().shift(),
                    );
                    self.set_discrete(cx, value);
                    meta.consume();
                }
                WindowEvent::FocusOut => self.end_drag(cx),
                _ => (),
            }
        });
    }
}
