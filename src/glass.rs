//! The Liquid Glass material surface.
//!
//! `NSGlassEffectView` arrived in macOS 26. It is a real, public AppKit class —
//! `objc2-app-kit` binds it — but it does not *exist* on anything older, so
//! every entry point here is guarded by [`is_supported`] rather than by a
//! compile-time version check, which Rust has no equivalent of.
//!
//! # Where the material belongs
//!
//! Apple's Liquid Glass guidance puts the material in the layer that floats
//! *above* content — navigation, controls, chrome — and warns against using it
//! for the content layer itself, where it competes with what the user is
//! reading rather than framing it.
//!
//! This crate makes a whole window out of glass, which is the widget/HUD case
//! that guidance permits rather than the general one. If you are building an
//! ordinary document or list window, the material belongs on its chrome, not
//! behind its text. Stated here because a crate this easy to adopt makes the
//! wrong choice as cheap as the right one.
//!
//! # Reduce Transparency
//!
//! A glass surface is the wrong choice when the user has asked for **Reduce
//! Transparency**, and nothing here detects that for you: the material has no
//! opacity control, so honouring the setting means building opaque content
//! instead. Check `macos_liquid_glass::accessibility::reduce_transparency()` before
//! constructing one, and observe
//! `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification` to re-decide.
//!
//! # Never stack glass on glass
//!
//! Content goes inside the surface's `contentView` — the only child with a
//! documented z-order guarantee (`NSGlassEffectView.h:27`: AppKit "only
//! guarantees the `contentView` will be placed inside the glass effect;
//! arbitrary subviews aren't guaranteed specific behavior with regard to
//! z-order") — never as a sibling laid over the top.
//!
//! The rule is about stacking, not about count. `NSGlassEffectContainerView`
//! hosts *several* glass views, merging those within its `spacing` and batching
//! them to cut render passes (`NSGlassEffectView.h:48-64`), so surfaces side by
//! side in a container are a supported pattern. One laid over another is not.
//!
//! This crate wraps the single view, not the container. A caller wanting
//! several surfaces should reach for `NSGlassEffectContainerView` directly —
//! [`GlassSurface::view`] gives them the `NSView` to put inside it.

use core::fmt;

use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::{MainThreadOnly, msg_send};
use objc2_app_kit::{NSColor, NSGlassEffectView, NSGlassEffectViewStyle, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSRect};

/// Which material the surface uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GlassStyle {
    /// The standard frosted material.
    Regular,
    /// The thinner, more transparent material. Pairs with a dimming layer over
    /// bright content — see the HIG's *Materials* guidance.
    Clear,
}

impl From<GlassStyle> for NSGlassEffectViewStyle {
    fn from(s: GlassStyle) -> Self {
        match s {
            GlassStyle::Regular => NSGlassEffectViewStyle::Regular,
            GlassStyle::Clear => NSGlassEffectViewStyle::Clear,
        }
    }
}

/// Why a [`GlassSurface`] could not be created.
///
/// One variant today, and `#[non_exhaustive]` so that a second can be added
/// without a major bump — which an `Option` would have made impossible forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GlassError {
    /// `NSGlassEffectView` is not registered with the Objective-C runtime,
    /// which means this is not macOS 26 or later.
    Unsupported,
}

impl fmt::Display for GlassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str(
                "NSGlassEffectView is not available on this system; \
                 Liquid Glass requires macOS 26 or later",
            ),
        }
    }
}

impl core::error::Error for GlassError {}

/// Whether this system has `NSGlassEffectView` — that is, whether it is macOS
/// 26 or later.
///
/// A cheap pre-flight check. [`GlassSurface::new`] performs it too and reports
/// [`GlassError::Unsupported`] rather than panicking, so this is for deciding
/// *before* you commit to a glass design — not a precondition you must satisfy.
///
/// This is a *runtime* class lookup, not a version comparison. `objc2`'s
/// `extern_class!` bindings resolve their class lazily on first use and panic
/// if it is absent, which is what this exists to get in front of.
#[must_use]
pub fn is_supported() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

