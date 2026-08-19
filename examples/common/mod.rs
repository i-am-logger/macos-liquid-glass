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
use macos_liquid_glass::icon_style::{StyleObserver, WidgetStyle};
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
/// Height of the drawn title band, in points. Chosen for this window's
/// proportions; it is not read from AppKit.
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
// `GN_PAPER_D` 0.22 and `GN_PAPER_L` 0.12 are tuned against the system widget
// over one wallpaper rather than derived from a measurement, so they are not
// necessarily right on another desktop. Each alpha below is overridable at run
// time by the environment variable named in its doc.

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
/// 0.20 is **not measured**; it is a preference. The band/body split below is
/// measured, this alpha is not.
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
// The flat fills below are built in sRGB explicitly, so the alpha constants
// have a defined colour space rather than whatever `NSColor.black` happens to
// be (Generic Gray Gamma 2.2, which cannot carry a hue at all).
//
// Not every colour on the surface is sRGB. `blendedColorWithFraction:ofColor:`
// returns Generic RGB, so the Tinted text colours are Generic RGB whatever
// their inputs; the theme wash is used in whichever space the system hands it
// over in unless `GN_TINT_SPACE=srgb`.
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
    #[name = "MacosLiquidGlassExampleContentView"]
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
        /// Laying out only once from `init` leaves the centred title and the
        /// monospaced rows at their original widths after any edge drag.
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
        /// The documented route, and it fires for every cause — the system
        /// setting, the window's own `appearance` being assigned, and a move
        /// between screens. Inferring light/dark from KVO on
        /// `AppleInterfaceStyle` misses the last two.
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
    /// The crate's free resolver, which takes an appearance rather than a
    /// window and so applies to an `NSView` too.
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
        // and the body at `GN_TBODY` of it.
        //
        // Drawn here rather than via `NSGlassEffectView.tintColor`, which
        // despite being documented as a bias floods the surface to a flat
        // saturated slab at full strength.
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
        // dimming it: swapping to NSRectFill makes the whole-window MAE 18x
        // worse (0.0082 against 0.00045) while changing the band not at all.
        NSBezierPath::fillRect(paper);

        NSGraphicsContext::restoreGraphicsState_class();
    }

    /// Set the text, and the appearance the paper should dim for.
    fn apply_style(&self, style: &WidgetStyle) {
        self.ivars().is_dark.set(self.resolve_is_dark());

        // `GN_TINT_SPACE=srgb` converts the system's theme colour into sRGB
        // before washing with it; `native` (the default) uses it exactly as
        // handed back, which is what the system itself does. Converting spreads
        // a small brightness shift across the band, the body and the text;
        // native confines it to the band.
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
    #[name = "MacosLiquidGlassExampleAppDelegate"]
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
            // more frames with no material tint applied. A capture that races
            // that gap reads the band 3.9 luma brighter than a settled one.
            let win_for_cb = window.clone();
            let glass_for_cb = glass.clone();
            let content_for_cb = content.clone();
            let observer = StyleObserver::new(mtm, move |style| {
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
/// material independently. Read the window's appearance in both places; reading
/// the app's in one and a view's in the other lets them disagree.
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
    // was, which is how a knob comes to look live while reaching no drawing
    // code.
    eprintln!(
        "[{}] paperD={} paperL={} tint={} tbody={} gt={} hue={} hsat={} \
         glass={} shadow={} {} tinted={} dark={}",
        content.identity().chrome_name(),
        paper_dark(),
        paper_light(),
        tint_alpha(),
        tint_body_share(),
        gt_applied,
        env_f64("GN_HUE", 0.60),
        env_f64("GN_HSAT", 0.80),
        glass_style_name(),
        want_shadow(is_dark),
        style,
        style.is_tinted(),
        is_dark,
    );
}

/// The material darkening, applied to the MATERIAL via `tintColor` rather than
/// as an opaque layer over it, so the title bar and the body share one surface.
///
/// Appearance-dependent: a black tint strong enough for Dark would wrongly
/// darken Light too. Returns `None` when the strength is zero.
///
/// The colour is a dark, slightly saturated blue (`GN_HUE` 0.60, `GN_HSAT`
/// 0.80) rather than pure black: black scales every channel equally, so it
/// cannot shift the material's hue, only scale the colour down — the harder it
/// darkens, the less colour survives.
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
/// Clear ▸ Light is translucent with the wallpaper legible through it.
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
/// documented on each knob above. Knobs are environment variables so a value
/// can be swept rather than guessed.
fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// `GN_ORIGIN=x,y` in AppKit screen coordinates. Malformed or out-of-range
/// input is ignored rather than fatal, so a bad value cannot stop the window
/// appearing at all.
///
/// "Parses as f64" is NOT sufficient for that promise. `nan`, `inf` and any
/// coordinate large enough to push the frame outside AppKit's INT_MIN..INT_MAX
/// box all parse, and `setFrameOrigin:` then fails an internal assertion
/// (`_NSWindowSetFrameIvar`, NSWindow.m:1076) and raises. The raise unwinds out
/// of `applicationDidFinishLaunching:`, so the observer is never installed and
/// the window never shows.
///
/// The limit is a containment test on the whole frame, `x + width <= INT_MAX`,
/// not on the point alone — see `finite_coordinate`. A mistyped exponent
/// reaches it, so this is not only about `nan`.
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
