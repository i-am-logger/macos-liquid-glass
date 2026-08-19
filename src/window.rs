//! A transparent window suitable for hosting a glass surface.
//!
//! Everything here exists because a glass window is not an ordinary window: the
//! material has to sample what is behind the window, which means the window
//! itself must paint nothing.
//!
//! Two shapes are available, and the difference is behavioural rather than
//! cosmetic: [`GlassWindow::new`] builds a normal titled window, and
//! [`GlassWindow::borderless`] one with no titlebar at all, which gives up the
//! behaviours tabulated there. A window that paints nothing and has no titlebar
//! is also refused focus by AppKit unless it says otherwise, which this module's
//! `NSWindow` subclass overrides.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSBackingStoreType, NSColor, NSView, NSWindow,
    NSWindowStyleMask, NSWindowTabbingMode, NSWindowTitleVisibility,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

/// Which window shape was built. Internal — callers choose by constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Chrome {
    Borderless,
    #[default]
    Titled,
}

impl Chrome {
    /// The style mask this chrome builds the window with.
    ///
    /// `Borderless` is the constant 0, so `Borderless | Resizable` IS simply
    /// `Resizable` — which is why adding `Miniaturizable` to it changes nothing
    /// useful without `Titled`.
    fn style_mask(self) -> NSWindowStyleMask {
        match self {
            Self::Borderless => NSWindowStyleMask::Borderless | NSWindowStyleMask::Resizable,
            Self::Titled => {
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::FullSizeContentView
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable
            }
        }
    }
}

/// Per-window state that has no `NSWindow` equivalent. Private: publishing it
/// would freeze where per-window state lives.
#[derive(Debug, Default)]
struct WindowState {
    close_on_escape: Cell<bool>,
}

/// Reject a size AppKit would trap on, with a message naming the value.
///
/// Measured on macOS 27.0: `initWithContentRect:` with a non-finite size raises
/// out of Objective-C, and Rust cannot catch a foreign exception — the process
/// dies with `fatal runtime error: Rust cannot catch foreign exceptions` and no
/// indication of which value was bad. A Rust assertion in front of it prints the
/// offending size first, then aborts the same way.
///
/// A panic rather than a `Result`: a non-finite size is a bug in the caller's
/// layout arithmetic, not a condition anything can act on at runtime.
fn checked_size(size: NSSize, what: &str) -> NSSize {
    assert!(
        size.width.is_finite() && size.height.is_finite(),
        "{what}: window size must be finite, got {size:?}"
    );
    size
}

define_class!(
    /// An `NSWindow` that will accept focus without a titlebar.
    ///
    /// `NSWindow` rejects focus when it has no titlebar — the default
    /// `canBecomeKeyWindow` returns NO for a borderless window. Overriding it
    /// is the only reason this subclass exists.
    #[unsafe(super(NSWindow))]
    // No `#[name = ...]`. objc2's own docs: "If you're developing a library, it
    // is recommended that you do not set this, and instead rely on the default
    // naming, since that usually works better with users having multiple
    // SemVer-incompatible versions of your library in the same binary."
    //
    // The name is stamped into four #[export_name] symbols — the registration
    // Once, the ivar offset and the drop-flag offset among them. Two
    // semver-incompatible copies of this crate in one binary would emit
    // identical symbols; the linker merges them, the second copy's registration
    // never runs, its method overrides are absent, and its ivars struct is
    // written into a slot sized for the other one's. Silently. The default name
    // interpolates CARGO_PKG_VERSION precisely to prevent that, and it bites at
    // 0.1-vs-0.2, not only across a major.
    #[thread_kind = MainThreadOnly]
    #[ivars = WindowState]
    #[derive(Debug)]
    struct PaneWindow;

    impl PaneWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }

        /// AppKit sends this on Escape.
        ///
        /// A borderless window has no close button, so `performClose:` has
        /// nothing to act on and merely beeps. Escape is the only close
        /// affordance such a window can offer without drawing its own.
        ///
        /// The delegate is consulted first, which is the part `close()` alone
        /// does not do: `performClose:` sends `windowShouldClose:` and honours a
        /// `NO`, so a consumer that vetoes closing — an unsaved-changes prompt —
        /// would otherwise be silently overruled by the Escape key while its
        /// veto still worked for every other route. `windowShouldClose:` is an
        /// OPTIONAL protocol method, so it is probed rather than sent blind.
        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) {
            if !self.ivars().close_on_escape.get() {
                return;
            }
            // Not a let-chain: those need Rust 1.88 and this crate declares an
            // MSRV of 1.85.
            if let Some(delegate) = self.delegate() {
                if delegate.respondsToSelector(sel!(windowShouldClose:)) {
                    // SAFETY: `respondsToSelector:` confirms the method is
                    // present, and `NSWindowDelegate` fixes its signature — one
                    // `NSWindow *` argument, `BOOL` return.
                    let should: bool = unsafe { msg_send![&*delegate, windowShouldClose: self] };
                    if !should {
                        return;
                    }
                }
            }
            self.close();
        }
    }
);

