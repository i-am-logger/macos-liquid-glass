//! A content view something else paints into: an `NSView` whose layer shows
//! an `IOSurface`, with a display link, the input events a responder wants,
//! and the backing-scale bookkeeping that AppKit leaves to the view.
//!
//! This is the seam between a renderer and the window. A renderer writes
//! pixels into an `IOSurface` -- under `IOSurfaceLock` on the CPU, or as a
//! Metal texture made from the same surface -- and hands it to
//! [`DrawableView::set_surface`], which sets it as the layer's `contents`
//! inside a Core Animation transaction with actions disabled. That commit
//! is the present: there is no drawable pool, no `nextDrawable` that can
//! block, and one surface serves a CPU and a GPU renderer alike.
//!
//! # The glass constraint
//!
//! The layer is **not opaque**. `NSGlassEffectView` samples what is behind
//! the effect and composites its content view on top, so an opaque layer
//! filling the content view paints the glass out entirely -- the crate's
//! measurements record exactly that for an opaque paper fill. A renderer
//! that wants the material visible clears to its background at an alpha
//! below one and leaves darkening to `GlassSurface::set_tint_color`.
//!
//! # Input
//!
//! Every event reaches one [`Responder`]. A key press goes to
//! [`Responder::key`] first; when it answers [`Handled::Interpret`] the
//! event is handed to `interpretKeyEvents:`, which routes it through the
//! input manager and comes back as [`Responder::text`] (including dead
//! keys and an IME commit) or [`Responder::command`] (a selector such as
//! `deleteBackward:`). The view is an `NSTextInputClient`, so the
//! candidate window lands where [`Responder::cursor_rect`] says. A key's
//! release goes to [`Responder::key_released`] and nowhere else: the input
//! manager has no use for it. So does a modifier key pressed or released
//! on its own, to [`Responder::modifiers_changed`]. The click that
//! brings an inactive window forward reaches [`Responder::mouse`] too.
//!
//! # Re-entrancy
//!
//! A responder call can run the run loop: any synchronous AppKit call with
//! a loop of its own does -- `-[NSWorkspace activateFileViewerSelectingURLs:]`,
//! a modal panel, an `NSAlert`. That loop delivers whatever was queued for
//! the view while the responder is still inside its call. None of it
//! reaches the responder until that call returns:
//!
//! - a [`Waker::wake`] is held, and several are folded into one
//!   [`Responder::wake`];
//! - a [`Waker::present`] puts its surface on the layer at once, as it
//!   always does, and its [`Responder::presented`] is held, one for each
//!   present, so none is lost -- later than the commit it reports, by the
//!   length of the outer call;
//! - a display-link tick is skipped: the frame in progress is that frame,
//!   and the next tick draws;
//! - an input or view event (a key, a click, a resize, a focus change) is
//!   held, in order, and delivered as it would have been;
//! - [`DrawableView::set_responder`] from inside a call takes effect when
//!   that call returns.
//!
//! What is held is delivered in that order -- events, then presents, then
//! the wake -- after the outermost call returns and before the view
//! returns to AppKit. A query that must answer at once,
//! `firstRectForCharacterRange:` for the candidate window, answers with an
//! empty rect while the responder is busy.
//!
//! A panic in a responder stops at the view: every method AppKit calls on
//! it catches the panic, writes the selector to standard error, and returns
//! what AppKit's own default would. Left to unwind, a panic in a perform
//! reaches Foundation, which catches and discards it, and Rust then aborts
//! the process. Under `panic = "abort"` the process aborts at the panic
//! regardless; nothing here can change that.

use core::ptr::NonNull;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AllocAnyThread, DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearanceCustomization, NSEvent, NSEventModifierFlags, NSResponder, NSTextInputClient,
    NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedString, NSAttributedStringKey, NSDefaultRunLoopMode,
    NSObjectProtocol, NSPoint, NSRange, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
    NSUInteger,
};
use objc2_foundation::{NSDictionary, NSNumber};
use objc2_io_surface::{
    IOSurface, IOSurfaceLockOptions, IOSurfacePropertyKey, IOSurfacePropertyKeyBytesPerElement,
    IOSurfacePropertyKeyHeight, IOSurfacePropertyKeyPixelFormat, IOSurfacePropertyKeyWidth,
    IOSurfaceRef,
};
use objc2_quartz_core::{CADisplayLink, CALayer, CATransaction, kCAGravityTopLeft};

/// The modifier keys held during an event, as the responder reads them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Modifiers {
    /// Either Shift key.
    pub shift: bool,
    /// Either Control key.
    pub control: bool,
    /// Either Option key.
    pub option: bool,
    /// Either Command key.
    pub command: bool,
    /// The `fn` key, or a key on a keyboard's function row.
    pub function: bool,
}

impl Modifiers {
    fn of(flags: NSEventModifierFlags) -> Modifiers {
        Modifiers {
            shift: flags.contains(NSEventModifierFlags::Shift),
            control: flags.contains(NSEventModifierFlags::Control),
            option: flags.contains(NSEventModifierFlags::Option),
            command: flags.contains(NSEventModifierFlags::Command),
            function: flags.contains(NSEventModifierFlags::Function),
        }
    }
}

/// A modifier key, told apart by side. `#[non_exhaustive]` so that a key
/// this crate does not name yet, such as Globe, can be added without a
/// major bump.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ModifierKey {
    /// The left Shift key.
    LeftShift,
    /// The right Shift key.
    RightShift,
    /// The left Control key.
    LeftControl,
    /// The right Control key.
    RightControl,
    /// The left Option key.
    LeftOption,
    /// The right Option key.
    RightOption,
    /// The left Command key.
    LeftCommand,
    /// The right Command key.
    RightCommand,
    /// The `fn` key.
    Function,
    /// Caps Lock.
    CapsLock,
}

impl ModifierKey {
    /// The modifier key a hardware key code names: the `kVK_` constants
    /// of HIToolbox's `Events.h`.
    fn of(key_code: u16) -> Option<ModifierKey> {
        Some(match key_code {
            0x38 => ModifierKey::LeftShift,
            0x3C => ModifierKey::RightShift,
            0x3B => ModifierKey::LeftControl,
            0x3E => ModifierKey::RightControl,
            0x3A => ModifierKey::LeftOption,
            0x3D => ModifierKey::RightOption,
            0x37 => ModifierKey::LeftCommand,
            0x36 => ModifierKey::RightCommand,
            0x3F => ModifierKey::Function,
            0x39 => ModifierKey::CapsLock,
            _ => return None,
        })
    }

