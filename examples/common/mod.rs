//! Content shared by both examples.
//!
//! `titled` and `borderless` draw exactly the same surface and differ ONLY in
//! the window they put it in — borderless versus a normal titled window. Keeping
//! the content here is what makes that comparison honest: any visual difference
//! between the two examples is attributable to the chrome, because there is one
//! copy of the drawing code.
//!
//! Included with `#[path]` rather than being an example in its own right —
//! `examples/common/` has no `main`, so cargo does not build it as one.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

use block2::RcBlock;

use macos_liquid_glass::glass::{GlassStyle, GlassSurface};
use macos_liquid_glass::icon_style::{Reconcile, StyleObserver, WidgetStyle};
use macos_liquid_glass::window::GlassWindow;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSBezierPath, NSColor, NSColorSpace, NSFont, NSFontWeightMedium, NSFontWeightRegular,
    NSGraphicsContext, NSLineBreakMode, NSMenu, NSMenuItem, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString, NSTimer,
};

// ---------------------------------------------------------------- geometry

/// Window size in points.
const SIZE: NSSize = NSSize::new(560.0, 360.0);
/// 32pt — the value the Swift oracle used (`b8f2058:223`), and what R20's band
/// parity was measured against, so it is correct for this window.
///
/// Its stated provenance was NOT: the comment used to claim it was read as
/// `contentView.height − contentLayoutRect.height`. Probed on 26A5406e that
/// formula returns 0 on a plain titled window and 28 with
/// `.fullSizeContentView` — no configuration yields 32. Treat 32 as chosen for
/// this window's proportions.
const TITLEBAR_H: f64 = 32.0;
/// Read from `effectiveCornerRadii` on a real *titled* window (16.0 on all four
/// corners). The borderless window this app creates uses `NSNextStepFrame`,
/// which does not implement that selector — so this is the system's window
/// radius, not a read of ours.
const CORNER_RADIUS: f64 = 16.0;
// Layout: chosen by eye for a plausible terminal, not measured. None of 560,
// 360, 14, 12.5, 17 or 12 appears in MEASUREMENTS.md.
const PAD: f64 = 14.0;
const FONT_SIZE: f64 = 12.5;
const LINE_HEIGHT: f64 = 17.0;
const ROW_COUNT: usize = 12;

// ------------------------------------------------------------- calibration
//
// Method note, NOT the derivation of the constants below: the body's dim is
// measured as its share of the BAND — which is what the paper composites over —
// not of the wallpaper.
//   Clear > Dark : body luma 37.6 (R13) / band 61.4 = 0.61 -> implies alpha 0.39
//   Clear > Light: body 100.9 / band 112.5           = 0.90 -> implies alpha 0.10
// The shipped 0.22 / 0.12 are NOT those numbers: they were tuned against the
// widget on the user's wallpaper (R12, reconfirmed R13). The 61.4 band reading
// is also not in MEASUREMENTS.md; §1.3 records the ClearDark band at 72.6.
// An earlier 0.64 came from comparing the body to the WALLPAPER instead of the
// band; it over-darkened the surface.

/// Extra body dim under a dark appearance (`GN_PAPER_D`). This is what makes
/// the title bar read as a band.
fn paper_dark() -> f64 {
    env_f64("GN_PAPER_D", 0.22)
}
/// Extra body dim under a light appearance (`GN_PAPER_L`).
fn paper_light() -> f64 {
    env_f64("GN_PAPER_L", 0.12)
}
/// Strength of the theme-colour wash under Tinted (`GN_TINT`).
///
/// 0.20 is inherited from the Swift original and is **not measured** — no
/// section of `MEASUREMENTS.md` records it. R15 establishes the band/body split
/// below, not this alpha.
fn tint_alpha() -> f64 {
    env_f64("GN_TINT", 0.20)
}
/// How much of the band's theme wash the BODY gets.
///
/// The wash is band-weighted, not uniform. Measured with a GREEN theme over
/// water: the widget's body stays BLUE-dominant, RGB (20.3, 39.4, 56.6) — it
/// keeps the water's own colour — while only its band is green, (28.9, 53.8,
/// 54.2). A uniform wash turned the whole terminal green.
fn tint_body_share() -> f64 {
    env_f64("GN_TBODY", 0.25)
}

