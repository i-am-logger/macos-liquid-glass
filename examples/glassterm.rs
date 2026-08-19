//! `glassterm` — the glass surface in a **borderless** window.
//!
//! Identical in content to [`glasswin`]; the two differ only in the window they
//! put that content in, which is the point of having both. This one is also the
//! surface every measurement in `MEASUREMENTS.md` was taken against, so its
//! rendering must not change casually.
//!
//! Borderless costs real behaviour — minimise and double-click-titlebar both do
//! nothing, because they live in `NSThemeFrame`. See
//! `macos_liquid_glass::window::GlassWindow::borderless`.
//!
//! [`glasswin`]: ../glasswin/index.html

#[path = "common/mod.rs"]
mod common;

fn main() {
    common::run(common::Identity {
        name: "glassterm",
        borderless: true,
    });
}