/// Reject a frame AppKit would trap on, with a message naming the value.
///
/// Only NaN. Measured on macOS 27.0, against the real class: an **infinite**
/// size is accepted and silently clamped to 2^45, a **negative** origin is
/// stored verbatim (ordinary AppKit — secondary displays sit at negative
/// coordinates), and a negative size is normalised. A NaN in the frame is the
/// one input that traps, and it traps hard: exit 133 (SIGTRAP) with **not one
/// byte** on stderr.
///
/// So this is deliberately narrower than the window's finite check — the two
/// have genuinely different validity domains, and an infinite size that works
/// here aborts `GlassWindow::new`. A predicate wide enough for both would
/// reject working input.
///
/// The corner radius gets no guard at all: NaN and negative were both measured
/// to be stored and to survive a forced display pass.
fn checked_frame(frame: NSRect) -> NSRect {
    assert!(
        !(frame.origin.x.is_nan()
            || frame.origin.y.is_nan()
            || frame.size.width.is_nan()
            || frame.size.height.is_nan()),
        "GlassSurface frame must not contain NaN, got {frame:?}"
    );
    frame
}

/// One Liquid Glass surface, hosting its content inside the material.
///
/// # Cloning
///
/// [`Clone`] is a **retain, not a copy** — every clone addresses the one
/// surface.
#[must_use = "a GlassSurface does nothing until it is installed as a content view; \
              dropping it releases the underlying NSGlassEffectView"]
#[derive(Debug)]
pub struct GlassSurface {
    view: Retained<NSGlassEffectView>,
}

impl Clone for GlassSurface {
    /// Retain the surface, increasing its reference count.
    ///
    /// Both handles address the same `NSGlassEffectView`.
    fn clone(&self) -> Self {
        Self {
            view: self.view.clone(),
        }
    }
}