    /// Whether this key is down in `flags`.
    ///
    /// The device-independent flags cannot say: with both Command keys
    /// held, releasing one leaves `Command` set. Below
    /// `NSEventModifierFlagDeviceIndependentFlagsMask` AppKit carries a bit
    /// for each side, IOKit's `NX_DEVICE*KEYMASK` in `IOLLEvent.h`, and
    /// those say which. An event with neither side's bit -- one a program
    /// made rather than a keyboard -- falls back to the shared flag.
    fn is_down(self, flags: NSEventModifierFlags) -> bool {
        const L_CTL: NSUInteger = 0x0000_0001;
        const L_SHIFT: NSUInteger = 0x0000_0002;
        const R_SHIFT: NSUInteger = 0x0000_0004;
        const L_CMD: NSUInteger = 0x0000_0008;
        const R_CMD: NSUInteger = 0x0000_0010;
        const L_ALT: NSUInteger = 0x0000_0020;
        const R_ALT: NSUInteger = 0x0000_0040;
        const R_CTL: NSUInteger = 0x0000_2000;
        let (own, pair, shared) = match self {
            ModifierKey::LeftShift => (L_SHIFT, L_SHIFT | R_SHIFT, NSEventModifierFlags::Shift),
            ModifierKey::RightShift => (R_SHIFT, L_SHIFT | R_SHIFT, NSEventModifierFlags::Shift),
            ModifierKey::LeftControl => (L_CTL, L_CTL | R_CTL, NSEventModifierFlags::Control),
            ModifierKey::RightControl => (R_CTL, L_CTL | R_CTL, NSEventModifierFlags::Control),
            ModifierKey::LeftOption => (L_ALT, L_ALT | R_ALT, NSEventModifierFlags::Option),
            ModifierKey::RightOption => (R_ALT, L_ALT | R_ALT, NSEventModifierFlags::Option),
            ModifierKey::LeftCommand => (L_CMD, L_CMD | R_CMD, NSEventModifierFlags::Command),
            ModifierKey::RightCommand => (R_CMD, L_CMD | R_CMD, NSEventModifierFlags::Command),
            ModifierKey::Function => return flags.contains(NSEventModifierFlags::Function),
            ModifierKey::CapsLock => return flags.contains(NSEventModifierFlags::CapsLock),
        };
        if flags.0 & pair != 0 {
            flags.0 & own != 0
        } else {
            flags.contains(shared)
        }
    }
}

/// A modifier key went down or came up on its own: AppKit's
/// `flagsChanged:`. `#[non_exhaustive]` so that a field, such as the Caps
/// Lock state [`Modifiers`] cannot take, can be added without a major bump.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub struct ModifierChange {
    /// The hardware key code of the key that changed, `NSEvent.keyCode`.
    pub key_code: u16,
    /// Which modifier key that is; `None` for a code this crate does not
    /// name.
    pub key: Option<ModifierKey>,
    /// Whether `key` is down now; `false` when `key` is `None`. For Caps
    /// Lock it is whether `NSEventModifierFlagCapsLock` is set after the
    /// change, which is not measured on a hardware keyboard.
    pub pressed: bool,
    /// The modifiers held after the change.
    pub modifiers: Modifiers,
}

impl ModifierChange {
    /// The change a `flagsChanged:` event's key code and flags describe.
    fn of(key_code: u16, flags: NSEventModifierFlags) -> ModifierChange {
        let key = ModifierKey::of(key_code);
        ModifierChange {
            key_code,
            key,
            pressed: key.is_some_and(|k| k.is_down(flags)),
            modifiers: Modifiers::of(flags),
        }
    }
}

/// A key press as AppKit delivered it, before interpretation; the same
/// fields describe its release, which [`Responder::key_released`] receives.
#[derive(Clone, Debug)]
pub struct KeyPress {
    /// The hardware key code, `NSEvent.keyCode`.
    pub key_code: u16,
    /// `NSEvent.characters`: what the key produced under its modifiers.
    pub characters: String,
    /// `NSEvent.charactersIgnoringModifiers`: the key's own identity.
    pub unmodified: String,
    /// The modifiers held.
    pub modifiers: Modifiers,
    /// A key-repeat rather than a fresh press; `false` for the releases
    /// AppKit sends.
    pub repeat: bool,
}

impl KeyPress {
    /// The fields of a `keyDown:` or `keyUp:` event. Only those two:
    /// `characters` raises on any other kind of event.
    fn of(event: &NSEvent) -> KeyPress {
        KeyPress {
            key_code: event.keyCode(),
            characters: event
                .characters()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            unmodified: event
                .charactersIgnoringModifiers()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            modifiers: Modifiers::of(event.modifierFlags()),
            repeat: event.isARepeat(),
        }
    }
}

/// What a responder did with a key press.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handled {
    /// The responder consumed it; nothing else sees it.
    Consumed,
    /// Hand it to the input manager, which answers through
    /// [`Responder::text`] or [`Responder::command`].
    Interpret,
}

/// Which mouse button an event is about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    /// The primary button.
    Left,
    /// The secondary button.
    Right,
    /// The wheel or a third button.
    Middle,
    /// `NSEvent.buttonNumber` beyond the three.
    Other(u8),
}

/// A mouse event in the view's own pixels, origin top-left.
#[derive(Clone, Copy, Debug)]
pub struct Mouse {
    /// What happened.
    pub kind: MouseKind,
    /// Which button, for a press or release; `Left` otherwise.
    pub button: Button,
    /// Position in points from the view's top-left corner.
    pub x: f64,
    /// Position in points from the view's top-left corner.
    pub y: f64,
    /// The modifiers held.
    pub modifiers: Modifiers,
    /// The click count of a press or release; zero for motion.
    pub clicks: u8,
}

/// What a mouse event reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseKind {
    /// A button went down.
    Down,
    /// A button came up.
    Up,
    /// The pointer moved with no button held.
    Moved,
    /// The pointer moved with a button held.
    Dragged,
    /// The pointer entered the view.
    Entered,
    /// The pointer left the view.
    Exited,
}

/// A scroll event in points, precise when a trackpad sent it.
#[derive(Clone, Copy, Debug)]
pub struct Scroll {
    /// Horizontal delta, in points when precise, else in lines.
    pub dx: f64,
    /// Vertical delta, in points when precise, else in lines.
    pub dy: f64,
    /// Whether a trackpad sent point deltas rather than wheel lines.
    pub precise: bool,
    /// Position in points from the view's top-left corner.
    pub x: f64,
    /// Position in points from the view's top-left corner.
    pub y: f64,
    /// The modifiers held.
    pub modifiers: Modifiers,
}

/// Whether the view holds keyboard focus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    /// The view became the key window's first responder.
    Gained,
    /// The view stopped being it.
    Lost,
}

/// Which way the view's *effective* appearance resolves.
///
/// This is the system light/dark axis, and deliberately not the icon and
/// widget style's own: `WidgetStyle::appearance` forces a mode that is
/// independent of the system's, and under the Default selection it infers
/// light whatever the system is doing. A host that wants its own colours to
/// follow the system reads this; a host that wants them to follow the icon
/// style reads that.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Appearance {
    /// The view resolves light.
    Light,
    /// The view resolves dark.
    Dark,
}

