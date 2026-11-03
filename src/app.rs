//! The application delegate an app with more than one window needs:
//! whether it quits when its last window closes, what a Dock click does
//! when no window is open, and the File menu's New Window, which has to
//! reach the app when there is no window for the action to go to.
//!
//! `NSApplication` asks its delegate. With none, the app stays running
//! after its last window closes -- "If your implementation returns NO,
//! control returns to the main event loop and the application is not
//! terminated" (`applicationShouldTerminateAfterLastWindowClosed:`) -- and a
//! Dock click with no visible window "goes through the standard untitled
//! document creation" (`applicationShouldHandleReopen:hasVisibleWindows:`),
//! which reaches a non-document app only through its delegate. So without
//! one, a closed last window cannot be replaced from the Dock or the menu.
//!
//! A consumer that `forbid`s `unsafe` cannot define the delegate class
//! itself; [`AppDelegate::install`] defines it here and takes plain
//! closures.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::MainThreadMarker;

/// What the app does when its last window closes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LastWindowClosed {
    /// Stay running with no window, in the Dock and the menu bar: AppKit's
    /// answer when no delegate gives one, and the macOS convention.
    #[default]
    KeepRunning,
    /// Quit: `NSApplication` then asks `applicationShouldTerminate:` as
    /// for any quit.
    Quit,
}

/// The delegate's state: the consumer's answers.
#[derive(Default)]
struct Hooks {
    new_window: RefCell<Option<Rc<dyn Fn()>>>,
    last: Cell<LastWindowClosed>,
}

impl std::fmt::Debug for Hooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hooks")
            .field("new_window", &self.new_window.borrow().is_some())
            .field("last", &self.last.get())
            .finish()
    }
}

define_class!(
    /// The `NSApplicationDelegate` behind [`AppDelegate`].
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Hooks]
    #[derive(Debug)]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            self.ivars().last.get() == LastWindowClosed::Quit
        }

        /// A Dock click, or a second launch from the Finder, on a running
        /// app. With no visible window -- a minimised one counts as
        /// visible -- the consumer's New Window opens one and AppKit does
        /// nothing more; otherwise AppKit brings the app's windows forward,
        /// its default.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, visible: bool) -> bool {
            // An early `return` is not converted to `BOOL` by the macro, so
            // this is one expression: handled here means AppKit does nothing.
            visible || !self.new_window()
        }
    }

    impl Delegate {
        /// File > New Window: the action goes down the responder chain,
        /// which ends at the application's delegate, so it arrives here
        /// with no window open as well as with one.
        #[unsafe(method(newWindow:))]
        fn new_window_action(&self, _sender: Option<&AnyObject>) {
            self.new_window();
        }
    }
);

impl Delegate {
    fn create(mtm: MainThreadMarker) -> Retained<Delegate> {
        let this = Delegate::alloc(mtm).set_ivars(Hooks::default());
        // SAFETY: `init` is NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }

    /// Calls the consumer's New Window, if it gave one; whether it did. The
    /// closure is cloned out first, so it may set another.
    fn new_window(&self) -> bool {
        let hook = self.ivars().new_window.borrow().clone();
        match hook {
            Some(f) => {
                f();
                true
            }
            None => false,
        }
    }
}

/// **The application's delegate**, installed on `NSApplication`.
///
/// `NSApplication`'s `delegate` is weak, so the app keeps this for as long
/// as it runs; dropped, the delegate goes and AppKit's defaults return.
#[must_use = "NSApplication holds its delegate weakly; keep this for as long as the app runs"]
#[derive(Debug)]
pub struct AppDelegate {
    delegate: Retained<Delegate>,
}

impl AppDelegate {
    /// Installs the delegate on `app`. Until it is told otherwise the app
    /// stays running after its last window closes, and New Window and a
    /// Dock click with no window do nothing.
    pub fn install(app: &NSApplication, mtm: MainThreadMarker) -> AppDelegate {
        let delegate = Delegate::create(mtm);
        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        AppDelegate { delegate }
    }

    /// What File > New Window (`newWindow:`, Command-N in the main menu)
    /// does, and what a Dock click does when no window is visible. Called
    /// on the main thread, from the menu's action or from the reopen event,
    /// never from inside a view's own callback.
    pub fn on_new_window(&self, f: impl Fn() + 'static) {
        *self.delegate.ivars().new_window.borrow_mut() = Some(Rc::new(f));
    }

    /// What the app does when its last window closes.
    pub fn set_last_window_closed(&self, what: LastWindowClosed) {
        self.delegate.ivars().last.set(what);
    }

    /// What the app does when its last window closes, as last set.
    #[must_use]
    pub fn last_window_closed(&self) -> LastWindowClosed {
        self.delegate.ivars().last.get()
    }

    /// The delegate as `NSApplication` holds it, for anything this wrapper
    /// does not cover.
    #[must_use]
    pub fn as_object(&self) -> &AnyObject {
        &self.delegate
    }
}