/// A transparent window that a glass surface can be installed into.
///
/// The window paints nothing itself: `isOpaque = NO` with a clear background,
/// so the material can sample the desktop behind it.
///
/// # Cloning
///
/// [`Clone`] is a **retain, not a copy** — every clone addresses the one
/// window. There is no `Drop` impl and closing is explicit, so a dropped handle
/// releases and nothing else; this is the ordinary Objective-C ownership
/// `Retained` already models.
#[must_use = "a GlassWindow releases its NSWindow when dropped; bind it for as long as the window should live"]
#[derive(Debug)]
pub struct GlassWindow {
    window: Retained<PaneWindow>,
    chrome: Chrome,
}

impl Clone for GlassWindow {
    /// Retain the window, increasing its reference count.
    ///
    /// Both handles address the same `NSWindow`. Useful for handing the window
    /// to a callback without giving up ownership.
    fn clone(&self) -> Self {
        Self {
            window: self.window.clone(),
            chrome: self.chrome,
        }
    }
}

impl GlassWindow {
    /// A normal window that happens to be made of glass.
    ///
    /// Titled, with a transparent titlebar and a hidden title, and its content
    /// extending underneath — so it *looks* like an undecorated glass surface
    /// while behaving like every other macOS window. Use this unless you have a
    /// specific reason not to; see [`borderless`] for what the alternative
    /// costs.
    ///
    /// Not visible until [`show`].
    ///
    /// # Panics
    ///
    /// If `size` is not finite. AppKit raises on such a size from inside
    /// `initWithContentRect:`, and a foreign exception aborts the process
    /// without naming the value; the assertion names it first.
    ///
    /// [`borderless`]: GlassWindow::borderless
    /// [`show`]: GlassWindow::show
    pub fn new(mtm: MainThreadMarker, size: NSSize, title: &str) -> Self {
        Self::with_chrome(mtm, size, title, Chrome::Titled)
    }

    /// A window with no titlebar at all.
    ///
    /// For a surface that genuinely wants no window behaviour — a HUD, a
    /// desktop widget, a measurement target. **It gives up more than
    /// decoration.** Those behaviours live in `NSThemeFrame`, and a borderless
    /// window has no instance of one. Measured on macOS 27.0 (26A5406e):
    ///
    /// | behaviour | borderless | [`new`] |
    /// |---|---|---|
    /// | close | works | works |
    /// | zoom | works | works |
    /// | **minimise** | **does nothing** — and adding `Miniaturizable` to the mask does not help | works |
    /// | **double-click titlebar** | nothing tracks it | works, honouring the user's own System Settings preference |
    /// | **traffic lights** | none; adding them by hand yields buttons with `target = nil`, because `standardWindowButton:forStyleMask:` is given a mask and has no window to point at | AppKit creates and wires them |
    /// | focus | needs the `canBecomeKeyWindow` override this crate applies | free |
    ///
    /// Not visible until [`show`].
    ///
    /// # Panics
    ///
    /// If `size` is not finite — see [`new`].
    ///
    /// [`new`]: GlassWindow::new
    /// [`show`]: GlassWindow::show
    pub fn borderless(mtm: MainThreadMarker, size: NSSize, title: &str) -> Self {
        Self::with_chrome(mtm, size, title, Chrome::Borderless)
    }

