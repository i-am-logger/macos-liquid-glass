//! Liquid Glass windows on macOS, that follow the system Icon & widget style.
//!
//! macOS 26 introduced `NSGlassEffectView`, the material behind Liquid Glass.
//! Putting a window on it is the easy part. The hard part — and what this crate
//! is actually for — is making that window agree with
//! **System Settings ▸ Appearance ▸ Icon & widget style**, the setting that
//! restyles desktop widgets, and keeping it in agreement as the user changes it.
//!
//! That turns out to involve a preference key with nine tokens behind a
//! four-option UI, a notification that no notification centre posts, and a
//! key-value observer that must never read a preference from inside its own
//! callback. Each of those is documented where it is handled, with the
//! measurement that established it. `MEASUREMENTS.md` in the repository is the
//! long form.
//!
//! # Features
//!
//! | feature | default | what it brings |
//! |---|---|---|
//! | `glass` | yes | the `glass::GlassSurface` wrapper and its availability guard |
//! | `window` | yes | a transparent window hosting one glass surface, titled or borderless |
//! | `icon-style` | yes | the Icon & widget style tracker — usable on its own |
//! | `private-spi` | **no** | two undocumented selectors; see below |
//!
//! [`is_dark`] and [`accessibility`] are behind **no** feature: the first is the
//! crate's only light/dark resolver and both halves need it, and the second is
//! an obligation rather than an option.
//!
//! # Accessibility
//!
//! A translucent surface has to answer to **Reduce Transparency**. This crate
//! reports that setting through [`accessibility`] and does **not** act on it for
//! you — `NSGlassEffectView` has no opacity control, so honouring it means not
//! using the material at all and substituting opaque content, which is the
//! caller's to build. That module states the reasoning in full and shows the
//! shape to write. Check it before you construct a surface.
//!
//! `private-spi` is off because App Store Review Guideline 2.5.1 is "Apps may
//! only use public APIs", with no `respondsToSelector:` exemption — a crate
//! reaching private selectors by default would hand every consumer a submission
//! liability they never opted into. Without it `icon-style` still tracks the
//! style; only `icon_style::WidgetStyle::tint` stops reporting a colour, because
//! the theme colour has no public source.
//!
//! # Example
//!
//! Compiled as a doctest, so it cannot drift from the API the way a README
//! snippet can.
//!
//! The example needs `window`, `glass` and `icon-style` together, so `cfg_attr`
//! picks the opening fence — runnable when all three are on, `ignore` otherwise
//! — which keeps `cargo test` green under every other feature set, including
//! `default-features = false, features = ["icon-style"]`, the standalone-tracker
//! configuration this crate advertises. Verify a change here with `cargo test`,
//! not `cargo check`: `check` does not compile doctests.
//!

#![cfg_attr(
    all(feature = "window", feature = "glass", feature = "icon-style"),
    doc = "```no_run"
)]
#![cfg_attr(
    not(all(feature = "window", feature = "glass", feature = "icon-style")),
    doc = "```ignore"
)]
//! use macos_liquid_glass::glass::{GlassStyle, GlassSurface};
//! use macos_liquid_glass::icon_style::{Reconcile, StyleObserver};
//! use macos_liquid_glass::window::GlassWindow;
//! use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mtm = MainThreadMarker::new().expect("main thread");
//! let size = NSSize::new(560.0, 360.0);
//! let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);
//!
//! let window = GlassWindow::new(mtm, size, "example");
//! let glass = GlassSurface::new(mtm, frame, GlassStyle::Clear, 16.0)?;
//! window.set_content_view(glass.view());
//!
//! // `Clone` retains rather than copies — both handles address the one window.
//! // The closure outlives this scope, so it needs its own.
//! let win = window.clone();
//! let _observer = StyleObserver::new(mtm, Reconcile::default(), move |style| {
//!     // FORCING the appearance, which is right for a surface that follows the
//!     // widget style — it carries its own light/dark, so under "Clear ▸ Dark"
//!     // it stays dark while the system is Light. An ordinary window that
//!     // should merely follow the system wants `None` here instead.
//!     win.set_appearance(style.appearance().as_deref());
//! });
//!
//! window.show();
//! # Ok(())
//! # }
//! ```
//!
//! Keep the `StyleObserver` alive for as long as the window: KVO does not
//! retain its observer, and dropping it is what unregisters.
//!
//! `icon-style` carries no dependency on `window`, so a consumer that already
//! has an `NSWindow` can take the tracker by itself:
//!
//! ```toml
//! macos-liquid-glass = { version = "1.0.0-beta.1", default-features = false, features = ["icon-style"] }
//! ```
//!
//! # Threads
//!
//! Everything here is main-thread-only and none of it is `Send` or `Sync` —
//! `GlassWindow`, `GlassSurface` and `StyleObserver` all hold objects that are
//! `MainThreadOnly`, so the compiler will not let one cross a thread boundary.
//! That is enforcement, not convention: you cannot construct any of them
//! without a `MainThreadMarker`, and you cannot move one to a thread that has
//! no such marker.
//!
//! The observer's callback is delivered on the main thread too, so it may touch
//! AppKit freely. It is `Fn` rather than `FnMut` because it can re-enter — see
//! `icon_style::StyleObserver::new`.
//!
//! [`accessibility`] is the exception, and reads `NSWorkspace`, which is not
//! main-thread-only.
//!
//! # Platform
//!
//! macOS only. On every other target this crate is empty rather than absent, so
//! a cross-platform consumer can depend on it unconditionally and gate at the
//! call site. The glass surface additionally requires macOS 26 at *runtime* —
//! `NSGlassEffectView` does not exist before it — which is checked rather than
//! assumed.