/// A plausible session, drawn as static rows. This is a terminal-*looking*
/// window, not a terminal emulator. `(text, is_prompt)`.
const SCRIPT: &[(&str, bool)] = &[
    ("~/Code/test $ sw_vers -productVersion", true),
    ("27.0", false),
    ("~/Code/test $ ./{NAME} --style", true),
    ("", false),
];

// ------------------------------------------------------------------ colour
//
// The flat fills here are built in sRGB explicitly. That pins which colour space
// the measured constants belong to; it is NOT a fix for an observed difference,
// and the rationale this block used to carry was wrong on every point. Probed on
// macOS 27.0 (26A5406e):
//
//   NSColor(hue:saturation:brightness:alpha:)  -> sRGB IEC61966-2.1, and its
//       components are bit-identical to the hand-rolled conversion below
//       (0.02, 0.052, 0.10 both ways). It is NOT the calibrated space.
//   NSColor.black                              -> Generic Gray Gamma 2.2. Not a
//       device space, and it cannot carry a hue at all.
//   blended(withFraction:of:) on two sRGB inputs -> Generic RGB.
//
// That last one matters: `primary`, `dim` and the title colour under Tinted all
// come from a blend, so every Tinted TEXT colour is calibrated RGB no matter what
// its inputs were. "Everything is sRGB" is therefore true of the flat fills,
// false of the theme wash (used as the system hands it over — see
// GN_TINT_SPACE), and false of the Tinted text. Left as-is because the Swift
// oracle blended the same way and R20 measured parity with it.
//
/// Black, in sRGB, at a given alpha.
fn srgb_black(alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, alpha)
}

/// HSB to an sRGB colour.
///
/// Done by hand rather than with `colorWithHue:saturation:brightness:alpha:` —
/// but NOT because that initialiser differs. Probed on macOS 27.0 (26A5406e) it
/// already returns sRGB and bit-identical components. This is belt-and-braces
/// against a version where it does not, and it is what R20's parity was
/// measured with, so it stays. It is not a fix for anything observed.
fn hsb_to_srgb(h: f64, s: f64, b: f64, alpha: f64) -> Retained<NSColor> {
    let c = b * s;
    let hp = (h.rem_euclid(1.0)) * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = b - c;
    NSColor::colorWithSRGBRed_green_blue_alpha(r1 + m, g1 + m, b1 + m, alpha)
}

/// White, in sRGB, at a given alpha.
fn srgb_white(alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, alpha)
}

// --------------------------------------------------------- traffic lights

// ----------------------------------------------------------- content view

/// Mutable state the drawn view needs across callbacks.
struct ContentIvars {
    /// Which example this is. The band shows it, so a screenshot identifies
    /// itself without needing the window frame for context.
    identity: Identity,
    title: Retained<NSTextField>,
    rows: RefCell<Vec<Retained<NSTextField>>>,
    /// Whether the surface resolved to a dark appearance.
    ///
    /// Refreshed both by the owner on a style change and by
    /// `viewDidChangeEffectiveAppearance`, so the two readers share a source
    /// where they can. This narrows, but does not close, the window in which
    /// the paper and the material disagree: under an `*Automatic` token the
    /// appearance is inherited, and a system light/dark flip reaches the view
    /// callback and the material at their own pace.
    is_dark: Cell<bool>,
    /// The theme colour to wash with, in the space the system handed it over
    /// in — converted to sRGB only when `GN_TINT_SPACE=srgb`, which is not the
    /// default because it measured worse (R20). `None` unless Tinted.
    tint: RefCell<Option<Retained<NSColor>>>,
}

define_class!(
    /// Draws the terminal "paper".
    ///
    /// No *gradient* is painted in the title bar: measured on the live Notes
    /// widget, the band tracks the wallpaper while the body does not, so its
    /// gradient comes from the material. Painting one there would be a fitted
    /// constant reproducing what the material already does.
    ///
    /// The band is not untouched, though. Under **Tinted** it takes the theme
    /// wash at full `GN_TINT` alpha while the body takes a quarter of it —
    /// R15's correction to R1, which had concluded the opposite from a reading
    /// taken with a blue theme over a blue wallpaper, where the tint is
    /// invisible.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlassTermContentView"]
    #[ivars = ContentIvars]
    struct TerminalContentView;

    impl TerminalContentView {
        /// Top-left origin, so the title band is at y = 0.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// The window is resizable, so the label frames must be recomputed
        /// whenever the surface changes size.
        ///
        /// An earlier revision of THIS app (82cf233) ran layout only once from
        /// `init`, which left the centred title and the monospaced rows at
        /// their original 560pt widths after any edge drag (R19). The Swift
        /// oracle was not guilty of it — `b8f2058` has an `onResize` hook and a
        /// `setFrameSize` override, by the same mechanism used here.
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, new_size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: new_size] };
            self.layout_subviews();
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw_paper();
        }

        /// AppKit's own light/dark hook.
        ///
        /// The Swift version had no equivalent: it inferred the appearance from
        /// KVO on `AppleInterfaceStyle` plus a reconcile timer, and got away
        /// with it only because the draw path re-read `effectiveAppearance`
        /// each time. This is the documented route and fires for every cause —
        /// the system setting, the window's own `appearance` being assigned,
        /// and a move between screens.
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            let dark = self.resolve_is_dark();
            self.ivars().is_dark.set(dark);
            self.setNeedsDisplay(true);
        }
    }
);

