//! A responder that runs the run loop inside one of its own calls: what
//! `-[NSWorkspace activateFileViewerSelectingURLs:]` does inside a frame, and
//! what any modal panel, any `NSAlert`, any synchronous AppKit call with a
//! nested loop does. The loop runs whatever was queued for the view -- a
//! [`Waker::wake`], a [`Waker::present`], a display-link tick, an input event
//! -- while the outer call still holds the responder.
//!
//! Each of those must arrive after the outer call returns, never be lost,
//! and never panic. And a panic in a responder must stop at the view: a
//! perform's panic that reaches Foundation is caught and discarded there,
//! and Rust then aborts the process.
//!
//! `harness = false` because an `NSView` is main-thread-only and libtest runs
//! every test on a worker thread: `main` here IS the main thread. Nothing is
//! shown: the view is never put in a window.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use macos_liquid_glass::drawable::{
    DrawableView, Focus, Handled, KeyPress, Mouse, MouseKind, Rect, Responder, Scroll, Surface,
    Waker,
};
use macos_liquid_glass::objc2::rc::Retained;
use macos_liquid_glass::objc2::{msg_send, sel};
use macos_liquid_glass::objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use macos_liquid_glass::objc2_core_foundation::CGFloat;
use macos_liquid_glass::objc2_foundation::{
    MainThreadMarker, NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSRunLoop, NSSize, NSString,
};
use macos_liquid_glass::objc2_quartz_core::CADisplayLink;

/// What the responder heard, in order.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Heard {
    FrameStart,
    FrameEnd,
    Mouse(MouseKind),
    Key(u16),
    Presented,
    Wake,
}

type Log = Rc<RefCell<Vec<Heard>>>;

/// What a frame does in the middle: the reentrant part of each check.
type Inside = Rc<dyn Fn()>;

struct Nested {
    heard: Log,
    inside: Option<Inside>,
    /// Panic on this many wakes before behaving.
    panics_left: Rc<Cell<u32>>,
}

impl Responder for Nested {
    fn key(&mut self, key: &KeyPress) -> Handled {
        self.heard.borrow_mut().push(Heard::Key(key.key_code));
        Handled::Consumed
    }
    fn text(&mut self, _: &str) {}
    fn command(&mut self, _: &str) {}
    fn marked_text(&mut self, _: &str) {}
    fn mouse(&mut self, event: &Mouse) {
        self.heard.borrow_mut().push(Heard::Mouse(event.kind));
    }
    fn scroll(&mut self, _: &Scroll) {}
    fn resized(&mut self, _: NSSize, _: CGFloat) {}
    fn frame(&mut self, _: f64) {
        self.heard.borrow_mut().push(Heard::FrameStart);
        if let Some(inside) = &self.inside {
            inside();
        }
        self.heard.borrow_mut().push(Heard::FrameEnd);
    }
    fn focus(&mut self, _: Focus) {}
    fn cursor_rect(&self) -> Rect {
        Rect::default()
    }
    fn wake(&mut self) {
        if self.panics_left.get() > 0 {
            self.panics_left.set(self.panics_left.get() - 1);
            panic!("a responder's own bug, deliberately");
        }
        self.heard.borrow_mut().push(Heard::Wake);
    }
    fn presented(&mut self) {
        self.heard.borrow_mut().push(Heard::Presented);
    }
}

fn view(mtm: MainThreadMarker) -> Retained<DrawableView> {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(64.0, 64.0));
    DrawableView::new(mtm, frame)
}

/// Runs the current run loop in the default mode until nothing is left to
/// fire: the nested loop a synchronous AppKit call runs. A
/// `performSelectorOnMainThread` queued in the common modes fires here.
fn run_nested_loop() {
    let run_loop = NSRunLoop::currentRunLoop();
    for _ in 0..8 {
        let soon = NSDate::dateWithTimeIntervalSinceNow(0.02);
        // SAFETY: the default-mode constant is a valid run-loop mode for
        // the life of the process.
        unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &soon) };
    }
}

