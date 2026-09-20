//! The application's main menu: the three menus every macOS app carries,
//! wired to the standard responder-chain actions, so `Cmd-Q` quits, `Cmd-W`
//! closes the window, `Cmd-M` minimises, and `Cmd-C`/`Cmd-V` reach the
//! first responder's `copy:` and `paste:` -- which `DrawableView` forwards to
//! its `Responder::command`. Both are behind the `drawable` feature, so these
//! are named rather than linked: this module is not gated on it, and an
//! intra-doc link into a module the default features do not build resolves
//! nowhere.
//!
//! A consumer that `forbid`s `unsafe` cannot set a menu item's action, which
//! the bindings make `unsafe` because a selector is an unchecked name; the
//! selectors here are AppKit's own, spelled once.

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::sel;
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{MainThreadMarker, NSString};

/// One item: its title, the action the responder chain receives, and the
/// key equivalent (with Command held; `""` for none).
struct Item {
    title: String,
    action: Sel,
    key: &'static str,
    mask: NSEventModifierFlags,
}

fn item(mtm: MainThreadMarker, spec: &Item) -> Retained<NSMenuItem> {
    let it = NSMenuItem::new(mtm);
    it.setTitle(&NSString::from_str(&spec.title));
    // SAFETY: the selector names a standard AppKit action, which every
    // responder that implements it takes with one object argument; an
    // unimplemented one leaves the item disabled by validation.
    unsafe { it.setAction(Some(spec.action)) };
    it.setKeyEquivalent(&NSString::from_str(spec.key));
    it.setKeyEquivalentModifierMask(spec.mask);
    it
}

fn menu(mtm: MainThreadMarker, title: &str, items: &[Option<Item>]) -> Retained<NSMenu> {
    let m = NSMenu::new(mtm);
    m.setTitle(&NSString::from_str(title));
    for spec in items {
        match spec {
            Some(spec) => m.addItem(&item(mtm, spec)),
            None => m.addItem(&NSMenuItem::separatorItem(mtm)),
        }
    }
    m
}

fn cmd(title: impl Into<String>, action: Sel, key: &'static str) -> Option<Item> {
    Some(Item {
        title: title.into(),
        action,
        key,
        mask: NSEventModifierFlags::Command,
    })
}

/// Install the standard main menu for an app called `name`: the app menu
/// (hide, hide others, show all, quit), Edit (copy, paste), View (bigger,
/// smaller, actual size, as `zoomIn:`, `zoomOut:` and `zoomActual:` on the
/// responder chain) and Window (minimize, zoom, close), the last also
/// registered as the windows menu.
pub fn install_main_menu(app: &NSApplication, mtm: MainThreadMarker, name: &str) {
    let app_menu = menu(
        mtm,
        name,
        &[
            cmd(format!("Hide {name}"), sel!(hide:), "h"),
            Some(Item {
                title: "Hide Others".into(),
                action: sel!(hideOtherApplications:),
                key: "h",
                mask: NSEventModifierFlags::Command | NSEventModifierFlags::Option,
            }),
            cmd("Show All", sel!(unhideAllApplications:), ""),
            None,
            cmd(format!("Quit {name}"), sel!(terminate:), "q"),
        ],
    );
    let edit = menu(
        mtm,
        "Edit",
        &[
            cmd("Copy", sel!(copy:), "c"),
            cmd("Paste", sel!(paste:), "v"),
        ],
    );
    // The view's zoom actions are not AppKit's; a responder that implements
    // them -- `DrawableView` forwards them as commands -- enables the items.
    let view = menu(
        mtm,
        "View",
        &[
            cmd("Bigger", sel!(zoomIn:), "+"),
            cmd("Smaller", sel!(zoomOut:), "-"),
            cmd("Actual Size", sel!(zoomActual:), "0"),
        ],
    );
    let window = menu(
        mtm,
        "Window",
        &[
            cmd("Minimize", sel!(performMiniaturize:), "m"),
            cmd("Zoom", sel!(performZoom:), ""),
            None,
            cmd("Close", sel!(performClose:), "w"),
        ],
    );
    let bar = NSMenu::new(mtm);
    for sub in [&app_menu, &edit, &view, &window] {
        let holder = NSMenuItem::new(mtm);
        holder.setSubmenu(Some(sub));
        bar.addItem(&holder);
    }
    app.setMainMenu(Some(&bar));
    app.setWindowsMenu(Some(&window));
}
