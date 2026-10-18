//! The standard materials: an `NSVisualEffectView` surface.
//!
//! Liquid Glass is one of two kinds of material Apple ships. The other is the
//! set of *standard* materials an `NSVisualEffectView` draws, and Apple puts
//! those, not glass, behind an app's content. The Human Interface Guidelines'
//! *Materials* page: "Don't use Liquid Glass in the content layer", and "use
//! Standard materials for elements in the content layer, such as app
//! backgrounds" (<https://developer.apple.com/design/human-interface-guidelines/materials>).
//! The `glass` module says the same of itself: a window whose body is text
//! puts glass on its chrome, not behind its text. This module is the surface
//! for that text.
//!
//! # Example
//!
#![cfg_attr(feature = "window", doc = "```no_run")]
#![cfg_attr(not(feature = "window"), doc = "```ignore")]
//! use macos_liquid_glass::material::{Material, MaterialSurface};
//! use macos_liquid_glass::window::GlassWindow;
//! use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
//!
//! let mtm = MainThreadMarker::new().expect("main thread");
//! let size = NSSize::new(560.0, 360.0);
//! let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);
//!
//! // The window paints nothing, so the material shows what is behind it.
//! let window = GlassWindow::new(mtm, size, "example");
//! let body = MaterialSurface::new(mtm, frame, Material::UnderWindowBackground);
//! window.set_content_view(body.view());
//! window.show();
//! ```
//!
//! # Which material
//!
//! Apple's rule is to "always pick a material based on its intended use"
//! (<https://developer.apple.com/documentation/appkit/nsvisualeffectview>), and
//! [`Material`] is named for each one's use rather than for how it looks:
//!
//! | variant | AppKit | Apple's words |
//! |---|---|---|
//! | [`Material::UnderWindowBackground`] | `.underWindowBackground` | used with behind-window blending "to create a sense of peeking through the back of the window" |
//! | [`Material::WindowBackground`] | `.windowBackground` | "The material for the background of opaque windows." |
//! | [`Material::ContentBackground`] | `.contentBackground` | "The material for the background of opaque content." |
//! | [`Material::Sidebar`] | `.sidebar` | "The material for the background of window sidebars." |
//!
//! Only the first is see-through: the window background and content
//! background materials are opaque fills.
//!
//! # Blending and the inactive window
//!
//! The surface blends **behind the window**, "with the contents behind the
//! window, such as the desktop or other windows"
//! (<https://developer.apple.com/documentation/appkit/nsvisualeffectview/blendingmode-swift.enum/behindwindow>),
//! which is what makes it a window body rather than a panel inside one. It
//! is the header's default (`NSVisualEffectView.h:100`), set explicitly so
//! the surface does not depend on it.
//!
//! Its state **follows the window**, also the default (`NSVisualEffectView.h:103`):
//! the material appears "active when the window is active, and inactive when
//! it is not"
//! (<https://developer.apple.com/documentation/appkit/nsvisualeffectview/state-swift.enum>).
//! The HIG's *Windows* page is why that matters: "inactive windows don't use
//! vibrancy ..., which makes them appear subdued", and a custom
//! implementation has to "do this work yourself"
//! (<https://developer.apple.com/design/human-interface-guidelines/windows>).
//! A standard material does it for you; [`MaterialState`] pins either look
//! for the rare surface that should not.
//!
//! # Reduce Transparency
//!
//! Unlike glass, a standard material is a system material that "system
//! settings can change" (HIG, *Materials*). What the user asked of the app
//! itself is unchanged, though: under **Reduce Transparency** an app should
//! "use only opaque windows"
//! (<https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayshouldreducetransparency>),
//! so content drawn translucently over the material is still the caller's to
//! make opaque. [`crate::accessibility::reduce_transparency`] reads the
//! setting.
//!
//! # Content
//!
//! `NSVisualEffectView` has no `contentView`. Content goes through
//! [`MaterialSurface::set_content_view`], which adds it as the surface's one
//! subview, sized to the surface and resized with it. The surface itself
//! draws nothing but the material; the header warns that its mask "does not
//! also mask subviews" (`NSVisualEffectView.h:106`).
//!
//! # Availability
//!
//! Every material here is `API_AVAILABLE(macos(10.14))` or older
//! (`NSVisualEffectView.h:33-57`), so unlike the `glass` module there is no
//! runtime check and no error: the class exists on every macOS this crate
//! builds for.

