//! `borderless` — the glass surface in a **borderless** window.
//!
//! Identical in content to the other example; the two differ only in which
//! [`GlassWindow`] constructor they call. Run both side by side to see what the
//! window shape costs:
//!
//! ```sh
//! cargo run --example titled
//! cargo run --example borderless
//! ```
//!
//! [`GlassWindow`]: macos_liquid_glass::window::GlassWindow

#[path = "common/mod.rs"]
mod common;

fn main() {
    common::run(common::Identity { borderless: true });
}