/// A rectangle in the view's points, origin top-left, for the input
/// manager's candidate window.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    /// Left edge in points.
    pub x: f64,
    /// Top edge in points.
    pub y: f64,
    /// Width in points.
    pub w: f64,
    /// Height in points.
    pub h: f64,
}

/// What the view reports to, all on the main thread.
pub trait Responder {
    /// A key went down. Answer [`Handled::Interpret`] to let the input
    /// manager turn it into text or a command.
    fn key(&mut self, key: &KeyPress) -> Handled;
    /// A key came up: AppKit's `keyUp:`, one for each key that went down
    /// while the view was first responder, however many repeats it sent
    /// between, except with Command held (below). Delivered whether the
    /// press was consumed or interpreted, and never interpreted itself, and
    /// the view does not pass a release on to the next responder. Nothing
    /// by default, so a responder that wants presses alone need not know
    /// releases exist.
    ///
    /// The view hears only the releases the application forwards to it,
    /// and `-[NSApplication sendEvent:]` keeps a key-up to itself while
    /// Command is held: measured, a Command-`a` sent through it reaches
    /// [`Responder::key`] going down and never reaches this coming up,
    /// where a plain `a` reaches both. This crate does not own the
    /// application object, so a release with Command held does not arrive
    /// here; `MEASUREMENTS.md` in the repository has the method.
    fn key_released(&mut self, key: &KeyPress) {
        let _ = key;
    }
    /// A modifier key went down or came up on its own: AppKit's
    /// `flagsChanged:`. A terminal that reports modifier keys as keys
    /// (kitty's "report all keys as escape codes") reads `key` and
    /// `pressed`; a host that follows Command -- a link lit under the
    /// pointer while it is held -- reads `modifiers`. Never interpreted,
    /// and nothing by default.
    fn modifiers_changed(&mut self, change: &ModifierChange) {
        let _ = change;
    }
    /// Text the input manager produced: a typed character, a dead-key
    /// composition, an IME commit.
    fn text(&mut self, text: &str);
    /// A command selector the input manager chose for a key it did not
    /// turn into text -- `deleteBackward:`, `insertNewline:`, `moveUp:` --
    /// or a menu action sent to the view: `copy:`, `paste:`, `zoomIn:`,
    /// `zoomOut:`, `zoomActual:`.
    fn command(&mut self, selector: &str);
    /// The text being composed by an IME, to show at the cursor; empty
    /// when composition ends.
    fn marked_text(&mut self, text: &str);
    /// A mouse press, release, motion, drag, enter or exit.
    fn mouse(&mut self, event: &Mouse);
    /// A wheel or trackpad scroll.
    fn scroll(&mut self, event: &Scroll);
    /// The view's size in points changed, or its backing scale did. The
    /// pixel size is the product, rounded.
    fn resized(&mut self, size: NSSize, scale: CGFloat);
    /// A display-link tick: the deadline is `target`, in
    /// `CACurrentMediaTime` seconds.
    fn frame(&mut self, target: f64);
    /// The view became or stopped being the key window's first responder.
    fn focus(&mut self, focus: Focus);
    /// Where the text cursor is, for the candidate window.
    fn cursor_rect(&self) -> Rect;
    /// Another thread asked for the responder's attention through a
    /// [`Waker`]: bytes arrived, a frame completed. Delivered on the main
    /// thread. Nothing by default, so a responder that never hands out a
    /// waker need not know one exists.
    ///
    /// Wakes that arrive while the responder is inside another call (a
    /// nested run loop) are held, folded into one, and delivered when that
    /// call returns.
    fn wake(&mut self) {}
    /// A surface a [`Waker::present`] handed over is on the layer: the
    /// `CATransaction` that set it as the contents has just committed, on
    /// the main thread, and the corresponding [`Responder::wake`] is still
    /// queued behind it. This is the moment a frame is *presented* as far
    /// as the process can see; a responder that times its frames stamps
    /// here rather than at the wake, which any other thread can also send.
    /// Nothing by default.
    ///
    /// Except while the responder is inside another call: a present that
    /// lands during a nested run loop is on the layer at once, but this is
    /// held until that call returns, one per present. A stamp taken then
    /// is late by the length of the call, and several arrive back to back,
    /// so a responder that times frames should not read those as frame
    /// times.
    fn presented(&mut self) {}

    /// The view's effective appearance changed: a system light/dark flip,
    /// the window being given an appearance, or a move to a screen under a
    /// different one. Resolved through [`crate::is_dark`] rather than by
    /// comparing appearance names, which that function documents as wrong.
    ///
    /// A host painting its own content needs this and cannot get it from
    /// `StyleObserver`: that watches the icon and widget style, a different
    /// setting on a different axis. Nothing by default. Named rather than
    /// linked, because `icon_style` is its own feature and this trait is not
    /// gated on it -- `drawable` without `icon-style` is a combination a
    /// consumer can ask for, and a link would dangle in it.
    fn appearance(&mut self, appearance: Appearance) {
        let _ = appearance;
    }
}

/// A handle that reaches the view from any thread, so a reader thread or
/// a GPU completion callback can ask the main thread to do something
/// without waiting for it: [`Waker::wake`] delivers [`Responder::wake`],
/// [`Waker::present`] shows a surface. Both go through
/// `performSelectorOnMainThread:withObject:waitUntilDone:NO`, which
/// AppKit documents as safe from any thread, and neither blocks the
/// caller.
///
/// It retains the view for the life of the process rather than releasing
/// it on drop, because an `NSView` must be released on the main thread
/// and a waker is dropped wherever its thread ends -- which is also why a
/// copy retains nothing more: the one retain outlives every copy.
#[derive(Clone, Copy)]
pub struct Waker {
    view: NonNull<AnyObject>,
}

// SAFETY: the only operations on `view` are `performSelectorOnMainThread`
// calls, which AppKit documents as callable from any thread; the pointer
// is retained once and never released, so it stays valid for the life of
// the process.
unsafe impl Send for Waker {}
unsafe impl Sync for Waker {}

impl Waker {
    /// Asks the main thread to call [`Responder::wake`]. Returns at once.
    pub fn wake(&self) {
        // SAFETY: the view is retained for the life of the process and
        // implements `wake:`; the selector is performed on the main thread
        // without waiting, which is the documented cross-thread contract.
        // Delivered in the common modes -- not the default mode alone -- so
        // it fires during a modal event-tracking loop (a live window resize),
        // where a default-mode perform sits undelivered until the drag ends
        // and the window would appear frozen.
        unsafe {
            let modes = NSArray::from_slice(&[NSRunLoopCommonModes]);
            let _: () = msg_send![
                self.view.as_ptr(),
                performSelectorOnMainThread: sel!(wake:),
                withObject: core::ptr::null::<AnyObject>(),
                waitUntilDone: false,
                modes: &*modes,
            ];
        }
    }

