//! `GlassWindow::add_titlebar_accessory` against a real `NSWindow`, headless:
//! the window is built and laid out but never ordered in, so nothing is
//! shown.
//!
//! The checks are relations, not points. Where AppKit puts an accessory
//! depends on the SDK the binary links against (the header's "linked on
//! macOS 27.0 or higher" clauses), so the test holds each accessory to the
//! traffic lights, the window's edge and `contentLayoutRect` rather than to
//! numbers measured on one build.
//!
//! `harness = false` because AppKit is main-thread-only and the test
//! harness runs each test on a thread of its own; this `main` is the
//! process's main thread, so `MainThreadMarker::new` succeeds.

#[cfg(target_os = "macos")]
fn main() {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use macos_liquid_glass::objc2::MainThreadOnly;
    use macos_liquid_glass::objc2::rc::Retained;
    use macos_liquid_glass::objc2_app_kit::{NSLayoutAttribute, NSView, NSWindowButton};
    use macos_liquid_glass::objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
    use macos_liquid_glass::window::{GlassWindow, TitlebarSide};

    let mtm = MainThreadMarker::new().expect("harness = false runs main on the main thread");

    let view = |width: f64| {
        NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 28.0)),
        )
    };
    let in_window = |v: &NSView| v.convertRect_toView(v.bounds(), None);
    // SAFETY: `superview` is unsafe only because AppKit may hand back a view
    // that is being torn down; nothing here is, and each one is used at once.
    let superview = |v: &NSView| unsafe { v.superview() };

    let window = GlassWindow::new(mtm, NSSize::new(960.0, 600.0), "accessories");
    let ns = window.ns_window();
    ns.layoutIfNeeded();

    // A view's ancestors below the theme frame, the content view's superview
    // and the root of everything the window draws. Structural, not by class
    // name: AppKit's titlebar classes are private and free to change, so the
    // test asks only whether two views share a container under that root. A
    // content subview's chain is the content view alone, which no titlebar
    // view descends from.
    let theme_frame = ns
        .contentView()
        .and_then(|c| superview(&c))
        .expect("a titled window's content view sits in its theme frame");
    let ancestors = |v: &NSView| -> Vec<*const NSView> {
        let mut out = Vec::new();
        let mut next = superview(v);
        while let Some(view) = next {
            if Retained::as_ptr(&view) == Retained::as_ptr(&theme_frame) {
                break;
            }
            out.push(Retained::as_ptr(&view));
            next = superview(&view);
        }
        out
    };
    let layout = ns.contentLayoutRect();
    let frame = ns.frame();
    let button = |b: NSWindowButton| ns.standardWindowButton(b).expect("titled window button");
    let close = button(NSWindowButton::CloseButton);
    let mini = button(NSWindowButton::MiniaturizeButton);
    let zoom = button(NSWindowButton::ZoomButton);
    let gap =
        in_window(&mini).origin.x - (in_window(&close).origin.x + in_window(&close).size.width);
    let zoom_max_x = in_window(&zoom).origin.x + in_window(&zoom).size.width;
    let button_ancestors = ancestors(&close);
    assert!(
        !button_ancestors.is_empty(),
        "the buttons sit in a titlebar view, not directly in the theme frame"
    );
    let shares_titlebar = |v: &NSView| ancestors(v).iter().any(|a| button_ancestors.contains(a));
    // The control: a content subview, the placement this API replaces, does
    // not share it.
    let content_subview = view(10.0);
    ns.contentView()
        .expect("content view")
        .addSubview(&content_subview);
    assert!(
        !shares_titlebar(&content_subview),
        "a content subview is not in the titlebar"
    );
    content_subview.removeFromSuperview();

    // Leading: beside the traffic lights, in the titlebar.
    let lockup = view(90.0);
    let leading = window.add_titlebar_accessory(&lockup, TitlebarSide::Leading);
    ns.layoutIfNeeded();
    assert_eq!(leading.layoutAttribute(), NSLayoutAttribute::Leading);
    assert!(
        *leading.view() == *lockup,
        "the accessory shows the caller's view"
    );
    assert_eq!(ns.titlebarAccessoryViewControllers().count(), 1);
    assert!(
        shares_titlebar(&lockup),
        "the accessory shares the traffic lights' titlebar container, so it goes where they go"
    );
    let r = in_window(&lockup);
    assert_eq!(
        r.origin.x,
        zoom_max_x + gap,
        "leading starts one button-gap after the zoom button"
    );
    assert_eq!(r.size.width, 90.0, "the width is the view's");
    assert_eq!(
        r.origin.y,
        layout.origin.y + layout.size.height,
        "the accessory sits above the content area"
    );
    assert_eq!(
        r.origin.y + r.size.height,
        frame.size.height,
        "and fills the titlebar to the window's top"
    );

    // Trailing: flush with the far edge, in the same titlebar.
    let lights = view(200.0);
    let trailing = window.add_titlebar_accessory(&lights, TitlebarSide::Trailing);
    ns.layoutIfNeeded();
    assert_eq!(trailing.layoutAttribute(), NSLayoutAttribute::Trailing);
    assert_eq!(ns.titlebarAccessoryViewControllers().count(), 2);
    assert!(shares_titlebar(&lights), "trailing is in the same titlebar");
    let r = in_window(&lights);
    assert_eq!(
        r.origin.x + r.size.width,
        frame.size.width,
        "trailing is flush"
    );
    assert_eq!(r.size.width, 200.0);

    assert_eq!(
        ns.contentLayoutRect(),
        layout,
        "adding accessories moves no content"
    );

    // A new width, keeping the height AppKit set: still flush, now narrower.
    lights.setFrameSize(NSSize::new(120.0, lights.frame().size.height));
    ns.layoutIfNeeded();
    let r = in_window(&lights);
    assert_eq!(r.size.width, 120.0);
    assert_eq!(r.origin.x + r.size.width, frame.size.width);

    // Removing one leaves the other, and the content area, alone.
    trailing.removeFromParentViewController();
    ns.layoutIfNeeded();
    assert_eq!(ns.titlebarAccessoryViewControllers().count(), 1);
    assert!(
        lights.window().is_none(),
        "the removed view leaves the window"
    );
    assert!(lockup.window().is_some());
    assert_eq!(ns.contentLayoutRect(), layout);

    // A borderless window has no titlebar: a Rust panic, not an abort.
    let borderless = GlassWindow::borderless(mtm, NSSize::new(320.0, 200.0), "borderless");
    let stray = view(40.0);
    // The expected panic would print through the default hook; silence it
    // for this one call.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let refused = catch_unwind(AssertUnwindSafe(|| {
        borderless.add_titlebar_accessory(&stray, TitlebarSide::Leading)
    }));
    std::panic::set_hook(hook);
    let message = refused.expect_err("borderless must be refused");
    let message = message
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| message.downcast_ref::<&str>().copied())
        .unwrap_or_default();
    assert!(
        message.contains("borderless"),
        "the panic names the reason: {message}"
    );

    window.close();
    borderless.close();
    println!("titlebar_accessory: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
