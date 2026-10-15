//! The accessibility display settings a translucent surface has to answer to.
//!
//! # What this crate does and does not do
//!
//! **This crate reads these settings. It does not act on them for you.**
//!
//! *Reduce Transparency* asks for a surface that is not see-through. There is
//! no way to satisfy it from inside `GlassSurface`: `NSGlassEffectView` has
//! no opacity control, and its whole function is to sample and blur what is
//! behind the window. Honouring the setting means **not using the material at
//! all** for the duration — substituting an opaque view, with the colours,
//! contrast and layout that go with it. That substitute is the caller's
//! content, not something a wrapper around one AppKit view can synthesise.
//!
//! This module makes the check one call; building the opaque alternative is
//! the caller's job.
//!
//! # The shape a caller wants
//!
//! Decide at construction, and re-decide when it changes:
//!
//! ```no_run
//! # #[cfg(target_os = "macos")] {
//! if macos_liquid_glass::accessibility::reduce_transparency() {
//!     // Build the opaque variant — no GlassSurface.
//! } else {
//!     // Build the glass one.
//! }
//! # }
//! ```
//!
//! These settings change while the app runs, and unlike the Icon & widget style
//! there IS a notification for them:
//! `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification`, posted on
//! `NSWorkspace.sharedWorkspace().notificationCenter()`. Observe it the same way
//! you would any other. (Note that is the *workspace's* centre, not
//! `NSNotificationCenter.default` — a common way to register an observer that
//! never fires.) [`dim_flashing_lights`] is the exception: it is
//! MediaAccessibility's setting, not `NSWorkspace`'s, and its change is
//! `kMADimFlashingLightsChangedNotification` on the *local* CoreFoundation
//! notification centre, which the SDK header names.
//!
//! # Also worth honouring
//!
//! [`increase_contrast`] asks for stronger borders and less subtle colour
//! separation, which matters for a material whose edges are defined by a faint
//! rim. [`differentiate_without_color`] asks that colour never be the only
//! carrier of meaning — relevant if you drive anything off
//! `WidgetStyle::tint`. [`reduce_motion`] and [`dim_flashing_lights`] are for
//! a caller that animates or flashes: a blinking cursor, a visual bell.

use objc2_app_kit::NSWorkspace;

/// Whether the user asked for **Reduce Transparency**.
///
/// System Settings ▸ Accessibility ▸ Display ▸ Reduce transparency.
///
/// When this is `true`, a Liquid Glass surface is the wrong choice and the
/// caller should build an opaque variant instead — see the module docs for why
/// this crate reports the setting rather than acting on it.
#[must_use]
pub fn reduce_transparency() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
}

/// Whether the user asked for **Increase Contrast**.
///
/// System Settings ▸ Accessibility ▸ Display ▸ Increase contrast. Implies
/// stronger borders and less reliance on subtle tonal separation, which a
/// glass surface leans on heavily.
#[must_use]
pub fn increase_contrast() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldIncreaseContrast()
}

/// Whether the user asked that colour not be the sole carrier of meaning.
///
/// System Settings ▸ Accessibility ▸ Display ▸ Differentiate without color.
#[must_use]
pub fn differentiate_without_color() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldDifferentiateWithoutColor()
}

/// Whether the user asked for **Reduce Motion**.
///
/// System Settings ▸ Accessibility ▸ Display ▸ Reduce motion. This crate
/// animates nothing, so it is here for callers that do.
#[must_use]
pub fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Whether the user asked to **Dim flashing lights**.
///
/// System Settings ▸ Accessibility ▸ Motion ▸ Dim flashing lights, offered on
/// Macs with Apple silicon. MediaAccessibility's `MADimFlashingLightsEnabled`,
/// which `MAVideoAccommodations.h` describes as the setting "to reduce
/// flashing or strobing lights in video content". This crate draws nothing
/// that flashes, so it is here for callers that do.
///
/// **Looked up at run time, not linked.** The function is macOS 13.3's, and
/// a library crate cannot weak-link it for its consumers on stable Rust: a
/// strong reference would stop every binary built on this crate loading on
/// anything older, where the rest of this crate runs. So
/// the first call opens MediaAccessibility and looks the symbol up, once;
/// every call after that is one call through the pointer it found. Where the
/// framework or the symbol is absent the setting does not exist, and this
/// reads `false`.
///
/// Read, not observed. Whether a read follows a change made while the
/// process runs, without the process receiving
/// `kMADimFlashingLightsChangedNotification`, is not established here;
/// toggling the setting under a running caller is the check.
#[must_use]
pub fn dim_flashing_lights() -> bool {
    match dim_flashing_lights_fn() {
        // SAFETY: the pointer is `MADimFlashingLightsEnabled`, which the
        // header declares as taking nothing and returning a C `bool`.
        Some(f) => unsafe { f() },
        None => false,
    }
}

/// `MADimFlashingLightsEnabled`, found on first use and kept; `None` on a
/// system without it.
fn dim_flashing_lights_fn() -> Option<unsafe extern "C" fn() -> bool> {
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::OnceLock;

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    // <dlfcn.h>.
    const RTLD_LAZY: c_int = 0x1;
    const RTLD_LOCAL: c_int = 0x4;

    static FOUND: OnceLock<Option<unsafe extern "C" fn() -> bool>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let path = c"/System/Library/Frameworks/MediaAccessibility.framework/MediaAccessibility";
        // SAFETY: the path is a NUL-terminated literal. The handle is never
        // closed, so what it maps stays mapped for the life of the process.
        let handle = unsafe { dlopen(path.as_ptr(), RTLD_LAZY | RTLD_LOCAL) };
        if handle.is_null() {
            return None;
        }
        // SAFETY: `handle` is live, from `dlopen` above; the name is a
        // NUL-terminated literal.
        let symbol = unsafe { dlsym(handle, c"MADimFlashingLightsEnabled".as_ptr()) };
        if symbol.is_null() {
            return None;
        }
        // SAFETY: a non-null address `dlsym` returns for a function is that
        // function, here the header's `bool (void)`; C's `bool` is Rust's.
        Some(unsafe { core::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> bool>(symbol) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first four are public AppKit properties on `NSWorkspace`, so this
    /// is really a check that the selectors resolve and return without raising
    /// — the values themselves depend on the running machine's settings. Dim
    /// flashing lights is MediaAccessibility's, looked up at run time.
    #[test]
    fn every_display_option_is_readable() {
        let _ = reduce_transparency();
        let _ = increase_contrast();
        let _ = differentiate_without_color();
        let _ = reduce_motion();
        let _ = dim_flashing_lights();
    }

    /// **The lookup finds the function where it exists**, so a `false` from
    /// [`dim_flashing_lights`] is the setting and not a lookup that failed,
    /// which would read exactly like the setting being off. Every macOS this
    /// test runs on in CI is 13.3 or later, where the symbol must resolve;
    /// below 13.3 it fails by design, meaning the OS is older, not that the
    /// lookup broke.
    #[test]
    fn dim_flashing_lights_resolves_where_macos_has_it() {
        assert!(
            dim_flashing_lights_fn().is_some(),
            "MADimFlashingLightsEnabled did not resolve"
        );
    }
}