    /// Asks the main thread to show `surface` on the view. Returns at
    /// once; the surface is retained until it has been shown.
    pub fn present(&self, surface: &SurfaceHandle) {
        // SAFETY: as in `wake`; the argument is an `IOSurface`, which is
        // thread-safe and which `performSelectorOnMainThread` retains
        // until the selector has run. In the common modes, so a frame that
        // completes mid-resize reaches the layer during the drag.
        unsafe {
            let modes = NSArray::from_slice(&[NSRunLoopCommonModes]);
            let _: () = msg_send![
                self.view.as_ptr(),
                performSelectorOnMainThread: sel!(presentSurface:),
                withObject: surface.0.as_ptr(),
                waitUntilDone: false,
                modes: &*modes,
            ];
        }
    }
}

/// An `IOSurface` a [`Waker`] can present from any thread: retained for
/// the life of the handle, and thread-safe because `IOSurface` is.
pub struct SurfaceHandle(NonNull<IOSurface>);

// SAFETY: `IOSurface` is documented thread-safe, and the handle only
// passes the pointer to `performSelectorOnMainThread`, which retains it.
unsafe impl Send for SurfaceHandle {}
unsafe impl Sync for SurfaceHandle {}

impl Drop for SurfaceHandle {
    fn drop(&mut self) {
        // SAFETY: the pointer was retained in `Surface::handle` and is
        // released exactly once here; `IOSurface` may be released on any
        // thread.
        unsafe { objc2::ffi::objc_release(self.0.as_ptr().cast()) };
    }
}

/// The view's state that the class methods reach through `ivars()`.
pub struct DrawableIvars {
    responder: RefCell<Option<Box<dyn Responder>>>,
    link: RefCell<Option<Retained<CADisplayLink>>>,
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
    marked: RefCell<String>,
    scale: Cell<CGFloat>,
    /// What arrived while the responder was busy, in order.
    held: RefCell<VecDeque<Held>>,
    /// A wake arrived while the responder was busy; several fold into one.
    wake_held: Cell<bool>,
    /// Presents that arrived while the responder was busy, each owed its
    /// own [`Responder::presented`].
    presents_held: Cell<u32>,
    /// A responder installed from inside a responder call.
    replacement: RefCell<Option<Box<dyn Responder>>>,
    /// Inside `drain`, which must not run inside itself.
    draining: Cell<bool>,
}

/// A call to make on the responder once it is free.
type HeldCall = Box<dyn FnOnce(&mut dyn Responder)>;

/// An event held while the responder was busy.
enum Held {
    /// A call to make on the responder.
    Call(HeldCall),
    /// A key press, held whole: its answer decides whether the input
    /// manager sees it, so it is replayed through `keyDown:` again.
    KeyDown(Retained<NSEvent>),
}

/// The responder was inside a call of its own.
struct Busy;

/// Runs `f`, the body of a method AppKit calls on the view, and stops a
/// panic there: one that unwinds into a Foundation perform is caught and
/// discarded by Foundation, and Rust aborts the process. On a panic it
/// writes the selector to standard error -- the panic hook has already
/// written the message and where -- and returns `fallback()`.
fn guarded<R>(selector: &str, fallback: impl FnOnce() -> R, f: impl FnOnce() -> R) -> R {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => {
            eprintln!(
                "macos-liquid-glass: a panic in DrawableView `{selector}` stopped at the view"
            );
            fallback()
        }
    }
}

/// Clears `draining` however `drain` ends, a panic included, so one panic
/// cannot leave everything after it held for good.
struct Draining<'a>(&'a Cell<bool>);

impl Drop for Draining<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

