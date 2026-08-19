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
//! never fires.)
//!
//! # Also worth honouring
//!
//! [`increase_contrast`] asks for stronger borders and less subtle colour
//! separation, which matters for a material whose edges are defined by a faint
//! rim. [`differentiate_without_color`] asks that colour never be the only
//! carrier of meaning — relevant if you drive anything off
//! `WidgetStyle::tint`.

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

#[cfg(test)]
mod tests {
    use super::*;

    /// All five are public AppKit properties on `NSWorkspace`, so this is
    /// really a check that the selectors resolve and return without raising —
    /// the values themselves depend on the running machine's settings.
    #[test]
    fn every_display_option_is_readable() {
        let _ = reduce_transparency();
        let _ = increase_contrast();
        let _ = differentiate_without_color();
        let _ = reduce_motion();
    }
}