#![cfg_attr(docsrs, feature(doc_cfg))]

/// The objc2 crates this crate's own signatures are built from, re-exported at
/// the exact versions it was compiled against.
///
/// Without these a consumer must declare `objc2-foundation` (and `-app-kit`,
/// and `-core-foundation`) themselves and keep them unifying by hand. When they
/// do not unify, cargo reports no version conflict — it compiles both copies and
/// rustc says `expected MainThreadMarker, found MainThreadMarker`, naming the
/// same type twice. Re-exporting makes the right versions reachable by
/// construction; declaring them directly still works and reads better.
///
/// # What this promises
///
/// These are pass-throughs so that versions unify, not a promise to freeze all
/// of AppKit. Items reachable here that this crate's own signatures do not use
/// carry objc2's stability guarantees, not this crate's.
///
/// The example below deliberately references no feature-gated item, so it holds
/// under every feature set — including `default-features = false`, which is
/// exactly the configuration a consumer taking the tracker alone uses.
///
/// ```
/// # #[cfg(target_os = "macos")] {
/// use macos_liquid_glass::objc2_app_kit::NSColor;
/// use macos_liquid_glass::objc2_core_foundation::CGFloat;
/// use macos_liquid_glass::objc2_foundation::{MainThreadMarker, NSSize};
///
/// let _: Option<MainThreadMarker> = MainThreadMarker::new();
/// let _: NSSize = NSSize::new(560.0, 360.0);
/// let _: Option<&NSColor> = None;
/// let _: CGFloat = 16.0;
/// # }
/// ```
#[cfg(target_os = "macos")]
pub use {objc2, objc2_app_kit, objc2_core_foundation, objc2_foundation};

#[cfg(target_os = "macos")]
pub mod accessibility;
#[cfg(all(target_os = "macos", feature = "glass"))]
pub mod glass;
#[cfg(all(target_os = "macos", feature = "icon-style"))]
pub mod icon_style;
#[cfg(all(target_os = "macos", feature = "window"))]
pub mod window;

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
#[cfg(target_os = "macos")]
use objc2_foundation::NSArray;

/// Whether an appearance resolves to Dark.
///
/// Deliberately **not** behind a feature: this is the crate's only light/dark
/// resolver, and a consumer taking `icon-style` alone needs it as much as one
/// taking `window`.
///
/// # Why not compare the name
///
/// `bestMatchFromAppearancesWithNames:` rather than `name() == DarkAqua`,
/// because an effective appearance can be a **vibrant or accessibility
/// variant** whose name is neither `Aqua` nor `DarkAqua`, and only asking for
/// the best match of the two resolves those. Measured on macOS 27.0: for
/// `NSAppearanceNameVibrantDark` a name comparison answers `false` — it calls a
/// dark appearance light — while this answers `true`. That naive comparison is
/// exactly what a consumer writes when this function is out of reach, which is
/// why it is reachable.
///
/// # Which appearance to pass
///
/// The *ambient* one — normally your window's or view's `effectiveAppearance`.
/// This takes it as an argument rather than reading `NSApp` because
/// `NSApplication::sharedApplication` **creates** the application object as a
/// side effect, which a library getter must never do, and because it would then
/// need a `MainThreadMarker` purely to reach an appearance.
#[cfg(target_os = "macos")]
pub fn is_dark(ambient: &NSAppearance) -> bool {
    let names = NSArray::from_slice(&[unsafe { NSAppearanceNameAqua }, unsafe {
        NSAppearanceNameDarkAqua
    }]);
    match ambient.bestMatchFromAppearancesWithNames(&names) {
        Some(best) => &*best == unsafe { NSAppearanceNameDarkAqua },
        None => false,
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use objc2_app_kit::NSAppearanceNameVibrantDark;

    use super::*;

    /// Pins the `bestMatch` algorithm against the name comparison a consumer
    /// reaches for when this resolver is unavailable.
    ///
    /// Kills the mutation "compare `name()` to `DarkAqua`": `VibrantDark` is a
    /// dark appearance whose name is neither Aqua nor DarkAqua, so the naive
    /// version answers `false` here and this test fails.
    #[test]
    fn vibrant_dark_resolves_dark_even_though_its_name_is_neither() {
        let vibrant = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameVibrantDark })
            .expect("VibrantDark is a stock appearance");
        assert_ne!(
            &*vibrant.name(),
            unsafe { NSAppearanceNameDarkAqua },
            "precondition: VibrantDark's name is not DarkAqua, or this proves nothing"
        );
        assert!(is_dark(&vibrant), "VibrantDark is a dark appearance");
    }

    #[test]
    fn the_two_stock_appearances_resolve_to_themselves() {
        for (name, want) in [
            (unsafe { NSAppearanceNameAqua }, false),
            (unsafe { NSAppearanceNameDarkAqua }, true),
        ] {
            let appearance = NSAppearance::appearanceNamed(name).expect("stock appearance");
            assert_eq!(is_dark(&appearance), want, "{name:?}");
        }
    }
}