impl TerminalContentView {
    fn new(mtm: MainThreadMarker, frame: NSRect, identity: Identity) -> Retained<Self> {
        let title = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        unsafe {
            title.setFont(Some(&NSFont::systemFontOfSize_weight(
                13.0,
                NSFontWeightMedium,
            )));
        }
        title.setAlignment(NSTextAlignment::Center);

        let this = Self::alloc(mtm).set_ivars(ContentIvars {
            identity,
            title,
            rows: RefCell::new(Vec::new()),
            is_dark: Cell::new(true),
            tint: RefCell::new(None),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        this.addSubview(&this.ivars().title);

        let mono =
            unsafe { NSFont::monospacedSystemFontOfSize_weight(FONT_SIZE, NSFontWeightRegular) };
        {
            let mut rows = this.ivars().rows.borrow_mut();
            for _ in 0..ROW_COUNT {
                let f = NSTextField::labelWithString(&NSString::from_str(""), mtm);
                f.setFont(Some(&mono));
                f.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                this.addSubview(&f);
                rows.push(f);
            }
        }

        this.layout_subviews();
        this
    }

    /// Which example this view belongs to.
    fn identity(&self) -> Identity {
        self.ivars().identity
    }

    /// Whether this view's effective appearance resolves to Dark.
    ///
    /// The crate's resolver, not a hand-rolled one. This used to be a verbatim
    /// 12-line copy of `GlassWindow::is_dark`, written because that method
    /// cannot be applied to an `NSView` — which is exactly why the resolver is
    /// now a free function.
    fn resolve_is_dark(&self) -> bool {
        macos_liquid_glass::is_dark(&self.effectiveAppearance())
    }

    /// Recomputed on every size change.
    fn layout_subviews(&self) {
        let w = self.bounds().size.width;
        let ivars = self.ivars();

        ivars
            .title
            .setFrame(NSRect::new(NSPoint::new(0.0, 8.0), NSSize::new(w, 18.0)));

        let mut y = TITLEBAR_H + PAD;
        for f in ivars.rows.borrow().iter() {
            f.setFrame(NSRect::new(
                NSPoint::new(PAD, y),
                NSSize::new(w - PAD * 2.0, LINE_HEIGHT),
            ));
            y += LINE_HEIGHT;
        }
    }

    /// The paper is a DIMMING layer, not a background fill.
    ///
    /// Measured on the live widget: the content area is darker than the
    /// backdrop in *both* appearances — light body 100.9 against a ~115
    /// wallpaper, dark body 41.1 against the same. An `NSColor
    /// .textBackgroundColor` fill gets this backwards, because it is pure white
    /// in Aqua, and rendered the light terminal as a flat white slab with no
    /// glass left in it. Both appearances darken, so the fill is black in both;
    /// only the strength changes.
    fn draw_paper(&self) {
        let bounds = self.bounds();
        let paper = NSRect::new(
            NSPoint::new(0.0, TITLEBAR_H),
            NSSize::new(bounds.size.width, bounds.size.height - TITLEBAR_H),
        );

        NSGraphicsContext::saveGraphicsState_class();

        // Clip to the window shape so the paper follows the bottom corners.
        let clip = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
            bounds,
            CORNER_RADIUS,
            CORNER_RADIUS,
        );
        clip.addClip();

        // Under Tinted the theme colour washes the BAND at full `GN_TINT` alpha
        // and the body at a quarter of it (R15, correcting R1's uniform-wash
        // model). R1's "washes the WHOLE surface" conclusion is WITHDRAWN and
        // must not be restated here — leaving superseded rationale stacked above
        // its replacement is the defect R19 records three instances of.
        //
        // Drawn here rather than via `NSGlassEffectView.tintColor`, which
        // despite being documented as a bias floods the surface to a flat
        // saturated slab at full strength (R4).
        if let Some(tint) = self.ivars().tint.borrow().as_ref() {
            let band = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(bounds.size.width, TITLEBAR_H),
            );
            tint.colorWithAlphaComponent(tint_alpha()).setFill();
            NSBezierPath::fillRect(band);
            tint.colorWithAlphaComponent(tint_alpha() * tint_body_share())
                .setFill();
            NSBezierPath::fillRect(paper);
        }

        let alpha = if self.ivars().is_dark.get() {
            paper_dark()
        } else {
            paper_light()
        };
        srgb_black(alpha).setFill();
        // NSBezierPath::fillRect, NOT NSRectFill. They are not interchangeable:
        // NSRectFill composites with Copy, which wipes the material instead of
        // dimming it. Measured against the Swift oracle, swapping to NSRectFill
        // made the whole-window MAE 18x worse (0.0082 against 0.00045) while
        // changing the band not at all.
        NSBezierPath::fillRect(paper);

        NSGraphicsContext::restoreGraphicsState_class();
    }