define_class!(
    /// An `NSView` whose layer shows whatever `IOSurface` was last set.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[name = "MacosLiquidGlassDrawableView"]
    #[ivars = DrawableIvars]
    pub struct DrawableView;

    impl DrawableView {
        /// Top-left origin, so a renderer's rows and AppKit's agree.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            guarded("isFlipped", || true, || true)
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            guarded("acceptsFirstResponder", || true, || true)
        }

        /// Click-through: the click that brings an inactive window forward
        /// also reaches `mouseDown:`, rather than only activating the
        /// window as AppKit's default `NO` does. Whatever the event, which
        /// AppKit may pass as nil.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            guarded("acceptsFirstMouse:", || true, || true)
        }

        /// The layer is ours and stays non-opaque: an opaque layer here
        /// paints the glass out.
        #[unsafe(method_id(makeBackingLayer))]
        fn make_backing_layer(&self) -> Retained<CALayer> {
            guarded("makeBackingLayer", CALayer::new, || {
                let layer = CALayer::new();
                layer.setOpaque(false);
                // SAFETY: `kCAGravityTopLeft` is a QuartzCore constant string.
                layer.setContentsGravity(unsafe { kCAGravityTopLeft });
                layer.setContentsScale(self.ivars().scale.get());
                layer
            })
        }

        /// AppKit asks before `drawRect:`; answering yes and doing nothing
        /// in `updateLayer` keeps it from replacing the surface with a
        /// drawn image.
        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            guarded("wantsUpdateLayer", || true, || true)
        }

        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            guarded("updateLayer", || (), || ());
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            guarded("viewDidChangeBackingProperties", || (), || {
                let _: () = unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
                self.refresh_scale();
                self.notify_resized();
            });
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            guarded("setFrameSize:", || (), || {
                let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
                self.notify_resized();
            });
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            guarded("viewDidChangeEffectiveAppearance", || (), || {
                let _: () = unsafe { msg_send![super(self), viewDidChangeEffectiveAppearance] };
                let appearance = if crate::is_dark(&self.effectiveAppearance()) {
                    Appearance::Dark
                } else {
                    Appearance::Light
                };
                self.deliver(move |r| r.appearance(appearance));
            });
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            guarded("viewDidMoveToWindow", || (), || {
                let _: () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
                self.refresh_scale();
                self.notify_resized();
            });
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            // AppKit's answer stands whatever the responder does: only the
            // delivery is guarded, so a panic in `focus` cannot make the
            // view deny what AppKit already did.
            let ok: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if ok {
                guarded("becomeFirstResponder", || (), || {
                    self.deliver(|r| r.focus(Focus::Gained));
                });
            }
            ok
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            let ok: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            if ok {
                guarded("resignFirstResponder", || (), || {
                    self.deliver(|r| r.focus(Focus::Lost));
                });
            }
            ok
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            guarded("keyDown:", || (), || self.press(event));
        }

        /// A key came up. The responder alone hears it: the input manager
        /// interprets presses, and a release is not one.
        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            guarded("keyUp:", || (), || {
                let release = KeyPress::of(event);
                self.deliver(move |r| r.key_released(&release));
            });
        }

        /// A modifier key went down or came up. Only `keyCode` and
        /// `modifierFlags` are read: `characters` raises on this kind of
        /// event.
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            guarded("flagsChanged:", || (), || {
                let change = ModifierChange::of(event.keyCode(), event.modifierFlags());
                self.deliver(move |r| r.modifiers_changed(&change));
            });
        }

        // The Edit menu's actions, sent down the responder chain by a key
        // equivalent or a click on the item: the responder gets them as the
        // selector, the way the input manager's commands arrive.
        #[unsafe(method(copy:))]
        fn copy_(&self, _sender: Option<&AnyObject>) {
            self.menu_command("copy:");
        }

        #[unsafe(method(paste:))]
        fn paste_(&self, _sender: Option<&AnyObject>) {
            self.menu_command("paste:");
        }

        // The View menu's zoom actions, the same way.
        #[unsafe(method(zoomIn:))]
        fn zoom_in(&self, _sender: Option<&AnyObject>) {
            self.menu_command("zoomIn:");
        }

        #[unsafe(method(zoomOut:))]
        fn zoom_out(&self, _sender: Option<&AnyObject>) {
            self.menu_command("zoomOut:");
        }

        #[unsafe(method(zoomActual:))]
        fn zoom_actual(&self, _sender: Option<&AnyObject>) {
            self.menu_command("zoomActual:");
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.forward_mouse("mouseDown:", event, MouseKind::Down);
        }
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.forward_mouse("mouseUp:", event, MouseKind::Up);
        }
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.forward_mouse("rightMouseDown:", event, MouseKind::Down);
        }
        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.forward_mouse("rightMouseUp:", event, MouseKind::Up);
        }
        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.forward_mouse("otherMouseDown:", event, MouseKind::Down);
        }
        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.forward_mouse("otherMouseUp:", event, MouseKind::Up);
        }
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.forward_mouse("mouseMoved:", event, MouseKind::Moved);
        }
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.forward_mouse("mouseDragged:", event, MouseKind::Dragged);
        }
        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.forward_mouse("rightMouseDragged:", event, MouseKind::Dragged);
        }
        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.forward_mouse("otherMouseDragged:", event, MouseKind::Dragged);
        }
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            self.forward_mouse("mouseEntered:", event, MouseKind::Entered);
        }
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            self.forward_mouse("mouseExited:", event, MouseKind::Exited);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            guarded("scrollWheel:", || (), || {
                let p = self.local_point(event);
                let scroll = Scroll {
                    dx: event.scrollingDeltaX(),
                    dy: event.scrollingDeltaY(),
                    precise: event.hasPreciseScrollingDeltas(),
                    x: p.x,
                    y: p.y,
                    modifiers: Modifiers::of(event.modifierFlags()),
                };
                self.deliver(move |r| r.scroll(&scroll));
            });
        }

        /// One tracking area over the visible rect, remade whenever AppKit
        /// asks, so mouse-moved and enter/exit arrive.
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            guarded("updateTrackingAreas", || (), || {
                let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
                if let Some(old) = self.ivars().tracking.borrow_mut().take() {
                    self.removeTrackingArea(&old);
                }
                let options = NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::MouseMoved
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect;
                // SAFETY: the owner is this view, which outlives the area it
                // holds in its ivars; the rect is ignored under InVisibleRect.
                let area = unsafe {
                    NSTrackingArea::initWithRect_options_owner_userInfo(
                        NSTrackingArea::alloc(),
                        self.bounds(),
                        options,
                        Some(self),
                        None,
                    )
                };
                self.addTrackingArea(&area);
                *self.ivars().tracking.borrow_mut() = Some(area);
            });
        }

        /// The display link's target: one frame. Skipped while the
        /// responder is inside a call: that call may be the frame itself,
        /// running a nested loop, and the next tick draws.
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) {
            guarded("tick:", || (), || {
                if self.ivars().responder.try_borrow_mut().is_err() {
                    return;
                }
                let target = link.targetTimestamp();
                self.deliver(move |r| r.frame(target));
            });
        }

        /// A [`Waker::wake`], arriving on the main thread. Held, and folded
        /// with any other, while the responder is busy.
        #[unsafe(method(wake:))]
        fn wake(&self, _ignored: Option<&AnyObject>) {
            guarded("wake:", || (), || {
                self.ivars().wake_held.set(true);
                self.drain();
            });
        }

        /// A [`Waker::present`], arriving on the main thread with the
        /// surface to show. The surface goes on the layer at once; the
        /// responder hears of it when it is free.
        #[unsafe(method(presentSurface:))]
        fn present_surface(&self, surface: &IOSurface) {
            guarded("presentSurface:", || (), || {
                self.set_surface(surface);
                let held = &self.ivars().presents_held;
                held.set(held.get().saturating_add(1));
                self.drain();
            });
        }
    }

    unsafe impl NSObjectProtocol for DrawableView {}

    unsafe impl NSTextInputClient for DrawableView {
        #[unsafe(method(insertText:replacementRange:))]
        unsafe fn insert_text_replacement_range(&self, text: &AnyObject, _range: NSRange) {
            guarded("insertText:replacementRange:", || (), || {
                let s = string_of(text);
                self.ivars().marked.borrow_mut().clear();
                self.deliver(move |r| {
                    r.marked_text("");
                    r.text(&s);
                });
            });
        }

        #[unsafe(method(doCommandBySelector:))]
        unsafe fn do_command_by_selector(&self, selector: Sel) {
            guarded("doCommandBySelector:", || (), || {
                let name = selector.name().to_str().unwrap_or("").to_string();
                self.deliver(move |r| r.command(&name));
            });
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        unsafe fn set_marked_text_selected_range_replacement_range(
            &self,
            text: &AnyObject,
            _selected: NSRange,
            _replacement: NSRange,
        ) {
            guarded("setMarkedText:selectedRange:replacementRange:", || (), || {
                let s = string_of(text);
                *self.ivars().marked.borrow_mut() = s.clone();
                self.deliver(move |r| r.marked_text(&s));
            });
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            guarded("unmarkText", || (), || {
                self.ivars().marked.borrow_mut().clear();
                self.deliver(|r| r.marked_text(""));
            });
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            let none = || NSRange::new(NSUInteger::MAX, 0);
            guarded("selectedRange", none, none)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            guarded("markedRange", || NSRange::new(NSUInteger::MAX, 0), || {
                let n = self.ivars().marked.borrow().encode_utf16().count();
                if n == 0 {
                    NSRange::new(NSUInteger::MAX, 0)
                } else {
                    NSRange::new(0, n)
                }
            })
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            guarded("hasMarkedText", || false, || {
                !self.ivars().marked.borrow().is_empty()
            })
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        unsafe fn attributed_substring_for_proposed_range_actual_range(
            &self,
            _range: NSRange,
            _actual: *mut NSRange,
        ) -> Option<Retained<NSAttributedString>> {
            guarded("attributedSubstringForProposedRange:actualRange:", || None, || None)
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            guarded("validAttributesForMarkedText", NSArray::new, NSArray::new)
        }

        /// Answers at once, so while the responder is busy the rect is
        /// empty, at the view's origin: the candidate window shows there
        /// for that one ask.
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        unsafe fn first_rect_for_character_range_actual_range(
            &self,
            _range: NSRange,
            _actual: *mut NSRange,
        ) -> NSRect {
            guarded("firstRectForCharacterRange:actualRange:", NSRect::default, || {
                let r = self
                    .ask(|r| r.cursor_rect())
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                // The responder speaks top-left points; AppKit wants the rect
                // in screen coordinates, bottom-left.
                let flipped_y = self.bounds().size.height - r.y - r.h;
                let local = NSRect::new(NSPoint::new(r.x, flipped_y), NSSize::new(r.w, r.h));
                let in_window = self.convertRect_toView(local, None);
                match self.window() {
                    Some(w) => w.convertRectToScreen(in_window),
                    None => in_window,
                }
            })
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, _point: NSPoint) -> NSUInteger {
            guarded("characterIndexForPoint:", || NSUInteger::MAX, || NSUInteger::MAX)
        }
    }
);

/// The string inside what `NSTextInputClient` hands over, which is either
/// an `NSString` or an `NSAttributedString`.
fn string_of(text: &AnyObject) -> String {
    if let Some(s) = text.downcast_ref::<NSString>() {
        return s.to_string();
    }
    if let Some(a) = text.downcast_ref::<NSAttributedString>() {
        return a.string().to_string();
    }
    String::new()
}

impl DrawableView {
    /// A view of `frame`, with no responder yet.
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DrawableIvars {
            responder: RefCell::new(None),
            link: RefCell::new(None),
            tracking: RefCell::new(None),
            marked: RefCell::new(String::new()),
            scale: Cell::new(1.0),
            held: RefCell::new(VecDeque::new()),
            wake_held: Cell::new(false),
            presents_held: Cell::new(0),
            replacement: RefCell::new(None),
            draining: Cell::new(false),
        });
        // SAFETY: `initWithFrame:` is NSView's designated initialiser.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        this.setWantsLayer(true);
        this
    }

    /// Installs the responder every event goes to. Called from inside a
    /// responder call, it takes effect when that call returns, and what
    /// was held for the old responder goes to the new one.
    pub fn set_responder(&self, responder: Box<dyn Responder>) {
        match self.ivars().responder.try_borrow_mut() {
            Ok(mut slot) => {
                // A replacement still waiting from an earlier call is older
                // than this one, and must not overwrite it later.
                self.ivars().replacement.borrow_mut().take();
                *slot = Some(responder);
            }
            Err(_) => *self.ivars().replacement.borrow_mut() = Some(responder),
        }
    }

    /// Calls the responder now, or holds the call until the responder is
    /// free, then delivers anything held.
    fn deliver(&self, f: impl FnOnce(&mut dyn Responder) + 'static) {
        if let Err(Busy) = self.call(f) {
            return;
        }
        self.drain();
    }

    /// Calls the responder now, or, while it is busy, holds `f` and
    /// answers [`Busy`]. Delivers nothing held.
    fn call(&self, f: impl FnOnce(&mut dyn Responder) + 'static) -> Result<(), Busy> {
        let Ok(mut slot) = self.ivars().responder.try_borrow_mut() else {
            self.ivars()
                .held
                .borrow_mut()
                .push_back(Held::Call(Box::new(f)));
            return Err(Busy);
        };
        if let Some(r) = slot.as_mut() {
            f(r.as_mut());
        }
        Ok(())
    }

    /// Asks the responder for an answer now, or answers [`Busy`]: for a
    /// question that cannot wait. `Ok(None)` when there is no responder.
    fn ask<R>(&self, f: impl FnOnce(&mut dyn Responder) -> R) -> Result<Option<R>, Busy> {
        let mut slot = self.ivars().responder.try_borrow_mut().map_err(|_| Busy)?;
        Ok(slot.as_mut().map(|r| f(r.as_mut())))
    }

    /// Delivers what was held while the responder was busy -- the events
    /// in order, then each present, then one wake -- until nothing is
    /// left. Does nothing while the responder is busy, or inside itself:
    /// the outer drain picks up what an inner call held.
    fn drain(&self) {
        let ivars = self.ivars();
        if ivars.draining.get() || ivars.responder.try_borrow_mut().is_err() {
            return;
        }
        ivars.draining.set(true);
        let _clear = Draining(&ivars.draining);
        loop {
            if let Some(next) = ivars.replacement.borrow_mut().take() {
                *ivars.responder.borrow_mut() = Some(next);
            }
            let next = ivars.held.borrow_mut().pop_front();
            if let Some(held) = next {
                match held {
                    Held::Call(f) => {
                        if self.call(f).is_err() {
                            return;
                        }
                    }
                    Held::KeyDown(event) => self.press(&event),
                }
            } else if ivars.presents_held.get() > 0 {
                ivars.presents_held.set(ivars.presents_held.get() - 1);
                if self.call(|r| r.presented()).is_err() {
                    return;
                }
            } else if ivars.wake_held.replace(false) {
                if self.call(|r| r.wake()).is_err() {
                    return;
                }
            } else {
                return;
            }
        }
    }

    /// A key press: to the responder, then to the input manager if it
    /// answers [`Handled::Interpret`] or there is no responder. Held whole
    /// while the responder is busy.
    fn press(&self, event: &NSEvent) {
        let press = KeyPress::of(event);
        let handled = match self.ask(|r| r.key(&press)) {
            Ok(handled) => handled.unwrap_or(Handled::Interpret),
            Err(Busy) => {
                self.ivars()
                    .held
                    .borrow_mut()
                    .push_back(Held::KeyDown(event.retain()));
                return;
            }
        };
        if handled == Handled::Interpret {
            let events = NSArray::from_slice(&[event]);
            self.interpretKeyEvents(&events);
        }
        self.drain();
    }

    /// A menu action, delivered as its selector.
    fn menu_command(&self, selector: &'static str) {
        guarded(
            selector,
            || (),
            || self.deliver(move |r| r.command(selector)),
        );
    }

    /// The window's backing scale factor, 1.0 before the view is in one.
    pub fn backing_scale_factor(&self) -> CGFloat {
        self.ivars().scale.get()
    }

    fn refresh_scale(&self) {
        let scale = self.window().map_or(1.0, |w| w.backingScaleFactor());
        self.ivars().scale.set(scale);
        if let Some(layer) = self.layer() {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            layer.setContentsScale(scale);
            CATransaction::commit();
        }
    }

    fn notify_resized(&self) {
        let size = self.bounds().size;
        let scale = self.ivars().scale.get();
        self.deliver(move |r| r.resized(size, scale));
    }

    /// Shows `surface` as the layer's contents. This is the present; the
    /// surface must not be written again until the next one is shown.
    pub fn set_surface(&self, surface: &IOSurface) {
        if let Some(layer) = self.layer() {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            // SAFETY: an IOSurface is one of the object types `contents`
            // documents, and the layer retains it.
            unsafe { layer.setContents(Some(surface)) };
            CATransaction::commit();
        }
    }

    /// Starts a display link that calls [`Responder::frame`] every frame,
    /// on the main run loop in the common modes so it keeps ticking during
    /// a live resize. Stops any earlier one.
    pub fn start_display_link(&self) {
        self.stop_display_link();
        // SAFETY: the target is this view, which implements `tick:` with
        // the display-link signature and outlives the link it stores.
        let link = unsafe { self.displayLinkWithTarget_selector(self, sel!(tick:)) };
        // SAFETY: the main run loop and the common modes constant are valid
        // for the life of the process.
        unsafe {
            link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
            link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSDefaultRunLoopMode);
        }
        *self.ivars().link.borrow_mut() = Some(link);
    }

    /// A handle that reaches this view from any thread.
    pub fn waker(&self) -> Waker {
        let retained: Retained<DrawableView> = self.retain();
        // The waker never releases: see its doc.
        let ptr = Retained::into_raw(retained).cast::<AnyObject>();
        Waker {
            view: NonNull::new(ptr).expect("a retained object is not null"),
        }
    }

    /// Stops the display link, if one is running.
    pub fn stop_display_link(&self) {
        if let Some(link) = self.ivars().link.borrow_mut().take() {
            link.invalidate();
        }
    }

    fn local_point(&self, event: &NSEvent) -> NSPoint {
        // Flipped view: the point is already top-left.
        self.convertPoint_fromView(event.locationInWindow(), None)
    }

    fn forward_mouse(&self, selector: &str, event: &NSEvent, kind: MouseKind) {
        guarded(selector, || (), || self.mouse_event(event, kind));
    }

    fn mouse_event(&self, event: &NSEvent, kind: MouseKind) {
        let p = self.local_point(event);
        let button = match event.buttonNumber() {
            0 => Button::Left,
            1 => Button::Right,
            2 => Button::Middle,
            n => Button::Other(u8::try_from(n).unwrap_or(u8::MAX)),
        };
        let clicks = match kind {
            MouseKind::Down | MouseKind::Up => u8::try_from(event.clickCount()).unwrap_or(1),
            _ => 0,
        };
        let m = Mouse {
            kind,
            button,
            x: p.x,
            y: p.y,
            modifiers: Modifiers::of(event.modifierFlags()),
            clicks,
        };
        self.deliver(move |r| r.mouse(&m));
    }
}

