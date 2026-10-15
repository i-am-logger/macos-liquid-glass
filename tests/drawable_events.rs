//! The view's event selectors, sent the way AppKit sends them, reach the
//! responder. `harness = false` because an `NSView` is main-thread-only and
//! libtest runs every test on a worker thread: `main` here IS the main
//! thread, so `MainThreadMarker::new` answers.
//!
//! What this proves is the view's half: that each selector is registered
//! under AppKit's name and turns the event into the responder call. Whether
//! the application routes a real event to the view is AppKit's half, and
//! only a live run shows it.

use std::cell::RefCell;
use std::rc::Rc;

use macos_liquid_glass::drawable::{
    DrawableView, Focus, Handled, KeyPress, Mouse, Rect, Responder, Scroll,
};
use macos_liquid_glass::objc2::msg_send;
use macos_liquid_glass::objc2::rc::Retained;
use macos_liquid_glass::objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use macos_liquid_glass::objc2_core_foundation::CGFloat;
use macos_liquid_glass::objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

/// What the responder heard, in order.
#[derive(Debug, PartialEq)]
enum Heard {
    Key(u16, bool),
    Released(u16, String, bool),
}

struct Recorder(Log);

impl Responder for Recorder {
    fn key(&mut self, key: &KeyPress) -> Handled {
        self.0
            .borrow_mut()
            .push(Heard::Key(key.key_code, key.repeat));
        Handled::Consumed
    }
    fn key_released(&mut self, key: &KeyPress) {
        self.0.borrow_mut().push(Heard::Released(
            key.key_code,
            key.characters.clone(),
            key.modifiers.shift,
        ));
    }
    fn text(&mut self, _: &str) {}
    fn command(&mut self, _: &str) {}
    fn marked_text(&mut self, _: &str) {}
    fn mouse(&mut self, _: &Mouse) {}
    fn scroll(&mut self, _: &Scroll) {}
    fn resized(&mut self, _: NSSize, _: CGFloat) {}
    fn frame(&mut self, _: f64) {}
    fn focus(&mut self, _: Focus) {}
    fn cursor_rect(&self) -> Rect {
        Rect::default()
    }
}

/// What a [`Recorder`] records into, shared with the test that reads it.
type Log = Rc<RefCell<Vec<Heard>>>;

/// One check, run on the main thread.
type Test = fn(MainThreadMarker);

/// A view with a recording responder, and what it records into.
fn view(mtm: MainThreadMarker) -> (Retained<DrawableView>, Log) {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(100.0, 100.0));
    let view = DrawableView::new(mtm, frame);
    view.set_responder(Box::new(Recorder(heard.clone())));
    (view, heard)
}

/// A key event as `keyEventWithType:...` makes one.
fn key_event(
    kind: NSEventType,
    key_code: u16,
    characters: &str,
    flags: NSEventModifierFlags,
    repeat: bool,
) -> Retained<NSEvent> {
    let chars = NSString::from_str(characters);
    NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        kind,
        NSPoint::new(0.0, 0.0),
        flags,
        0.0,
        0,
        None,
        &chars,
        &chars,
        repeat,
        key_code,
    )
    .expect("AppKit makes a key event")
}

/// `keyUp:` reaches `key_released` with the event's key, characters and
/// modifiers, after the press and its repeat went to `key`.
fn a_release_reaches_the_responder(mtm: MainThreadMarker) {
    let (view, heard) = view(mtm);
    let none = NSEventModifierFlags::empty();
    let down = key_event(NSEventType::KeyDown, 0, "a", none, false);
    let again = key_event(NSEventType::KeyDown, 0, "a", none, true);
    let up = key_event(
        NSEventType::KeyUp,
        0,
        "A",
        NSEventModifierFlags::Shift,
        false,
    );
    // SAFETY: each is a key event of the type the selector documents.
    unsafe {
        let _: () = msg_send![&*view, keyDown: &*down];
        let _: () = msg_send![&*view, keyDown: &*again];
        let _: () = msg_send![&*view, keyUp: &*up];
    }
    assert_eq!(
        *heard.borrow(),
        [
            Heard::Key(0, false),
            Heard::Key(0, true),
            Heard::Released(0, "A".into(), true),
        ]
    );
}

fn main() {
    let mtm = MainThreadMarker::new().expect("a harness-less test's main is the main thread");
    let tests: &[(&str, Test)] = &[(
        "a_release_reaches_the_responder",
        a_release_reaches_the_responder,
    )];
    for (name, test) in tests {
        test(mtm);
        println!("test {name} ... ok");
    }
}