    /// Set the text, and the appearance the paper should dim for.
    fn apply_style(&self, style: &WidgetStyle) {
        self.ivars().is_dark.set(self.resolve_is_dark());

        // `GN_TINT_SPACE=srgb` converts the system's theme colour into sRGB
        // before washing with it; `native` (the default) uses it exactly as
        // handed back.
        //
        // Native is the default because it is what the reference was fitted
        // against: the Swift original washed with the colour as-is, and its
        // constants were tuned by comparing against a live widget rendered by
        // the system, which is also using the colour natively.
        //
        // Measured, the conversion is not free. Under Tinted ▸ Light it spread
        // +0.11 luma across band, body AND text (MAE 0.0011); native confines
        // the difference to the band alone and leaves body and text exact
        // (MAE 0.00045). Note this only chooses the better of two imperfect
        // options — native does NOT reach pixel-identical either. Both Tinted
        // tokens carry a ~0.4 luma band residual whose cause is still open;
        // see MEASUREMENTS.md R20.
        // `retain`, not `clone`: `tint()` hands out a borrowed `&NSColor` — it
        // retains nothing — so taking ownership is an explicit retain.
        let tint = style.tint().map(|c| {
            if std::env::var("GN_TINT_SPACE").as_deref() == Ok("srgb") {
                c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
                    .unwrap_or_else(|| c.retain())
            } else {
                c.retain()
            }
        });
        self.ivars().tint.replace(tint.clone());

        // Content is WHITE in every style — not `labelColor`.
        //
        // This is Apple's accented-rendering rule, and the reference obeys it:
        // the system "tints primary and accented content white in iOS and
        // macOS". The live widget's text is light under Clear ▸ Light too, over
        // a light surface. Resolving text through `labelColor` gets this
        // exactly backwards, because labelColor is black in Aqua — which
        // rendered the light terminal with black text the widget never shows.
        // Text alphas (0.35 blend, 0.62 dim, 0.85 title): chosen by eye, not
        // measured. Do NOT cite R6 for them — R6's 0.35/0.85 are *Ghostty's*
        // hand-fit, quoted there as someone else's numbers.
        //
        // Under Tinted the white is replaced by the theme colour, pulled toward
        // white so it stays legible on the dimmed surface.
        let primary = match tint.as_ref() {
            Some(t) => t
                .blendedColorWithFraction_ofColor(0.35, &srgb_white(1.0))
                .unwrap_or_else(|| srgb_white(1.0)),
            None => srgb_white(1.0),
        };
        let dim = primary.colorWithAlphaComponent(0.62);

        let ivars = self.ivars();
        ivars
            .title
            .setTextColor(Some(&primary.colorWithAlphaComponent(0.85)));
        ivars.title.setStringValue(&NSString::from_str(&format!(
            "macos-liquid-glass · {} — {}",
            ivars.identity.chrome_name(),
            style
        )));

        for (i, f) in ivars.rows.borrow().iter().enumerate() {
            match SCRIPT.get(i) {
                Some((text, is_prompt)) => {
                    // Each example's transcript names itself.
                    let text = text.replace("{NAME}", ivars.identity.chrome_name());
                    f.setStringValue(&NSString::from_str(&text));
                    f.setTextColor(Some(if *is_prompt { &primary } else { &dim }));
                }
                None => f.setStringValue(&NSString::from_str("")),
            }
        }

        self.setNeedsDisplay(true);
    }
}

