//! `MaterialSurface` against the real class, headless: a view built on the
//! main thread with no window and no application, so nothing is shown.
//!
//! `harness = false` because AppKit views are main-thread-only and the test
//! harness runs each test on a thread of its own; this `main` is the
//! process's main thread, so `MainThreadMarker::new` succeeds.

#[cfg(target_os = "macos")]
fn main() {
    use macos_liquid_glass::material::{Material, MaterialState, MaterialSurface};
    use macos_liquid_glass::objc2::MainThreadOnly;
    use macos_liquid_glass::objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
    use macos_liquid_glass::objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

    let mtm = MainThreadMarker::new().expect("harness = false runs main on the main thread");
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 200.0));

    let surface = MaterialSurface::new(mtm, frame, Material::UnderWindowBackground);
    assert_eq!(surface.material(), Some(Material::UnderWindowBackground));
    assert_eq!(surface.state(), Some(MaterialState::FollowsWindow));
    assert!(surface.blends_behind_window());
    assert!(
        surface.content_view().is_none(),
        "a new surface has no subview of its own, so its one subview is the caller's"
    );

    let first = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    surface.set_content_view(&first);
    let shown = surface.content_view().expect("content installed");
    assert!(*shown == *first);
    assert_eq!(
        first.frame(),
        surface.view().bounds(),
        "content fills the surface"
    );
    assert!(
        first.autoresizingMask().contains(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable
        ),
        "content resizes with the surface"
    );

    let second = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    surface.set_content_view(&second);
    assert_eq!(
        surface.view().subviews().count(),
        1,
        "the old content is replaced"
    );
    assert!(*surface.content_view().expect("content") == *second);
    surface.set_content_view(&second);
    assert_eq!(
        surface.view().subviews().count(),
        1,
        "setting it again keeps one"
    );

    surface.set_material(Material::WindowBackground);
    assert_eq!(surface.material(), Some(Material::WindowBackground));
    surface.set_state(MaterialState::Inactive);
    assert_eq!(surface.state(), Some(MaterialState::Inactive));
    println!("material: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