    /// The style mask is fixed here and never changed afterwards. Assigning
    /// `styleMask` on a live window rebuilds its frame view and DISCARDS an
    /// installed `NSGlassEffectView`, so the shape has to be decided before the
    /// surface goes in.
    fn with_chrome(mtm: MainThreadMarker, size: NSSize, title: &str, chrome: Chrome) -> Self {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), checked_size(size, "GlassWindow"));
        let style = chrome.style_mask();

        let this = PaneWindow::alloc(mtm).set_ivars(WindowState::default());
        let window: Retained<PaneWindow> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: frame,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        };

        window.setOpaque(false);
        // Clear, not just translucent: the glass must sample what is BEHIND the
        // window, and any background colour composites over that sample.
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setTitle(&NSString::from_str(title));

        match chrome {
            // Nothing drags a borderless window but its background.
            Chrome::Borderless => window.setMovableByWindowBackground(true),
            Chrome::Titled => {
                // Let the material run unbroken to the top of the window
                // instead of stopping beneath an opaque titlebar, and drop the
                // title text — the surface draws its own.
                window.setTitlebarAppearsTransparent(true);
                window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
                // The titlebar already drags; leaving background-drag on as
                // well makes the whole surface draggable, which is wrong for a
                // window that has a real titlebar to grab.
                window.setMovableByWindowBackground(false);
            }
        }

        // Rust owns this window through `Retained`. Without this, closing it
        // would release it out from under us and leave the pointer dangling.
        // SAFETY: the window is not in a document/window-controller hierarchy
        // that expects to own it. Note `close` IS sent — `cancelOperation:`
        // above calls it on Escape — and this call is precisely what makes that
        // safe, by keeping ownership with the `Retained` rather than handing it
        // to AppKit. Do not remove it on the assumption that nothing closes.
        unsafe { window.setReleasedWhenClosed(false) };

        // macOS will otherwise merge a plain window into a tab group, which
        // puts a tab bar on top of the material and undoes the whole effect.
        window.setTabbingMode(NSWindowTabbingMode::Disallowed);
        window.setRestorable(false);

        Self { window, chrome }
    }

    /// Whether this window was built by [`borderless`], and therefore lacks the
    /// behaviours listed there.
    ///
    /// [`borderless`]: GlassWindow::borderless
    pub fn is_borderless(&self) -> bool {
        self.chrome == Chrome::Borderless
    }

    /// Centre the window on the active screen.
    pub fn center(&self) {
        self.window.center();
    }

    /// Position the window's bottom-left corner, in AppKit screen coordinates.
    pub fn set_origin(&self, origin: NSPoint) {
        self.window.setFrameOrigin(origin);
    }

    /// Whether the window casts a shadow.
    ///
    /// Not set by default, and deliberately not a constructor argument. This is
    /// a preference, NOT a measurement — `MEASUREMENTS.md` contains no shadow
    /// reading of any kind: over a light material the shadow reads as a hard
    /// dark edge, where the system's own light glass surfaces appear to take
    /// their definition from a faint bright rim. The caller decides, because
    /// only the caller knows which appearance the surface resolved to.
    pub fn set_has_shadow(&self, has_shadow: bool) {
        if self.window.hasShadow() == has_shadow {
            return;
        }
        self.window.setHasShadow(has_shadow);
        // AppKit caches the shadow shape derived from the window's opaque
        // region. For a transparent window that shape comes from the rendered
        // content, so a toggle on an already-visible window needs the cached
        // shape discarded.
        self.window.invalidateShadow();
    }

    /// Make the window visible and give it focus.
    pub fn show(&self) {
        self.window.makeKeyAndOrderFront(None::<&AnyObject>);
    }

    /// Make Escape close the window.
    ///
    /// Off by default — a window that vanishes on Escape is a surprise unless
    /// the caller asked for it. Worth turning on for any borderless window,
    /// which otherwise offers the user no way to close it at all.
    pub fn set_close_on_escape(&self, enabled: bool) {
        self.window.ivars().close_on_escape.set(enabled);
    }

    /// Force the window's appearance, or pass `None` to inherit the system's.
    ///
    /// Forcing is correct for a widget-style surface, which carries its own
    /// light/dark independent of the system appearance — unlike an ordinary
    /// window, which should simply follow.
    pub fn set_appearance(&self, appearance: Option<&NSAppearance>) {
        self.window.setAppearance(appearance);
    }

    /// The appearance this window *forces*, if any.
    ///
    /// `None` means it inherits — which is the common case, and is **not** the
    /// same question as [`is_dark`]. Measured: a window that forces nothing
    /// reports `None` here while `is_dark()` answers `true` under a dark
    /// system. Use this to read back what you set; use [`is_dark`] or
    /// [`effective_appearance`] to find out what the window actually looks like.
    ///
    /// [`is_dark`]: GlassWindow::is_dark
    /// [`effective_appearance`]: GlassWindow::effective_appearance
    #[must_use]
    pub fn appearance(&self) -> Option<Retained<NSAppearance>> {
        self.window.appearance()
    }

    /// The appearance the window actually resolves to, forced or inherited.
    #[must_use]
    pub fn effective_appearance(&self) -> Retained<NSAppearance> {
        self.window.effectiveAppearance()
    }

    /// Whether the window's effective appearance resolves to Dark.
    ///
    /// Callers should prefer this over reading their own view's appearance, so
    /// that everything deciding light/dark reads one source *at the moment a
    /// style is applied*. It is not a guarantee that nothing can ever disagree:
    /// when the window inherits the system appearance rather than forcing one,
    /// a system light/dark flip reaches each observer on its own schedule.
    ///
    /// The resolution itself is `macos_liquid_glass::is_dark`, which is available
    /// without the `window` feature and documents why a name comparison is
    /// wrong.
    #[must_use]
    pub fn is_dark(&self) -> bool {
        crate::is_dark(&self.effective_appearance())
    }

    /// Install the window's content view.
    ///
    /// For a glass window this should be the glass surface itself and nothing
    /// else — exactly one glass surface, with all content inside it. See the
    /// `glass` module.
    ///
    /// AppKit **adopts Auto Layout** for the window as a side effect if the
    /// view has constraints; a view positioned by explicit `setFrame:` calls
    /// keeps working, but mixing the two in one hierarchy does not.
    pub fn set_content_view(&self, view: &NSView) {
        self.window.setContentView(Some(view));
    }

    /// The window's installed content view, if it has one.
    #[must_use]
    pub fn content_view(&self) -> Option<Retained<NSView>> {
        self.window.contentView()
    }

    /// The window's **content** size in points — the quantity the constructors
    /// take.
    ///
    /// `contentRectForFrameRect:` rather than `frame().size`, so that
    /// `w.set_size(w.size())` is a no-op by construction rather than by
    /// coincidence. The two are equal today only because the titled shape
    /// carries `FullSizeContentView`: measured on a titled mask *without* that
    /// bit, `frame().size` is 28pt taller than the content size, and a
    /// `set_size(size())` round-trip grows the window by the titlebar height on
    /// every call.
    #[must_use]
    pub fn size(&self) -> NSSize {
        self.window
            .contentRectForFrameRect(self.window.frame())
            .size
    }

    /// Resize the window's content area, keeping its top-left corner fixed.
    ///
    /// # Panics
    ///
    /// If `size` is not finite — see [`new`].
    ///
    /// [`new`]: GlassWindow::new
    pub fn set_size(&self, size: NSSize) {
        self.window
            .setContentSize(checked_size(size, "GlassWindow::set_size"));
    }

    /// The window's bottom-left corner, in AppKit screen coordinates.
    #[must_use]
    pub fn origin(&self) -> NSPoint {
        self.window.frame().origin
    }

    /// Whether the window casts a shadow.
    #[must_use]
    pub fn has_shadow(&self) -> bool {
        self.window.hasShadow()
    }

    /// Whether Escape closes the window — see [`set_close_on_escape`].
    ///
    /// This reads state that exists only in this crate: unlike every other
    /// getter here it has no `NSWindow` equivalent, so a consumer with a
    /// checkable menu item would otherwise have to shadow the flag itself.
    ///
    /// [`set_close_on_escape`]: GlassWindow::set_close_on_escape
    #[must_use]
    pub fn close_on_escape(&self) -> bool {
        self.window.ivars().close_on_escape.get()
    }

    /// The window's title.
    ///
    /// Set by the constructors. Both shapes carry one even though neither
    /// *displays* it — the titled shape hides it deliberately — because it is
    /// what the Window menu, Mission Control and accessibility clients read.
    #[must_use]
    pub fn title(&self) -> String {
        self.window.title().to_string()
    }

    /// Change the window's title.
    pub fn set_title(&self, title: &str) {
        self.window.setTitle(&NSString::from_str(title));
    }

    /// Whether the window is on screen.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    /// Take the window off screen without closing it.
    ///
    /// `orderOut:` — the window keeps its state and [`show`] puts it back.
    ///
    /// [`show`]: GlassWindow::show
    pub fn hide(&self) {
        self.window.orderOut(None::<&AnyObject>);
    }

    /// Close the window.
    ///
    /// The handle stays valid: the constructors call
    /// `setReleasedWhenClosed(false)`, so ownership remains with this `Retained`
    /// and [`show`] will put the window back.
    ///
    /// **But that is a statement about memory, not about your process.** A
    /// delegate answering `applicationShouldTerminateAfterLastWindowClosed:`
    /// with `true` — which is the common shape, and what both of this crate's
    /// examples do — terminates the application when the last window closes, so
    /// there is nothing left to re-show.
    ///
    /// Unlike Escape (see [`set_close_on_escape`]) this does **not** consult
    /// `windowShouldClose:`. It is the caller's own decision to close, already
    /// made; `NSWindow::close` behaves the same way.
    ///
    /// [`show`]: GlassWindow::show
    /// [`set_close_on_escape`]: GlassWindow::set_close_on_escape
    pub fn close(&self) {
        self.window.close();
    }

    /// The main-thread marker this window was built with.
    ///
    /// Free — `MainThreadOnly::mtm` "exists purely in the type-system" — and it
    /// saves a consumer threading a separate marker through their app struct
    /// just to build a surface or an observer alongside the window they already
    /// hold.
    #[must_use]
    pub fn mtm(&self) -> MainThreadMarker {
        self.window.mtm()
    }

    /// The underlying `NSWindow`, for anything this wrapper does not cover.
    #[must_use]
    pub fn ns_window(&self) -> &NSWindow {
        &self.window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Borderless` is the constant 0, so the borderless mask is literally just
    /// `Resizable` — which is why adding `Miniaturizable` to it buys nothing.
    /// Measured: `miniaturize:` does nothing on that window even with the bit
    /// set, because miniaturising needs a theme frame.
    #[test]
    fn borderless_mask_is_resizable_alone() {
        assert_eq!(
            Chrome::Borderless.style_mask(),
            NSWindowStyleMask::Resizable
        );
    }

    /// The titled mask must carry FullSizeContentView, or the content view
    /// stops below the titlebar and the material is cut off at the top.
    #[test]
    fn titled_mask_carries_the_bits_the_behaviours_need() {
        let m = Chrome::Titled.style_mask();
        for (bit, why) in [
            (
                NSWindowStyleMask::Titled,
                "the theme frame every behaviour lives in",
            ),
            (
                NSWindowStyleMask::FullSizeContentView,
                "content under the titlebar",
            ),
            (NSWindowStyleMask::Closable, "the close button"),
            (
                NSWindowStyleMask::Miniaturizable,
                "minimise, which borderless cannot do",
            ),
            (NSWindowStyleMask::Resizable, "zoom and edge resize"),
        ] {
            assert!(m.contains(bit), "titled mask must contain {bit:?}: {why}");
        }
    }

    #[test]
    fn titled_is_the_default_chrome() {
        assert_eq!(Chrome::default(), Chrome::Titled);
    }
}