// -------------------------------------------------------------- app glue

/// Which example is running.
///
/// The two differ ONLY in this: same surface, same knobs, same drawing code.
#[derive(Clone, Copy)]
pub struct Identity {
    /// Build a borderless window rather than a normal titled one.
    pub borderless: bool,
}

impl Identity {
    /// `"borderless"` or `"titled"` — also the example's own name, shown in the
    /// band so a screenshot says which of the two it is without needing the
    /// window frame for context.
    pub fn chrome_name(self) -> &'static str {
        if self.borderless {
            "borderless"
        } else {
            "titled"
        }
    }
}

#[derive(Default)]
struct DelegateState {
    identity: Cell<Option<Identity>>,
    window: RefCell<Option<GlassWindow>>,
    glass: RefCell<Option<GlassSurface>>,
    content: RefCell<Option<Retained<TerminalContentView>>>,
    /// Must be held: dropping it unregisters the KVO observers and invalidates
    /// the reconcile timer, so the window would silently stop following the
    /// setting.
    observer: RefCell<Option<StyleObserver>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GlassTermAppDelegate"]
    #[ivars = DelegateState]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let mtm = MainThreadMarker::from(self);
            let id = self
                .ivars()
                .identity
                .get()
                .expect("identity set before launch");
            let window = if id.borderless {
                GlassWindow::borderless(mtm, SIZE, id.chrome_name())
            } else {
                GlassWindow::new(mtm, SIZE, id.chrome_name())
            };
            let frame = NSRect::new(NSPoint::new(0.0, 0.0), SIZE);

            // Exactly ONE glass surface, as the window's whole content view,
            // with all content inside it. Never glass on glass.
            // `unwrap_or_else` rather than `expect`: the crate owns the
            // explanation now, and `expect` would print it with `Debug`
            // ("Unsupported") instead of the sentence.
            let glass = GlassSurface::new(mtm, frame, glass_style(), CORNER_RADIUS)
                .unwrap_or_else(|e| panic!("{e}"));

            let content = TerminalContentView::new(mtm, frame, id);

            glass.set_content_view(&content);
            window.set_content_view(glass.view());

            match origin_from_env() {
                Some(origin) => window.set_origin(origin),
                None => window.center(),
            }
            window.set_close_on_escape(true);

            // Follow the setting live. The callback fires once immediately, so
            // constructing the observer is also what applies the style — and it
            // must happen BEFORE the window is shown.
            //
            // Ordered the other way round, the window is on screen for one or
            // more frames with no material tint applied. Measured: a capture
            // that raced that gap read the band 3.9 luma brighter than a
            // settled one, which is 40x the real difference between this build
            // and the Swift original and would have been read as a porting
            // error.
            let win_for_cb = window.clone();
            let glass_for_cb = glass.clone();
            let content_for_cb = content.clone();
            let observer = StyleObserver::new(mtm, reconcile(), move |style| {
                apply(&win_for_cb, &glass_for_cb, &content_for_cb, style);
            });

            window.show();

            // Without this the app stays behind whatever had focus, and every
            // capture silently shows a NON-KEY window. The material renders
            // differently when the window is not key, so that is not a
            // cosmetic difference — it invalidates the reading.
            NSApplication::sharedApplication(mtm).activate();

            self.ivars().window.replace(Some(window));
            self.ivars().glass.replace(Some(glass));
            self.ivars().content.replace(Some(content));
            self.ivars().observer.replace(Some(observer));
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            true
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateState::default());
        unsafe { msg_send![super(this), init] }
    }
}