use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView,
};
use objc2_foundation::{MainThreadMarker, NSRect};

/// Which standard material the surface draws, named for its use.
///
/// `#[non_exhaustive]`: AppKit has more semantic materials than these four
/// (menus, popovers, sheets, HUD windows), and adding one later is not a
/// breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Material {
    /// "The material used under window backgrounds" (`NSVisualEffectView.h:56`),
    /// which with behind-window blending gives "a sense of peeking through the
    /// back of the window". The see-through one.
    UnderWindowBackground,
    /// "The material used by opaque window backgrounds"
    /// (`NSVisualEffectView.h:41`). Shows nothing of what is behind the
    /// window; Apple notes it "supports Desktop Tinting in Dark Mode".
    WindowBackground,
    /// "The material used as the opaque background of content (e.g., by
    /// NSScrollView, NSTableView, NSCollectionView, etc.)"
    /// (`NSVisualEffectView.h:53`).
    ContentBackground,
    /// "The material used in the background of window sidebars"
    /// (`NSVisualEffectView.h:32`).
    Sidebar,
}

impl From<Material> for NSVisualEffectMaterial {
    fn from(m: Material) -> Self {
        match m {
            Material::UnderWindowBackground => NSVisualEffectMaterial::UnderWindowBackground,
            Material::WindowBackground => NSVisualEffectMaterial::WindowBackground,
            Material::ContentBackground => NSVisualEffectMaterial::ContentBackground,
            Material::Sidebar => NSVisualEffectMaterial::Sidebar,
        }
    }
}

impl Material {
    /// The crate's variant for an AppKit material, or `None` for one it does
    /// not name. The AppKit type is an open newtype over `NSInteger`.
    fn of(m: NSVisualEffectMaterial) -> Option<Self> {
        match m {
            NSVisualEffectMaterial::UnderWindowBackground => Some(Self::UnderWindowBackground),
            NSVisualEffectMaterial::WindowBackground => Some(Self::WindowBackground),
            NSVisualEffectMaterial::ContentBackground => Some(Self::ContentBackground),
            NSVisualEffectMaterial::Sidebar => Some(Self::Sidebar),
            _ => None,
        }
    }
}

/// When the material takes its active look.
///
/// [`MaterialState::FollowsWindow`] is AppKit's default and the one the HIG
/// asks for: an inactive window looks subdued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MaterialState {
    /// "Use the active look only when the containing window is active"
    /// (`NSVisualEffectView.h:81`).
    #[default]
    FollowsWindow,
    /// "Use the active look always" (`NSVisualEffectView.h:84`).
    Active,
    /// "Use the inactive look always" (`NSVisualEffectView.h:87`).
    Inactive,
}

impl From<MaterialState> for NSVisualEffectState {
    fn from(s: MaterialState) -> Self {
        match s {
            MaterialState::FollowsWindow => NSVisualEffectState::FollowsWindowActiveState,
            MaterialState::Active => NSVisualEffectState::Active,
            MaterialState::Inactive => NSVisualEffectState::Inactive,
        }
    }
}

impl MaterialState {
    fn of(s: NSVisualEffectState) -> Option<Self> {
        match s {
            NSVisualEffectState::FollowsWindowActiveState => Some(Self::FollowsWindow),
            NSVisualEffectState::Active => Some(Self::Active),
            NSVisualEffectState::Inactive => Some(Self::Inactive),
            _ => None,
        }
    }
}

/// One standard-material surface, blending behind the window, hosting its
/// content as its one subview.
///
/// Installed the way a `glass::GlassSurface` is: as the window's
/// content view, with the caller's content inside it.
///
/// # Cloning
///
/// [`Clone`] is a **retain, not a copy** — every clone addresses the one
/// surface.
#[must_use = "a MaterialSurface does nothing until it is installed as a content view; \
              dropping it releases the underlying NSVisualEffectView"]