fn mouse_down() -> Retained<NSEvent> {
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseDown,
        NSPoint::new(10.0, 10.0),
        NSEventModifierFlags::empty(),
        0.0,
        0,
        None,
        0,
        1,
        1.0,
    )
    .expect("AppKit makes a mouse event")
}

fn key_down() -> Retained<NSEvent> {
    let a = NSString::from_str("a");
    NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags::empty(),
        0.0,
        0,
        None,
        &a,
        &a,
        false,
        0,
    )
    .expect("AppKit makes a key event")
}

fn tick(view: &DrawableView, link: &CADisplayLink) {
    // SAFETY: `tick:` is the view's display-link target and takes the link.
    let _: () = unsafe { msg_send![view, tick: link] };
}

/// A frame that queues a wake and a present, sends a tick and a click of
/// its own, then runs a nested loop: the wake, the present, the click and the key
/// arrive after the frame returns, each once, and the nested tick is
/// skipped -- the next real tick draws.
fn what_a_frame_runs_into_arrives_after_it(mtm: MainThreadMarker) {
    let view = view(mtm);
    let heard: Log = Rc::new(RefCell::new(Vec::new()));
    let waker: Waker = view.waker();
    let surface = Surface::new(4, 4).expect("an IOSurface");
    // SAFETY: the view implements `tick:` with the display-link signature
    // and outlives the link, which is never added to a run loop: this test
    // sends `tick:` itself.
    let link = unsafe { view.displayLinkWithTarget_selector(&view, sel!(tick:)) };

    let inside: Inside = {
        let view = view.clone();
        let link = link.clone();
        let handle = Rc::new(surface.handle());
        Rc::new(move || {
            waker.present(&handle);
            waker.wake();
            // First, as baud met it: the queued performs fire through
            // Foundation's `__NSThreadPerformPerform`.
            run_nested_loop();
            tick(&view, &link);
            // SAFETY: a left-mouse-down event, the type `mouseDown:` takes.
            let _: () = unsafe { msg_send![&*view, mouseDown: &*mouse_down()] };
            // SAFETY: a key-down event, the type `keyDown:` takes. Held
            // whole and replayed, so its answer still decides.
            let _: () = unsafe { msg_send![&*view, keyDown: &*key_down()] };
        })
    };
    view.set_responder(Box::new(Nested {
        heard: heard.clone(),
        inside: Some(inside),
        panics_left: Rc::new(Cell::new(0)),
    }));

    tick(&view, &link);
    run_nested_loop();

    assert_eq!(
        *heard.borrow(),
        [
            Heard::FrameStart,
            Heard::FrameEnd,
            Heard::Mouse(MouseKind::Down),
            Heard::Key(0),
            Heard::Presented,
            Heard::Wake,
        ],
        "one frame, the nested tick skipped, and everything else after it"
    );
}

/// A wake arriving through Foundation's perform -- the path of the crash --
/// whose responder panics: the panic stops at the view, the process lives,
/// and the next wake reaches the responder, so the panic released it.
fn a_panic_in_a_responder_stops_at_the_view(mtm: MainThreadMarker) {
    let view = view(mtm);
    let heard: Log = Rc::new(RefCell::new(Vec::new()));
    view.set_responder(Box::new(Nested {
        heard: heard.clone(),
        inside: None,
        panics_left: Rc::new(Cell::new(1)),
    }));
    let waker = view.waker();

    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    waker.wake();
    run_nested_loop();
    std::panic::set_hook(quiet);

    waker.wake();
    run_nested_loop();
    assert_eq!(*heard.borrow(), [Heard::Wake], "the second wake arrives");
}

type Test = fn(MainThreadMarker);

fn main() {
    let mtm = MainThreadMarker::new().expect("a harness-less test's main is the main thread");
    let tests: &[(&str, Test)] = &[
        (
            "what_a_frame_runs_into_arrives_after_it",
            what_a_frame_runs_into_arrives_after_it,
        ),
        (
            "a_panic_in_a_responder_stops_at_the_view",
            a_panic_in_a_responder_stops_at_the_view,
        ),
    ];
    for (name, test) in tests {
        test(mtm);
        println!("test {name} ... ok");
    }
}