impl GlassSurface {
    /// Create a surface, or [`GlassError::Unsupported`] on a system without the
    /// material.
    ///
    /// # Errors
    ///
    /// [`GlassError::Unsupported`] if `NSGlassEffectView` is absent — see
    /// [`is_supported`].
    ///
    /// # Panics
    ///
    /// If `frame` contains a NaN. AppKit traps on such a frame inside
    /// `initWithFrame:` with no diagnostic at all; the assertion prints the
    /// frame first. Infinite and negative values are accepted — AppKit clamps
    /// or normalises them.
    #[must_use = "a GlassSurface does nothing until it is installed as a content view"]
    pub fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        style: GlassStyle,
        corner_radius: CGFloat,
    ) -> Result<Self, GlassError> {
        if !is_supported() {
            return Err(GlassError::Unsupported);
        }

        let view =
            NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), checked_frame(frame));

        view.setStyle(style.into());
        view.setCornerRadius(corner_radius);
        view.setWantsLayer(true);

        Ok(Self { view })
    }

    /// Put a view inside the material.
    ///
    /// This is the *only* supported way to place content on glass. A view added
    /// as a sibling above the surface has no z-order guarantee against it.
    pub fn set_content_view(&self, content: &NSView) {
        self.view.setContentView(Some(content));
    }

    /// Tint the material **toward** a colour.
    ///
    /// AppKit's own words: "the color the glass effect view uses to tint the
    /// background and glass effect toward" (`NSGlassEffectView.h:33`). A bias,
    /// not a fill — but it saturates hard, and pushed far it floods the surface
    /// to a flat slab rather than tinting it.
    ///
    /// A dark, desaturated tint reads as a darkening that scales the channels
    /// together without shifting hue; a saturated one moves the hue.
    ///
    /// The colour is copied when set, so later mutation of `tint` has no
    /// effect.
    pub fn set_tint_color(&self, tint: Option<&NSColor>) {
        self.view.setTintColor(tint);
    }

    /// The material's tint colour, if one is set.
    #[must_use]
    pub fn tint_color(&self) -> Option<Retained<NSColor>> {
        self.view.tintColor()
    }

    /// The corner curvature applied to all four corners.
    #[must_use]
    pub fn corner_radius(&self) -> CGFloat {
        self.view.cornerRadius()
    }

    /// Change the corner curvature.
    ///
    /// Takes effect on an already-displayed surface.
    ///
    /// # Concentricity
    ///
    /// If you round the corners of anything *inside* the surface, its radius
    /// should be this one minus the inset between them, so the two curves stay
    /// concentric. A nested corner that reuses the outer radius reads as
    /// visibly tighter than the surface containing it.
    pub fn set_corner_radius(&self, radius: CGFloat) {
        self.view.setCornerRadius(radius);
    }

    /// The view currently inside the material, if any.
    #[must_use]
    pub fn content_view(&self) -> Option<Retained<NSView>> {
        self.view.contentView()
    }

    /// Whether the material responds visually to being interacted with.
    ///
    /// `None` on a system without the property — it is
    /// `API_AVAILABLE(macos(27.0))`, so macOS 26 does not have it.
    ///
    /// This is **public** API (`NSGlassEffectView.h:45`), not SPI, and needs no
    /// feature gate. It is reached by selector only because `objc2-app-kit`
    /// 0.3.2's bindings were generated against an earlier SDK and do not carry
    /// it yet; verified present at runtime on macOS 27.0 alongside its
    /// underscore-prefixed private twin, which this deliberately does not use.
    #[must_use]
    pub fn effect_is_interactive(&self) -> Option<bool> {
        if !self
            .view
            .respondsToSelector(objc2::sel!(effectIsInteractive))
        {
            return None;
        }
        // SAFETY: the header declares `@property BOOL effectIsInteractive`, so
        // the getter takes no arguments and returns BOOL.
        Some(unsafe { msg_send![&*self.view, effectIsInteractive] })
    }

    /// Turn the interactive response on or off, if this system has it.
    ///
    /// Returns whether the property was actually set. AppKit's guidance is that
    /// it "should be enabled for glass that is used as the background for
    /// interactive controls or when used as the container of interactive
    /// controls" (`NSGlassEffectView.h:41`) — which is precisely what a surface
    /// hosting a caller's content usually is, so this is worth setting rather
    /// than leaving at its default.
    ///
    /// A plain `bool` rather than a `Result`, because there is exactly one
    /// reason it can fail — this system predates the property — and
    /// [`effect_is_interactive`] already reports that as `None`.
    ///
    /// [`effect_is_interactive`]: GlassSurface::effect_is_interactive
    #[must_use = "returns whether the property was actually set; it is absent before macOS 27"]
    pub fn set_effect_interactive(&self, interactive: bool) -> bool {
        if !self
            .view
            .respondsToSelector(objc2::sel!(setEffectIsInteractive:))
        {
            return false;
        }
        // SAFETY: the header declares `@property BOOL effectIsInteractive`, so
        // the setter takes one BOOL and returns void.
        unsafe { msg_send![&*self.view, setEffectIsInteractive: interactive] }
        true
    }

    /// Change the material after construction.
    ///
    /// Takes effect on an already-displayed surface: a window switched from
    /// [`GlassStyle::Clear`] to [`GlassStyle::Regular`] after `show` renders
    /// pixel-identical to one constructed `Regular`.
    pub fn set_style(&self, style: GlassStyle) {
        self.view.setStyle(style.into());
    }

    /// The material's current style, read back from the view.
    ///
    /// `None` if the view reports a style this crate does not know — the AppKit
    /// type is an open newtype over `NSInteger`, so a future material would
    /// arrive as an integer with no variant. Returning the crate's own enum
    /// rather than AppKit's keeps `objc2-app-kit` out of a return type.
    #[must_use]
    pub fn style(&self) -> Option<GlassStyle> {
        match self.view.style() {
            NSGlassEffectViewStyle::Regular => Some(GlassStyle::Regular),
            NSGlassEffectViewStyle::Clear => Some(GlassStyle::Clear),
            _ => None,
        }
    }

    /// The surface as a plain view, for installing into a window.
    ///
    /// For installing, and for reading view geometry — **not** a place to add
    /// subviews. A view added here is a sibling of the material with no z-order
    /// guarantee against it; content goes through [`set_content_view`].
    ///
    /// [`set_content_view`]: GlassSurface::set_content_view
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.view
    }
}
