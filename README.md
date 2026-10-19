# nice-plug-vizia

A replacement for the old `nih-plug-vizia` which updates it to the latest version of `vizia`.

This crate allows for the use of the Vizia GUI library with the [nice-plug](https://codeberg.org/BillyDM/nice-plug.git) (formerly NIH-plug) plugin framework.

## Backend

Unlike the upstream `vizia_baseview` (which is coupled to baseview 0.2.x and a git-fork of baseview), this crate:

- Merges the `vizia_baseview` windowing/rendering backend in as `src/backend/`, split into the same modules (`proxy`, `application`, `window`).
- Drives vizia from the **main published [`baseview`](https://crates.io/crates/baseview) 0.4.x** directly, so `EditorHandle::Window` is a real `baseview::Window` exposing the window-control API nice-plug needs.
- Uses **vizia without its `baseview` feature**; the `ViziaEditor` is type-erased (a concrete, non-generic `Plugin::Editor`).

## Slider layout compatibility

The newer Vizia revision sets `ZStack` to `size: auto` by default. `ParamSlider` explicitly applies `.size(Stretch(1.0))` to its inner `ZStack` so the value label and fill bar occupy the slider's bounds. Without this, the inner content can collapse to zero size while the outer slider still responds to hover. This accommodates a Vizia theme change, rather than a baseview backend change.

## Building Examples

```
cargo +nightly xtask bundle gain_gui --release
```

The outputs will be placed in the `target\bundled` directory.