impl Drop for DrawableIvars {
    fn drop(&mut self) {
        if let Some(link) = self.link.get_mut().take() {
            link.invalidate();
        }
    }
}

/// A protocol object of the view, for AppKit APIs that take one.
pub fn as_text_input_client(view: &DrawableView) -> &ProtocolObject<dyn NSTextInputClient> {
    ProtocolObject::from_ref(view)
}

/// The colour space the surface's bytes are in, which Core Animation
/// converts from to whatever display the window is on. A terminal's
/// colours are specified as sRGB, so that is the default; Display P3 reads
/// the same bytes as the wider gamut, for a caller that wants that on
/// purpose. Untagged, a surface is shown as the display's native values,
/// which differ from display to display.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ColorSpace {
    /// The bytes are sRGB, what a terminal's colours are specified as.
    #[default]
    Srgb,
    /// The bytes are Display P3, the wider gamut, on purpose.
    DisplayP3,
}

/// A BGRA, 32-bit `IOSurface` a renderer writes into on the CPU.
///
/// `write` takes the surface's lock, hands the pixels over as one slice of
/// `0xAARRGGBB` words -- **premultiplied**, which is what Core Animation
/// reads -- with the row stride the kernel chose, and releases the lock.
/// Show it with [`DrawableView::set_surface`]; then leave it alone until
/// another surface is showing, which is why a presenter rotates through
/// three.
pub struct Surface {
    inner: Retained<IOSurface>,
    width: usize,
    height: usize,
    /// Pixels per row, which the kernel may round up from `width`.
    stride: usize,
}

