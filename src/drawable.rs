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
//! candidate window lands where [`Responder::cursor_rect`] says.

use core::ptr::NonNull;
use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AllocAnyThread, DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSEvent, NSEventModifierFlags, NSResponder, NSTextInputClient, NSTrackingArea,
    NSTrackingAreaOptions, NSView,
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

/// A key press as AppKit delivered it, before interpretation.
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
    /// A key-repeat rather than a fresh press.
    pub repeat: bool,
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
    /// Text the input manager produced: a typed character, a dead-key
    /// composition, an IME commit.
    fn text(&mut self, text: &str);
    /// A command selector the input manager chose for a key it did not
    /// turn into text -- `deleteBackward:`, `insertNewline:`, `moveUp:`.
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
    fn wake(&mut self) {}
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
        unsafe {
            let _: () = msg_send![
                self.view.as_ptr(),
                performSelectorOnMainThread: sel!(wake:),
                withObject: core::ptr::null::<AnyObject>(),
                waitUntilDone: false,
            ];
        }
    }

    /// Asks the main thread to show `surface` on the view. Returns at
    /// once; the surface is retained until it has been shown.
    pub fn present(&self, surface: &SurfaceHandle) {
        // SAFETY: as in `wake`; the argument is an `IOSurface`, which is
        // thread-safe and which `performSelectorOnMainThread` retains
        // until the selector has run.
        unsafe {
            let _: () = msg_send![
                self.view.as_ptr(),
                performSelectorOnMainThread: sel!(presentSurface:),
                withObject: surface.0.as_ptr(),
                waitUntilDone: false,
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
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// The layer is ours and stays non-opaque: an opaque layer here
        /// paints the glass out.
        #[unsafe(method_id(makeBackingLayer))]
        fn make_backing_layer(&self) -> Retained<CALayer> {
            let layer = CALayer::new();
            layer.setOpaque(false);
            // SAFETY: `kCAGravityTopLeft` is a QuartzCore constant string.
            layer.setContentsGravity(unsafe { kCAGravityTopLeft });
            layer.setContentsScale(self.ivars().scale.get());
            layer
        }

        /// AppKit asks before `drawRect:`; answering yes and doing nothing
        /// in `updateLayer` keeps it from replacing the surface with a
        /// drawn image.
        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {}

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            let _: () = unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
            self.refresh_scale();
            self.notify_resized();
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.notify_resized();
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            let _: () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
            self.refresh_scale();
            self.notify_resized();
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            let ok: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if ok {
                self.with_responder(|r| r.focus(Focus::Gained));
            }
            ok
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            let ok: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            if ok {
                self.with_responder(|r| r.focus(Focus::Lost));
            }
            ok
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let press = KeyPress {
                key_code: event.keyCode(),
                characters: event.characters().map(|s| s.to_string()).unwrap_or_default(),
                unmodified: event
                    .charactersIgnoringModifiers()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                modifiers: Modifiers::of(event.modifierFlags()),
                repeat: event.isARepeat(),
            };
            let handled = self
                .with_responder(|r| r.key(&press))
                .unwrap_or(Handled::Interpret);
            if handled == Handled::Interpret {
                let events = NSArray::from_slice(&[event]);
                self.interpretKeyEvents(&events);
            }
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, _event: &NSEvent) {}

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Down);
        }
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Up);
        }
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Down);
        }
        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Up);
        }
        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Down);
        }
        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Up);
        }
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Moved);
        }
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Dragged);
        }
        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Dragged);
        }
        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Dragged);
        }
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Entered);
        }
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            self.forward_mouse(event, MouseKind::Exited);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let p = self.local_point(event);
            let scroll = Scroll {
                dx: event.scrollingDeltaX(),
                dy: event.scrollingDeltaY(),
                precise: event.hasPreciseScrollingDeltas(),
                x: p.x,
                y: p.y,
                modifiers: Modifiers::of(event.modifierFlags()),
            };
            self.with_responder(|r| r.scroll(&scroll));
        }

        /// One tracking area over the visible rect, remade whenever AppKit
        /// asks, so mouse-moved and enter/exit arrive.
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
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
        }

        /// The display link's target: one frame.
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) {
            let target = link.targetTimestamp();
            self.with_responder(|r| r.frame(target));
        }

        /// A [`Waker::wake`], arriving on the main thread.
        #[unsafe(method(wake:))]
        fn wake(&self, _ignored: Option<&AnyObject>) {
            self.with_responder(|r| r.wake());
        }

        /// A [`Waker::present`], arriving on the main thread with the
        /// surface to show.
        #[unsafe(method(presentSurface:))]
        fn present_surface(&self, surface: &IOSurface) {
            self.set_surface(surface);
        }
    }

    unsafe impl NSObjectProtocol for DrawableView {}

    unsafe impl NSTextInputClient for DrawableView {
        #[unsafe(method(insertText:replacementRange:))]
        unsafe fn insert_text_replacement_range(&self, text: &AnyObject, _range: NSRange) {
            let s = string_of(text);
            self.ivars().marked.borrow_mut().clear();
            self.with_responder(|r| {
                r.marked_text("");
                r.text(&s);
            });
        }

        #[unsafe(method(doCommandBySelector:))]
        unsafe fn do_command_by_selector(&self, selector: Sel) {
            let name = selector.name().to_str().unwrap_or("").to_string();
            self.with_responder(|r| r.command(&name));
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        unsafe fn set_marked_text_selected_range_replacement_range(
            &self,
            text: &AnyObject,
            _selected: NSRange,
            _replacement: NSRange,
        ) {
            let s = string_of(text);
            *self.ivars().marked.borrow_mut() = s.clone();
            self.with_responder(|r| r.marked_text(&s));
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.ivars().marked.borrow_mut().clear();
            self.with_responder(|r| r.marked_text(""));
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            NSRange::new(NSUInteger::MAX, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let n = self.ivars().marked.borrow().encode_utf16().count();
            if n == 0 {
                NSRange::new(NSUInteger::MAX, 0)
            } else {
                NSRange::new(0, n)
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            !self.ivars().marked.borrow().is_empty()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        unsafe fn attributed_substring_for_proposed_range_actual_range(
            &self,
            _range: NSRange,
            _actual: *mut NSRange,
        ) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        unsafe fn first_rect_for_character_range_actual_range(
            &self,
            _range: NSRange,
            _actual: *mut NSRange,
        ) -> NSRect {
            let r = self
                .with_responder(|r| r.cursor_rect())
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
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, _point: NSPoint) -> NSUInteger {
            NSUInteger::MAX
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
        });
        // SAFETY: `initWithFrame:` is NSView's designated initialiser.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        this.setWantsLayer(true);
        this
    }

    /// Installs the responder every event goes to.
    pub fn set_responder(&self, responder: Box<dyn Responder>) {
        *self.ivars().responder.borrow_mut() = Some(responder);
    }

    fn with_responder<R>(&self, f: impl FnOnce(&mut dyn Responder) -> R) -> Option<R> {
        let mut slot = self.ivars().responder.borrow_mut();
        slot.as_mut().map(|r| f(r.as_mut()))
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
        self.with_responder(|r| r.resized(size, scale));
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
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        // Flipped view: `p` is already top-left.
        p
    }

    fn forward_mouse(&self, event: &NSEvent, kind: MouseKind) {
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
        self.with_responder(|r| r.mouse(&m));
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

    /// A surface of `width` by `height` pixels; `None` when the kernel
    /// declines, or for a zero side.
    pub fn new(width: usize, height: usize) -> Option<Surface> {
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
    use super::Surface;

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