/// Push a resolved style through to the window and its content.
///
/// The window's appearance is set BEFORE the content re-reads its own, so a
/// style change applies one light/dark value to both. That is narrower than
/// "they cannot disagree": under an `*Automatic` token the appearance is
/// inherited, and a *system* light/dark flip reaches the view callback and the
/// material independently. It is still an improvement on an earlier revision
/// of this app (82cf233), which read the *app's* appearance in one place and a
/// view's in another; the Swift oracle read the window's in both.
fn apply(
    window: &GlassWindow,
    glass: &GlassSurface,
    content: &TerminalContentView,
    style: &WidgetStyle,
) {
    window.set_appearance(style.appearance().as_deref());
    content.apply_style(style);

    // Resolved from the WINDOW's appearance, which is the same source the
    // content view's own `resolve_is_dark` reads, so a style change applies one
    // value to both. Under an `*Automatic` token a *system* appearance flip
    // still arrives independently at each, so this is a shared source rather
    // than a guarantee they can never differ.
    let is_dark = window.is_dark();

    // Surface darkening goes on the MATERIAL, not as a layer over it, so the
    // title bar and the body share one surface.
    let (tint_color, gt_applied) = material_tint(is_dark);
    glass.set_tint_color(tint_color.as_deref());

    window.set_has_shadow(want_shadow(is_dark));
    // The log reports the values ACTUALLY USED, which is why `gt` is whatever
    // `material_tint` resolved rather than a re-read of GN_GT_D/GN_GT_L. Those
    // two are only consulted when GN_GTINT is unset; printing them regardless
    // would report two alphas that were not applied and never name the one that
    // was — exactly the "inert knob" defect R19 records in the Swift, where the
    // harness's only per-run record named knobs that reached no drawing code.
    eprintln!(
        "[{}] paperD={} paperL={} tint={} tbody={} gt={} hue={} hsat={} \
         poll={:?} glass={} shadow={} {} tinted={} dark={}",
        content.identity().chrome_name(),
        paper_dark(),
        paper_light(),
        tint_alpha(),
        tint_body_share(),
        gt_applied,
        env_f64("GN_HUE", 0.60),
        env_f64("GN_HSAT", 0.80),
        reconcile(),
        glass_style_name(),
        want_shadow(is_dark),
        style,
        style.is_tinted(),
        is_dark,
    );
}

/// `GN_POLL` — seconds between forced-sync reconcile passes. `0` disables the
/// pass entirely and relies on KVO alone.
///
/// An earlier revision of THIS app (82cf233) clamped this unconditionally to a
/// 0.05s floor, so `GN_POLL=0` meant 20 syncs a second — the exact opposite of
/// what the name implies (R19). The Swift oracle guarded it correctly.
fn reconcile() -> Reconcile {
    let v = env_f64("GN_POLL", 0.75);
    if v <= 0.0 {
        return Reconcile::KvoOnly;
    }
    // `try_from_secs_f64`, not `from_secs_f64`: the panicking constructor
    // rejects +inf and anything at or above u64::MAX seconds, and this is
    // evaluated inside `applicationDidFinishLaunching:`, so the panic unwinds
    // into Objective-C and ABORTS rather than reporting a bad knob. Measured:
    // Measured here, running the built binary directly: GN_POLL=inf and
    // GN_POLL=1e300 both died, printing "libc++abi: terminating due to uncaught
    // foreign exception". A later audit could not reproduce that exact text and
    // saw an ordinary Rust panic exiting 101 instead — the two runs used
    // different harnesses and the discrepancy is unresolved. What is not in
    // doubt is that both values killed the process before the window appeared.
    // A value Duration cannot represent now means what GN_POLL=0 means.
    std::time::Duration::try_from_secs_f64(v.max(0.05)).map_or(Reconcile::KvoOnly, Reconcile::Every)
}

/// The material darkening, applied to the MATERIAL via `tintColor` rather than
/// as an opaque layer over it, so the title bar and the body share one surface.
///
/// Appearance-dependent: a black tint strong enough for Dark would wrongly
/// darken Light too. Returns `None` when the strength is zero.
///
/// The colour stays a dark, slightly saturated blue rather than pure black.
/// Measured at 0.50/0.70/0.85 alpha, a BLACK tint left the blue ratio B/R
/// pinned at 1.45 while luma fell 44.5 → 27.0 → 13.3: black scales all channels
/// equally, so it darkens without shifting hue, and darkening more costs colour.
/// A real widget sits at B/R 2.6–2.8. This is a preference, not a match.
/// Returns the colour and the alpha it resolved, so the caller can log the
/// value that was actually applied rather than re-deriving it and getting it
/// wrong when `GN_GTINT` overrides the pair.
fn material_tint(is_dark: bool) -> (Option<Retained<NSColor>>, f64) {
    // Distinguish UNSET from set-to-zero. Reading it as `env_f64(.., 0.0)` and
    // testing `> 0.0` made GN_GTINT=0 indistinguishable from absent, so it
    // silently fell through to the appearance-dependent pair instead of
    // turning the material darkening off — a knob that reports one thing and
    // does another, which is the defect class R19 records.
    let forced = std::env::var("GN_GTINT")
        .ok()
        .and_then(|s| s.parse::<f64>().ok());
    let alpha = if let Some(forced) = forced {
        forced.max(0.0)
    } else if is_dark {
        env_f64("GN_GT_D", 0.54)
    } else {
        env_f64("GN_GT_L", 0.14)
    };
    let color = (alpha > 0.0).then(|| {
        hsb_to_srgb(
            env_f64("GN_HUE", 0.60),
            env_f64("GN_HSAT", 0.80),
            0.10,
            alpha,
        )
    });
    (color, alpha)
}