impl Surface {
    /// `'BGRA'`, little-endian `0xAARRGGBB` words.
    const BGRA: u32 = 0x4247_5241;

    /// A surface of `width` by `height` pixels, tagged sRGB; `None` when
    /// the kernel declines, or for a zero side.
    pub fn new(width: usize, height: usize) -> Option<Surface> {
        Surface::in_space(width, height, ColorSpace::Srgb)
    }

    /// A surface of `width` by `height` pixels whose bytes are in `space`.
    pub fn in_space(width: usize, height: usize, space: ColorSpace) -> Option<Surface> {
        if width == 0 || height == 0 {
            return None;
        }
        let number = |v: usize| -> Retained<AnyObject> {
            let n = NSNumber::new_usize(v);
            Retained::into_super(Retained::into_super(Retained::into_super(n)))
        };
        // SAFETY: the four property keys are IOSurface framework constants.
        let (k_w, k_h, k_bpe, k_fmt): (
            &IOSurfacePropertyKey,
            &IOSurfacePropertyKey,
            &IOSurfacePropertyKey,
            &IOSurfacePropertyKey,
        ) = unsafe {
            (
                IOSurfacePropertyKeyWidth,
                IOSurfacePropertyKeyHeight,
                IOSurfacePropertyKeyBytesPerElement,
                IOSurfacePropertyKeyPixelFormat,
            )
        };
        let values = [
            number(width),
            number(height),
            number(4),
            number(usize::try_from(Surface::BGRA).ok()?),
        ];
        let props: Retained<NSDictionary<IOSurfacePropertyKey, AnyObject>> =
            NSDictionary::from_retained_objects(&[k_w, k_h, k_bpe, k_fmt], &values);
        let inner = IOSurface::initWithProperties(IOSurface::alloc(), &props)?;
        // The colour space, serialised the way the key's documentation
        // asks (`CGColorSpaceCopyPropertyList`), so the compositor converts
        // the bytes to each display rather than showing them native.
        // SAFETY: the two names are CoreGraphics framework constants.
        let name = unsafe {
            match space {
                ColorSpace::Srgb => objc2_core_graphics::kCGColorSpaceSRGB,
                ColorSpace::DisplayP3 => objc2_core_graphics::kCGColorSpaceDisplayP3,
            }
        };
        let cg = objc2_core_graphics::CGColorSpace::with_name(Some(name))?;
        let plist = cg.property_list()?;
        // SAFETY: `IOSurface` the class and `IOSurfaceRef` the C type are one
        // object under two names (toll-free bridged); the key is the
        // framework's constant, and the value is the serialised colour space
        // it documents.
        unsafe {
            let r: &IOSurfaceRef = core::ptr::NonNull::from(&*inner)
                .cast::<IOSurfaceRef>()
                .as_ref();
            r.set_value(objc2_io_surface::kIOSurfaceColorSpace, &plist);
        }
        let stride = usize::try_from(inner.bytesPerRow()).ok()? / 4;
        Some(Surface {
            inner,
            width,
            height,
            stride,
        })
    }

