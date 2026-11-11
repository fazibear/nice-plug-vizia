# nice-plug-vizia

A replacement for the old `nih-plug-vizia` which updates it to the latest version of `vizia`.

This crate allows for the use of the Vizia GUI library with the [nice-plug](https://codeberg.org/BillyDM/nice-plug.git) (formerly NIH-plug) plugin framework.

## Backend

Like upstream `vizia_baseview`, this crate uses published baseview 0.4.x. It keeps a local backend for nice-plug integration:

- Merges the `vizia_baseview` windowing/rendering backend in as `src/backend/`, split into the same modules (`proxy`, `application`, `window`).
- Drives vizia from the **main published [`baseview`](https://crates.io/crates/baseview) 0.4.x** directly, so `EditorHandle::Window` is a real `baseview::Window` exposing the window-control API nice-plug needs.
- Uses **vizia without its `baseview` feature**; the `ViziaEditor` is type-erased (a concrete, non-generic `Plugin::Editor`).

## Slider layout compatibility

The newer Vizia revision sets `ZStack` to `size: auto` by default. `ParamSlider` explicitly applies `.size(Stretch(1.0))` to its inner `ZStack` so the value label and fill bar occupy the slider's bounds. Without this, the inner content can collapse to zero size while the outer slider still responds to hover. This accommodates a Vizia theme change, rather than a baseview backend change.

## Parameter knobs

`widgets::ParamKnob` binds a rotary control directly to a nice-plug parameter:

```rust,ignore
use nice_plug_vizia::widgets::{ParamKnob, ParamKnobExt};

ParamKnob::new(cx, &params.gain).class("gain-dial");
// Optional for controls in scrollable containers:
ParamKnob::new(cx, &params.mix).disable_scroll_wheel();
```

Vertical dragging adjusts the unmodulated value; Shift provides fine adjustment.
The indicator follows host automation and modulation. Wheel and arrow keys use
parameter-aware steps. Double-click, right-click, or command/control-click resets
to default. Alt-click opens value entry using the parameter's own formatter and
parser; Enter accepts, Escape cancels, and invalid input stays editable.
Every edit emits matching host begin/change/end events. Value entry can be
turned off with `.disable_text_entry()`.

Style `param-knob` for size (80×80 by default), and its Vizia `knob`, `.knob-head`,
`.knob-track`, and `.knob-tick` children for appearance. Hide the track and make
the head transparent to overlay an existing background image. `.value-entry`
styles the optional text field.

## Building Examples

```
cargo +nightly xtask bundle gain_gui --release
```

The outputs will be placed in the `target\bundled` directory.