/// `GN_SHADOW` — `auto` (default), `always`, or `never`.
///
/// `auto` casts a shadow only under a dark appearance. Over a LIGHT material a
/// window shadow draws as a hard black outline, which none of the system's own
/// light glass surfaces have — they get their definition from a faint bright
/// rim instead, which a window shadow cannot imitate. Dark keeps it: the shadow
/// ends the glass edge at a boundary rather than letting it dissolve into the
/// desktop over several pixels, and that boundary is most of what makes a glass
/// surface read as an object rather than a bright patch of wallpaper.
///
/// This is the one deliberate behavioural divergence from the Swift original,
/// which set `hasShadow` unconditionally. It is a knob rather than a hardcoded
/// choice precisely so the divergence can be measured: `GN_SHADOW=always`
/// reproduces the Swift exactly, and the A/B run under a light style is
/// pixel-identical with it set.
fn want_shadow(is_dark: bool) -> bool {
    match std::env::var("GN_SHADOW").as_deref() {
        Ok("always") => true,
        Ok("never") => false,
        _ => is_dark,
    }
}

/// `GN_GLASS` — force `clear` or `regular`.
///
/// Clear is the default. Apple's HIG (*Materials*) pairs the clear style with a
/// dimming layer over bright content, which is what the terminal body is; the
/// SDK header documents nothing beyond the style name.
///
/// Observed, not measured: `regular` renders as a flat near-white slab in Aqua
/// with no glass left in it, unlike the reference widget, which under
/// Clear ▸ Light is translucent with the wallpaper legible through it. §4 still
/// records the clear-vs-regular comparison as open.
fn glass_style() -> GlassStyle {
    match glass_style_name().as_str() {
        "regular" => GlassStyle::Regular,
        _ => GlassStyle::Clear,
    }
}

fn glass_style_name() -> String {
    std::env::var("GN_GLASS").unwrap_or_else(|_| "clear".into())
}

/// Read an `f64` knob from the environment, falling back to the default
/// documented on each knob above — measured where `MEASUREMENTS.md` records
/// one, inherited from the Swift original where it does not. Knobs are
/// environment variables so a value can be swept rather than guessed.
fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// `GN_ORIGIN=x,y` in AppKit screen coordinates. Malformed input is ignored
/// rather than fatal — a harness sweeping values should not be able to stop the
/// window appearing at all.
///
/// "Parses as f64" is NOT sufficient for that promise. `nan`, `inf` and any
/// coordinate large enough to push the frame outside AppKit's INT_MIN..INT_MAX
/// box all parse, and `setFrameOrigin:` then fails an internal assertion
/// (`_NSWindowSetFrameIvar`, NSWindow.m:1076) and raises. The raise unwinds out
/// of `applicationDidFinishLaunching:`, so the observer is never installed, the
/// window never shows, and the record line is never printed.
///
/// What happens next was measured twice, differently, and is UNRESOLVED: one
/// run recorded the process surviving on the GN_SECS timer and exiting 0 with a
/// zero-byte log; another recorded an immediate SIGABRT (exit 134,
/// "Rust cannot catch foreign exceptions"), on the grounds that the raise
/// cannot unwind through this Rust frame. Either way the run produces no
/// window and no record line, which is what the guard below prevents.
///
/// Measured boundary on the x axis: `2147483087,0` places the window (the frame
/// ends exactly at INT_MAX); `2147483088,0` produced the silent zero-byte run.
/// A mistyped exponent reaches that, so this is not only about `nan`.
fn origin_from_env() -> Option<NSPoint> {
    let raw = std::env::var("GN_ORIGIN").ok()?;
    let (x, y) = raw.split_once(',')?;
    let x: f64 = x.trim().parse().ok()?;
    let y: f64 = y.trim().parse().ok()?;
    if !finite_coordinate(x) || !finite_coordinate(y) {
        eprintln!("GN_ORIGIN={raw:?} is out of range; centring instead");
        return None;
    }
    Some(NSPoint::new(x, y))
}

