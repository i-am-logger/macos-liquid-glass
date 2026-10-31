//! `AppDelegate` and the File menu against the real `NSApplication`,
//! headless: the delegate is installed and its methods sent as AppKit
//! sends them, and the menu is built and read back. No window is made and
//! the app never runs, so nothing is shown.
//!
//! `harness = false` because AppKit is main-thread-only and the test
//! harness runs each test on a thread of its own; this `main` is the
//! process's main thread, so `MainThreadMarker::new` succeeds.

#[cfg(target_os = "macos")]
fn main() {
    use std::cell::Cell;
    use std::rc::Rc;

    use macos_liquid_glass::app::{AppDelegate, LastWindowClosed};
    use macos_liquid_glass::objc2::msg_send;
    use macos_liquid_glass::objc2::runtime::{AnyObject, Bool};
    use macos_liquid_glass::objc2::sel;
    use macos_liquid_glass::objc2_app_kit::NSApplication;
    use macos_liquid_glass::objc2_foundation::{MainThreadMarker, NSString};

    let mtm = MainThreadMarker::new().expect("harness = false runs main on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = AppDelegate::install(&app, mtm);
    let object = delegate.as_object();
    let app_object: &AnyObject = &app;

    // SAFETY: each selector is the NSApplicationDelegate method of that
    // name, which the delegate implements with these argument types.
    let terminate = || -> bool {
        let b: Bool = unsafe {
            msg_send![object, applicationShouldTerminateAfterLastWindowClosed: app_object]
        };
        b.as_bool()
    };
    let reopen = |visible: bool| -> bool {
        let b: Bool = unsafe {
            msg_send![object, applicationShouldHandleReopen: app_object, hasVisibleWindows: Bool::new(visible)]
        };
        b.as_bool()
    };

    // The delegate is NSApplication's.
    assert!(
        app.delegate().is_some(),
        "the delegate is installed on the app"
    );

    // The last window's close: stays running by default, quits when told.
    assert_eq!(delegate.last_window_closed(), LastWindowClosed::KeepRunning);
    assert!(!terminate(), "stays running by default");
    delegate.set_last_window_closed(LastWindowClosed::Quit);
    assert!(terminate(), "quits when told");
    delegate.set_last_window_closed(LastWindowClosed::KeepRunning);
    assert!(!terminate());

    // With no New Window, a reopen is AppKit's to handle.
    assert!(reopen(false), "no hook: AppKit's default");
    let opened = Rc::new(Cell::new(0u32));
    let count = Rc::clone(&opened);
    delegate.on_new_window(move || count.set(count.get() + 1));
    // A Dock click with a window visible brings it forward, AppKit's way.
    assert!(reopen(true));
    assert_eq!(opened.get(), 0, "a visible window opens nothing");
    // With none, the consumer's New Window opens one and AppKit does nothing.
    assert!(!reopen(false));
    assert_eq!(opened.get(), 1, "the Dock click opened a window");
    // File > New Window reaches the delegate.
    let responds: Bool = unsafe { msg_send![object, respondsToSelector: sel!(newWindow:)] };
    assert!(responds.as_bool(), "the delegate answers newWindow:");
    let none: Option<&AnyObject> = None;
    let _: () = unsafe { msg_send![object, newWindow: none] };
    assert_eq!(opened.get(), 2, "New Window opened a window");

    // The File menu carries New Window (Command-N) and Close (Command-W);
    // the Window menu no longer carries Close.
    macos_liquid_glass::menu::install_main_menu(&app, mtm, "Test");
    let bar = app.mainMenu().expect("the main menu");
    let titles: Vec<String> = (0..bar.numberOfItems())
        .filter_map(|i| bar.itemAtIndex(i))
        .filter_map(|holder| holder.submenu())
        .map(|m| m.title().to_string())
        .collect();
    assert_eq!(titles, ["Test", "File", "Edit", "View", "Window"]);
    let file = bar
        .itemAtIndex(1)
        .and_then(|h| h.submenu())
        .expect("the File menu");
    let new = file
        .itemWithTitle(&NSString::from_str("New Window"))
        .expect("New Window");
    assert_eq!(new.keyEquivalent().to_string(), "n");
    assert_eq!(new.action(), Some(sel!(newWindow:)));
    let close = file
        .itemWithTitle(&NSString::from_str("Close"))
        .expect("Close in File");
    assert_eq!(close.keyEquivalent().to_string(), "w");
    assert_eq!(close.action(), Some(sel!(performClose:)));
    let window = app.windowsMenu().expect("the windows menu");
    assert!(
        window.itemWithTitle(&NSString::from_str("Close")).is_none(),
        "Close left the Window menu"
    );
    println!("app_delegate: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