    /// Width and height in pixels.
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Pixels from one row to the next in the slice `write` hands over.
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// The `IOSurface` itself, for [`DrawableView::set_surface`] or a Metal
    /// texture.
    pub fn io_surface(&self) -> &IOSurface {
        &self.inner
    }

    /// A retained handle to the surface a [`Waker`] can present from any
    /// thread.
    pub fn handle(&self) -> SurfaceHandle {
        let retained: Retained<IOSurface> = self.inner.clone();
        SurfaceHandle(NonNull::new(Retained::into_raw(retained)).expect("retained"))
    }

    /// Runs `f` over the pixels under the surface's lock: `stride` words a
    /// row, `height` rows. Returns `false`, and does not call `f`, when the
    /// lock is refused.
    pub fn write(&self, f: impl FnOnce(&mut [u32], usize)) -> bool {
        let opts = IOSurfaceLockOptions::empty();
        // SAFETY: a null seed is documented as "not wanted".
        if self.inner.lockWithOptions_seed(opts, std::ptr::null_mut()) != 0 {
            return false;
        }
        let words = self.stride * self.height;
        // SAFETY: while locked, the base address is a mapping of at least
        // `bytesPerRow * height` bytes that nothing else writes; it is
        // 4-byte aligned for a 32-bit format, and the slice does not
        // outlive the lock because `f` returns before `unlock`.
        let pixels = unsafe {
            std::slice::from_raw_parts_mut(self.inner.baseAddress().as_ptr().cast::<u32>(), words)
        };
        f(pixels, self.stride);
        self.inner
            .unlockWithOptions_seed(opts, std::ptr::null_mut());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{ModifierChange, ModifierKey, Surface};
    use objc2_app_kit::NSEventModifierFlags as F;
    use objc2_foundation::NSUInteger;

    /// Each side-specific modifier: its key code, the `NX_DEVICE*KEYMASK`
    /// bit for it and for its other side, and the shared flag.
    const SIDED: [(u16, ModifierKey, NSUInteger, NSUInteger, F); 8] = [
        (0x38, ModifierKey::LeftShift, 0x02, 0x04, F::Shift),
        (0x3C, ModifierKey::RightShift, 0x04, 0x02, F::Shift),
        (0x3B, ModifierKey::LeftControl, 0x01, 0x2000, F::Control),
        (0x3E, ModifierKey::RightControl, 0x2000, 0x01, F::Control),
        (0x3A, ModifierKey::LeftOption, 0x20, 0x40, F::Option),
        (0x3D, ModifierKey::RightOption, 0x40, 0x20, F::Option),
        (0x37, ModifierKey::LeftCommand, 0x08, 0x10, F::Command),
        (0x36, ModifierKey::RightCommand, 0x10, 0x08, F::Command),
    ];

    /// A key going down alone reads as that key pressed, and coming up
    /// as released.
    #[test]
    fn a_modifier_alone_reads_as_pressed_then_released() {
        for (code, key, own, _, shared) in SIDED {
            let down = ModifierChange::of(code, F(shared.0 | own));
            assert_eq!((down.key, down.pressed), (Some(key), true), "{key:?}");
            let up = ModifierChange::of(code, F::empty());
            assert_eq!((up.key, up.pressed), (Some(key), false), "{key:?}");
            assert_eq!(up.modifiers, Default::default(), "{key:?}");
        }
    }

    /// **Both sides held, one released**: the shared flag stays set, so
    /// only the side's own bit can say the released key is up -- and that
    /// the other side is still down.
    #[test]
    fn releasing_one_side_while_the_other_is_held_reads_as_released() {
        for (code, key, own, other, shared) in SIDED {
            let up = ModifierChange::of(code, F(shared.0 | other));
            assert!(!up.pressed, "{key:?} read as down after its release");
            let down = ModifierChange::of(code, F(shared.0 | own | other));
            assert!(down.pressed, "{key:?} read as up with both sides held");
        }
        let held = ModifierChange::of(0x37, F(F::Command.0 | 0x10));
        assert!(
            held.modifiers.command,
            "the other Command key still holds it"
        );
    }

    /// An event with no side bits -- one a program made -- falls back to
    /// the shared flag rather than reading every key as up.
    #[test]
    fn without_side_bits_the_shared_flag_decides() {
        for (code, key, _, _, shared) in SIDED {
            assert!(ModifierChange::of(code, shared).pressed, "{key:?}");
        }
    }

    #[test]
    fn caps_lock_and_fn_read_their_own_flags() {
        let on = ModifierChange::of(0x39, F::CapsLock);
        assert_eq!((on.key, on.pressed), (Some(ModifierKey::CapsLock), true));
        assert!(!ModifierChange::of(0x39, F::empty()).pressed);
        let fn_down = ModifierChange::of(0x3F, F::Function);
        assert_eq!(
            (fn_down.key, fn_down.pressed),
            (Some(ModifierKey::Function), true)
        );
        assert!(fn_down.modifiers.function);
        assert!(!ModifierChange::of(0x3F, F::empty()).pressed);
    }

    /// A code this crate does not name still carries the modifiers, so a
    /// host following Command loses nothing.
    #[test]
    fn an_unnamed_key_code_carries_the_modifiers_and_no_key() {
        let c = ModifierChange::of(0x00, F::Command);
        assert_eq!((c.key, c.pressed), (None, false));
        assert!(c.modifiers.command);
    }

    #[test]
    fn a_surface_takes_pixels_and_reports_its_stride() {
        let s = Surface::new(5, 3).expect("the kernel makes a 5x3 BGRA surface");
        assert_eq!(s.size(), (5, 3));
        assert!(s.stride() >= 5, "{}", s.stride());
        let stride = s.stride();
        assert!(s.write(|px, st| {
            assert_eq!(st, stride);
            assert_eq!(px.len(), stride * 3);
            px[st * 2 + 4] = 0xFF11_2233;
        }));
        assert!(s.write(|px, st| assert_eq!(px[st * 2 + 4], 0xFF11_2233)));
        assert!(Surface::new(0, 3).is_none());
    }
}
