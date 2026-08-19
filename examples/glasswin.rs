//! `glasswin` — the same glass surface in a **normal titled** window.
//!
//! Identical in content to [`glassterm`]; only the window differs. Here the
//! titlebar is transparent and the title hidden, so it looks the same while
//! behaving like every other macOS window: minimise works, double-clicking the
//! titlebar does whatever the user asked for in System Settings, and the traffic
//! lights are AppKit's own rather than hand-added.
//!
//! [`glassterm`]: ../glassterm/index.html

#[path = "common/mod.rs"]
mod common;

fn main() {
    common::run(common::Identity {
        name: "glasswin",
        borderless: false,
    });
}