#[derive(Debug)]
pub struct MaterialSurface {
    view: Retained<NSVisualEffectView>,
}

impl Clone for MaterialSurface {
    /// Retain the surface, increasing its reference count.
    ///
    /// Both handles address the same `NSVisualEffectView`.
    fn clone(&self) -> Self {
        Self {
            view: self.view.clone(),
        }
    }
}

impl MaterialSurface {
    /// A surface drawing `material`, blending behind the window, its state
    /// following the window's.
    ///
    /// Infallible: `NSVisualEffectView` and every [`Material`] predate the
    /// oldest macOS this crate builds for.
    pub fn new(mtm: MainThreadMarker, frame: NSRect, material: Material) -> Self {
        let view = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
        view.setMaterial(material.into());
        view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        view.setState(MaterialState::FollowsWindow.into());
        view.setWantsLayer(true);
        Self { view }
    }

    /// Put a view on the material, replacing the one there before.
    ///
    /// The view is sized to the surface and resized with it. This is the one
    /// way content goes on the surface: a sibling laid over the top is not on
    /// the material at all.
    pub fn set_content_view(&self, content: &NSView) {
        if let Some(old) = self.content_view() {
            if &*old == content {
                return;
            }
            old.removeFromSuperview();
        }
        content.setFrame(self.view.bounds());
        content.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        self.view.addSubview(content);
    }

    /// The view on the material, if any: the surface's one subview.
    #[must_use]
    pub fn content_view(&self) -> Option<Retained<NSView>> {
        self.view.subviews().lastObject()
    }

    /// Change the material after construction.
    pub fn set_material(&self, material: Material) {
        self.view.setMaterial(material.into());
    }

    /// The material, read back from the view.
    ///
    /// `None` if the view reports one this crate does not name.
    #[must_use]
    pub fn material(&self) -> Option<Material> {
        Material::of(self.view.material())
    }

    /// Pin the active or inactive look, or follow the window again.
    pub fn set_state(&self, state: MaterialState) {
        self.view.setState(state.into());
    }

    /// When the material takes its active look, read back from the view.
    ///
    /// `None` if the view reports a state this crate does not name.
    #[must_use]
    pub fn state(&self) -> Option<MaterialState> {
        MaterialState::of(self.view.state())
    }

    /// Whether the surface blends with what is behind the window, which it
    /// is built to and never changes. AppKit "may fall back to a more
    /// appropriate blending mode as needed" for a material that does not
    /// support it (`NSVisualEffectView.h:100`), so this reads the view.
    #[must_use]
    pub fn blends_behind_window(&self) -> bool {
        self.view.blendingMode() == NSVisualEffectBlendingMode::BehindWindow
    }

    /// The surface as a plain view, for installing into a window.
    ///
    /// For installing, and for reading view geometry — content goes through
    /// [`set_content_view`](MaterialSurface::set_content_view).
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each variant reaches the AppKit value the 27.0 header gives it, and
    /// comes back as itself.
    #[test]
    fn each_material_maps_to_the_headers_value_and_back() {
        for (m, raw) in [
            (Material::UnderWindowBackground, 21),
            (Material::WindowBackground, 12),
            (Material::ContentBackground, 18),
            (Material::Sidebar, 7),
        ] {
            let ns = NSVisualEffectMaterial::from(m);
            assert_eq!(ns.0, raw, "{m:?}");
            assert_eq!(Material::of(ns), Some(m), "{m:?}");
        }
        assert_eq!(Material::of(NSVisualEffectMaterial::Menu), None);
    }

    #[test]
    fn each_state_maps_to_the_headers_value_and_back() {
        for (s, raw) in [
            (MaterialState::FollowsWindow, 0),
            (MaterialState::Active, 1),
            (MaterialState::Inactive, 2),
        ] {
            let ns = NSVisualEffectState::from(s);
            assert_eq!(ns.0, raw, "{s:?}");
            assert_eq!(MaterialState::of(ns), Some(s), "{s:?}");
        }
    }

    /// The default is the header's and the HIG's: follow the window.
    #[test]
    fn the_default_state_follows_the_window() {
        assert_eq!(MaterialState::default(), MaterialState::FollowsWindow);
    }
}