/// Whether a coordinate keeps the window frame inside the box AppKit asserts on.
///
/// Conservative by the window's own extent, so the check holds for the largest
/// dimension the app uses rather than only for a point.
fn finite_coordinate(v: f64) -> bool {
    const LIMIT: f64 = i32::MAX as f64;
    let margin = SIZE.width.max(SIZE.height);
    v.is_finite() && v <= LIMIT - margin && v >= -LIMIT + margin
}

/// Build the minimum main menu that makes the app quittable.
///
/// A borderless window has no close button, and `NSApplication` routes Cmd-Q
/// through the main menu — so without a menu there is no way to quit short of
/// killing the process.
fn install_menu(app: &NSApplication, mtm: MainThreadMarker, id: Identity) {
    let menubar = NSMenu::new(mtm);
    let app_item = NSMenuItem::new(mtm);
    menubar.addItem(&app_item);

    let app_menu = NSMenu::new(mtm);

    // No Close item: `performClose:` on a borderless window has no close
    // button to act on, so it beeps and does nothing. Escape covers it instead.
    // SAFETY: `terminate:` is a standard NSApplication action, and the menu
    // item is owned by the menu handed to NSApplication below.
    let quit = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(&format!("Quit {}", id.chrome_name())),
            Some(sel!(terminate:)),
            &NSString::from_str("q"),
        )
    };
    app_menu.addItem(&quit);

    app_item.setSubmenu(Some(&app_menu));
    app.setMainMenu(Some(&menubar));
}

/// Run the example under the given identity.
pub fn run(identity: Identity) {
    let mtm = MainThreadMarker::new().expect("run() must be called from main()");
    let app = NSApplication::sharedApplication(mtm);
    install_menu(&app, mtm, identity);

    // Regular, not Accessory: an Accessory app gets no menu bar, and the Quit
    // item installed above is the only way out besides Escape. (An accessory
    // app *can* be activated programmatically — NSRunningApplication.h — so
    // activation is not the reason.)
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let delegate = AppDelegate::new(mtm);
    delegate.ivars().identity.set(Some(identity));
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    // `GN_SECS` — self-terminate, so a capture harness cannot leave a drift of
    // orphaned windows behind across a sweep.
    if let Some(secs) = std::env::var("GN_SECS").ok().and_then(|s| s.parse().ok()) {
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            NSApplication::sharedApplication(mtm).terminate(None);
        });
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(secs, false, &block) };
    }

    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GN_ORIGIN` reaches `setFrameOrigin:`, which fails an internal AppKit
    /// parameter assertion and RAISES for a non-finite coordinate or one that
    /// pushes the frame outside the INT_MIN..INT_MAX box. That raise unwinds
    /// out of `applicationDidFinishLaunching:`, so the window never appears —
    /// which is exactly what this function's own doc promises cannot happen.
    ///
    /// The measured boundary is a containment test, `x + width <= INT_MAX`, so
    /// the guard has to account for the window's own extent. A point-only check
    /// would admit the first failing value.
    #[test]
    fn origin_rejects_what_appkit_would_raise_on() {
        // SAFETY: single-threaded test; each value is read before the next set.
        unsafe {
            for bad in ["nan,nan", "inf,0", "0,inf", "1e308,0", "2147483647,0"] {
                std::env::set_var("GN_ORIGIN", bad);
                assert!(
                    origin_from_env().is_none(),
                    "GN_ORIGIN={bad} must be rejected, not passed to setFrameOrigin:"
                );
            }
            for good in ["1,2", " 1 , 2 ", "-100,-50", "0,0"] {
                std::env::set_var("GN_ORIGIN", good);
                assert!(
                    origin_from_env().is_some(),
                    "GN_ORIGIN={good} must be accepted"
                );
            }
            for malformed in ["1,2,3", "1,", ",2", "", "1;2", "abc,2"] {
                std::env::set_var("GN_ORIGIN", malformed);
                assert!(
                    origin_from_env().is_none(),
                    "GN_ORIGIN={malformed} must fall through to centring"
                );
            }
            std::env::remove_var("GN_ORIGIN");
        }
    }
}
