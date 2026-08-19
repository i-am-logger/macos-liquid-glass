//! Tracking **System Settings ▸ Appearance ▸ Icon & widget style**.
//!
//! This is the setting that restyles desktop widgets, and it is surprisingly
//! hard to follow correctly. Three things about it are not obvious, and each
//! cost a measurement to establish:
//!
//! 1. **The four-option UI is nine tokens.** `AppleIconAppearanceTheme` carries
//!    a style family *and* a light/dark/automatic axis in one string. Branch on
//!    the nine, never on the four. Choosing "Default" **removes** the key rather
//!    than writing a value, so an absent key is a meaningful state.
//! 2. **`defaults write` does not apply it.** The value changes and the system
//!    ignores it; only the real UI path takes effect. Any test loop has to drive
//!    System Settings and read the key back.
//! 3. **No notification centre posts a change.** Not `NSWorkspace`'s, not
//!    `NotificationCenter.default`, not `DistributedNotificationCenter`, and not
//!    SkyLight's own coordinated-change notification. KVO on the global domain
//!    is the only event source that fires — see [`observer`].
//!
//! [`observer`]: StyleObserver

use core::fmt;
use core::str::FromStr;
use std::cell::{Cell, RefCell};
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
#[cfg(feature = "private-spi")]
use objc2_app_kit::NSWorkspace;
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColor};
use objc2_core_foundation::{
    CFPreferencesAppSynchronize, CFPreferencesCopyValue, CFString, kCFPreferencesAnyApplication,
    kCFPreferencesAnyHost, kCFPreferencesCurrentUser,
};
use objc2_foundation::{
    MainThreadMarker, NSKeyValueObservingOptions, NSObject, NSObjectNSKeyValueObserverRegistration,
    NSObjectProtocol, NSString, NSTimer, NSUserDefaults,
};

/// The preference key the Appearance pane writes.
pub const ICON_APPEARANCE_KEY: &str = "AppleIconAppearanceTheme";

/// How a style renders, which is coarser than which button the user pressed.
///
/// This is a **lossy** grouping, deliberately: for a desktop widget, Default and
/// Clear render identically (MAE 0), as do Dark and Clear ▸ Dark, so the family
/// only has to separate untinted from tinted. The light/dark half is on
/// [`ThemeMode`].
///
/// Because it is lossy, [`Regular`] covers two different Settings selections —
/// Default and the Dark style with its Auto sub-option — which render the same
/// but are not the same choice. If you need to know exactly which of the nine
/// the user picked, match on [`WidgetStyle::token`], not on this.
///
/// [`Regular`]: IconStyle::Regular
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IconStyle {
    /// "Default".
    Regular,
    /// "Dark".
    RegularDark,
    /// "Clear".
    Clear,
    /// "Tinted" — the only family that carries a theme colour.
    Tinted,
}

/// The light/dark axis, which is independent of the family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ThemeMode {
    /// Force light.
    Light,
    /// Force dark.
    Dark,
    /// Follow the system appearance, and keep following it as it changes.
    Auto,
}

impl ThemeMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::Auto => "Auto",
        }
    }
}

/// One of the nine values `AppleIconAppearanceTheme` can hold.
///
/// The four-option Settings pane writes **nine** distinct tokens, because the
/// style family and the light/dark axis are encoded together in one string.
/// Branch on the nine, never on the four.
///
/// [`RegularLight`] is inferred rather than observed: choosing Default
/// *removes* the key instead of writing that string, so no value can be read
/// back for it. Note that `"Dark"` is not a token — the Dark selection writes
/// [`RegularDark`].
///
/// A plain enum rather than a newtype over an integer, so a `match` is checked.
/// `#[non_exhaustive]`, so a tenth Settings option can ship in a minor release
/// rather than forcing a major one on every consumer.
///
/// [`RegularLight`]: IconAppearanceToken::RegularLight
/// [`RegularDark`]: IconAppearanceToken::RegularDark
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum IconAppearanceToken {
    /// `"RegularAutomatic"` — 0. The *Dark* style with its Auto sub-option,
    /// **not** Default. See [`WidgetStyle`]'s `Display`.
    RegularAutomatic,
    /// `"RegularLight"` — 1. Default. Inferred; the key is absent in this state.
    RegularLight,
    /// `"RegularDark"` — 2.
    RegularDark,
    /// `"ClearAutomatic"` — 3.
    ClearAutomatic,
    /// `"ClearLight"` — 4.
    ClearLight,
    /// `"ClearDark"` — 5.
    ClearDark,
    /// `"TintedAutomatic"` — 6.
    TintedAutomatic,
    /// `"TintedLight"` — 7.
    TintedLight,
    /// `"TintedDark"` — 8.
    TintedDark,
}

impl IconAppearanceToken {
    /// Every token, in preference-value order.
    ///
    /// A slice, not an array: the length is not part of the type, so a tenth
    /// token does not change this item's signature.
    pub const ALL: &'static [Self] = &[
        Self::RegularAutomatic,
        Self::RegularLight,
        Self::RegularDark,
        Self::ClearAutomatic,
        Self::ClearLight,
        Self::ClearDark,
        Self::TintedAutomatic,
        Self::TintedLight,
        Self::TintedDark,
    ];

    /// The exact string the Appearance pane writes.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::RegularAutomatic => "RegularAutomatic",
            Self::RegularLight => "RegularLight",
            Self::RegularDark => "RegularDark",
            Self::ClearAutomatic => "ClearAutomatic",
            Self::ClearLight => "ClearLight",
            Self::ClearDark => "ClearDark",
            Self::TintedAutomatic => "TintedAutomatic",
            Self::TintedLight => "TintedLight",
            Self::TintedDark => "TintedDark",
        }
    }

    /// The `iconAppearanceTheme` integer for this token.
    #[must_use]
    pub fn raw(self) -> i64 {
        match self {
            Self::RegularAutomatic => 0,
            Self::RegularLight => 1,
            Self::RegularDark => 2,
            Self::ClearAutomatic => 3,
            Self::ClearLight => 4,
            Self::ClearDark => 5,
            Self::TintedAutomatic => 6,
            Self::TintedLight => 7,
            Self::TintedDark => 8,
        }
    }

    /// The style family this token selects.
    #[must_use]
    pub fn style(self) -> IconStyle {
        match self {
            Self::RegularAutomatic | Self::RegularLight => IconStyle::Regular,
            Self::RegularDark => IconStyle::RegularDark,
            Self::ClearAutomatic | Self::ClearLight | Self::ClearDark => IconStyle::Clear,
            Self::TintedAutomatic | Self::TintedLight | Self::TintedDark => IconStyle::Tinted,
        }
    }

    /// The light/dark axis this token selects.
    #[must_use]
    pub fn mode(self) -> ThemeMode {
        match self {
            Self::RegularAutomatic | Self::ClearAutomatic | Self::TintedAutomatic => {
                ThemeMode::Auto
            }
            Self::RegularLight | Self::ClearLight | Self::TintedLight => ThemeMode::Light,
            Self::RegularDark | Self::ClearDark | Self::TintedDark => ThemeMode::Dark,
        }
    }

    /// Whether this token's family carries a theme colour.
    #[must_use]
    pub fn is_tinted(self) -> bool {
        self.style() == IconStyle::Tinted
    }
}

impl fmt::Display for IconAppearanceToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for IconAppearanceToken {
    type Err = UnknownToken;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|t| t.name() == s)
            .ok_or_else(|| UnknownToken(s.to_owned()))
    }
}

impl TryFrom<i64> for IconAppearanceToken {
    type Error = UnknownThemeValue;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::ALL
            .iter()
            .copied()
            .find(|t| t.raw() == value)
            .ok_or(UnknownThemeValue(value))
    }
}

/// The preference string was not one of the nine tokens.
///
/// Distinct from [`UnknownThemeValue`] on purpose: an unrecognised *string* and
/// an out-of-range *integer* are different failures with different fallbacks,
/// and one error type carrying both would have to stringify the integer to
/// report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownToken(String);

impl UnknownToken {
    /// The string that was not recognised.
    #[must_use]
    pub fn found(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UnknownToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown icon appearance token {:?}", self.0)
    }
}

impl core::error::Error for UnknownToken {}

/// The `iconAppearanceTheme` integer was not one of 0..=8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownThemeValue(i64);

impl UnknownThemeValue {
    /// The integer that was out of range.
    #[must_use]
    pub fn found(&self) -> i64 {
        self.0
    }
}

impl fmt::Display for UnknownThemeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "icon appearance theme {} is out of range 0..=8", self.0)
    }
}

impl core::error::Error for UnknownThemeValue {}

/// A resolved Icon & widget style: which of the nine the user chose, plus the
/// theme colour if that choice carries one.
///
/// Exactly nine states are representable, because the token is the state and
/// the family and light/dark axis are read back off it. Storing [`IconStyle`]
/// and [`ThemeMode`] separately would make twelve combinations constructible
/// for nine real ones — `RegularDark` with `Light` is a style named "Dark" that
/// renders light.
#[derive(Debug, Clone)]
pub struct WidgetStyle {
    token: IconAppearanceToken,
    tint: Option<Retained<NSColor>>,
}

impl WidgetStyle {
    /// Build a style from a token and, for the tinted family, its theme colour.
    ///
    /// A `tint` passed with a non-tinted token is dropped rather than stored:
    /// only [`IconStyle::Tinted`] carries a colour, and keeping one would make
    /// two styles that render identically compare unequal.
    #[must_use]
    pub fn from_token(token: IconAppearanceToken, tint: Option<Retained<NSColor>>) -> Self {
        let tint = if token.is_tinted() { tint } else { None };
        Self { token, tint }
    }

    /// Which of the nine the user chose.
    #[must_use]
    pub fn token(&self) -> IconAppearanceToken {
        self.token
    }

    /// The style family.
    #[must_use]
    pub fn style(&self) -> IconStyle {
        self.token.style()
    }

    /// The light/dark axis.
    #[must_use]
    pub fn mode(&self) -> ThemeMode {
        self.token.mode()
    }

    /// The resolved theme colour. Only ever `Some` under [`IconStyle::Tinted`].
    ///
    /// Always `None` without the `private-spi` feature: the colour's only
    /// source is an undocumented selector. The style itself still resolves.
    #[must_use]
    pub fn tint(&self) -> Option<&NSColor> {
        self.tint.as_deref()
    }

    /// Whether the theme colour applies.
    #[must_use]
    pub fn is_tinted(&self) -> bool {
        self.token.is_tinted()
    }

    /// The appearance to force, or `None` to inherit the system's.
    ///
    /// Forcing is correct here, unlike for a window that merely follows the
    /// system: the icon/widget style carries its own light/dark, independent of
    /// the system appearance. Under "Clear ▸ Dark" the surface stays dark even
    /// while the system appearance is Light.
    ///
    /// `None` under [`ThemeMode::Auto`] means *inherit*, not *unknown* — to
    /// resolve it to an actual light/dark, use [`is_dark`].
    ///
    /// [`is_dark`]: WidgetStyle::is_dark
    #[must_use]
    pub fn appearance(&self) -> Option<Retained<NSAppearance>> {
        let name = match self.mode() {
            ThemeMode::Light => unsafe { NSAppearanceNameAqua },
            ThemeMode::Dark => unsafe { NSAppearanceNameDarkAqua },
            ThemeMode::Auto => return None,
        };
        NSAppearance::appearanceNamed(name)
    }

    /// Whether this style renders dark, resolving [`ThemeMode::Auto`] against
    /// an ambient appearance.
    ///
    /// This is the half [`appearance`] cannot answer. A host painting its own
    /// chrome hits `Auto` on its first real style change and otherwise has to
    /// reimplement the resolver — see `macos_liquid_glass::is_dark` for why the obvious
    /// implementation is wrong.
    ///
    /// [`appearance`]: WidgetStyle::appearance
    #[must_use]
    pub fn is_dark(&self, ambient: &NSAppearance) -> bool {
        match self.mode() {
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
            ThemeMode::Auto => crate::is_dark(ambient),
        }
    }
}

/// The name shown for the selection.
///
/// [`IconAppearanceToken::RegularAutomatic`] and
/// [`IconAppearanceToken::RegularLight`] are both in the Regular family but are
/// different UI selections: the first is the Dark style with its Auto
/// sub-option, the second is Default. They must not render the same string.
///
/// Width and fill specifiers are ignored, as for most hand-written `Display`
/// impls.
impl fmt::Display for WidgetStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.style(), self.mode()) {
            (IconStyle::Regular, ThemeMode::Auto) => f.write_str("Dark ▸ Auto"),
            (IconStyle::Regular, _) => f.write_str("Default"),
            (IconStyle::RegularDark, _) => f.write_str("Dark"),
            (IconStyle::Clear, m) => write!(f, "Clear ▸ {}", m.as_str()),
            (IconStyle::Tinted, m) => write!(f, "Tinted ▸ {}", m.as_str()),
        }
    }
}

impl PartialEq for WidgetStyle {
    fn eq(&self, other: &Self) -> bool {
        if self.token != other.token {
            return false;
        }
        match (&self.tint, &other.tint) {
            (None, None) => true,
            (Some(a), Some(b)) => a.isEqual(Some(b)),
            _ => false,
        }
    }
}

/// Resolve an `iconAppearanceTheme` integer, leniently.
///
/// Out-of-range integers resolve to [`IconAppearanceToken::RegularAutomatic`]
/// (enum 0). This is a *different* fallback from the one [`current`] applies to
/// an unrecognised preference *string*, which resolves to enum 1
/// ([`IconAppearanceToken::RegularLight`]). The two are easy to conflate: enum 0
/// displays as "Dark ▸ Auto" and enum 1 as "Default".
///
/// Private, because [`IconAppearanceToken::try_from`] is the boundary a
/// consumer should meet — it reports the bad value rather than silently
/// choosing for them.
fn resolve(theme: i64) -> IconAppearanceToken {
    IconAppearanceToken::try_from(theme).unwrap_or(IconAppearanceToken::RegularAutomatic)
}

/// `NSWorkspace`'s icon appearance configuration.
///
/// `-currentIconAppearanceConfiguration` is exported by AppKit but has no public
/// header, so it is reached by selector. It returns an
/// `SLSIconAppearanceConfiguration` carrying the icon style and the resolved
/// theme colour together — the colour has no preference-key equivalent, which is
/// the only reason this private path is used at all.
#[cfg(feature = "private-spi")]
fn workspace_config() -> Option<Retained<AnyObject>> {
    let workspace = NSWorkspace::sharedWorkspace();
    if !workspace.respondsToSelector(sel!(currentIconAppearanceConfiguration)) {
        return None;
    }
    // SAFETY: the selector is confirmed present immediately above. It takes no
    // arguments and returns an autoreleased object.
    unsafe { msg_send![&*workspace, currentIconAppearanceConfiguration] }
}

/// Without the `private-spi` feature there is no configuration to consult: the
/// only source of the resolved theme colour is an undocumented selector, and
/// App Store Review Guideline 2.5.1 permits public APIs only.
#[cfg(not(feature = "private-spi"))]
fn workspace_config() -> Option<Retained<AnyObject>> {
    None
}

/// Unreachable without `private-spi` — `workspace_config` returns `None` there,
/// so there is never a configuration to read. Defined so `current` compiles in
/// one shape rather than two.
#[cfg(not(feature = "private-spi"))]
fn config_tint(_cfg: &AnyObject) -> Option<Retained<NSColor>> {
    None
}

/// See [`config_tint`].
#[cfg(not(feature = "private-spi"))]
fn config_theme(_cfg: &AnyObject) -> Option<i64> {
    None
}

/// Whether `obj` implements `sel`.
///
/// `AnyObject` does not carry `NSObjectProtocol`, so this asks the runtime
/// directly. A `false` is a value; the KVC alternative raises.
#[cfg(feature = "private-spi")]
fn responds(obj: &AnyObject, sel: objc2::runtime::Sel) -> bool {
    // SAFETY: `respondsToSelector:` is implemented by NSObject, which every
    // object reaching here descends from.
    unsafe { msg_send![obj, respondsToSelector: sel] }
}

/// The resolved theme colour, or `None` if this OS does not carry it.
///
/// Sent as a selector, NOT read by KVC. `valueForKey:` **raises**
/// `NSUnknownKeyException` for a key that is absent, and Rust cannot catch an
/// Objective-C exception — it aborts the process, uncatchably:
///
/// ```text
/// fatal runtime error: Rust cannot catch foreign exceptions, aborting
/// ```
///
/// That mattered because `SLSIconAppearanceConfiguration` is private and has no
/// availability contract, so its properties are exactly the thing that can move
/// in a point release. Guarding the private *selector* while reading its
/// properties by KVC guarded the cheap half and left the expensive half
/// exposed. `respondsToSelector:` returns a value rather than raising, so this
/// shape has no exception surface at all.
#[cfg(feature = "private-spi")]
fn config_tint(cfg: &AnyObject) -> Option<Retained<NSColor>> {
    if !responds(cfg, sel!(resolvedIconTintColor)) {
        return None;
    }
    // SAFETY: presence confirmed immediately above; a zero-argument getter
    // returning an autoreleased NSColor.
    unsafe { msg_send![cfg, resolvedIconTintColor] }
}

/// The configuration's own `iconAppearanceTheme`, or `None` if absent.
///
/// See [`config_tint`] for why this is a selector send rather than KVC.
#[cfg(feature = "private-spi")]
fn config_theme(cfg: &AnyObject) -> Option<i64> {
    if !responds(cfg, sel!(iconAppearanceTheme)) {
        return None;
    }
    // SAFETY: presence confirmed immediately above; a zero-argument getter
    // returning NSInteger.
    let theme: isize = unsafe { msg_send![cfg, iconAppearanceTheme] };
    Some(theme as i64)
}

/// Read the live Icon & widget style.
///
/// Reads the preference **string**, not the workspace configuration's integer.
/// Both are correct eventually, but the cached configuration lags a real
/// settings change by several seconds in-process — measured still reporting
/// "Clear ▸ Dark" a full 2s after the system had moved to ClearLight. Forcing a
/// CFPreferences sync and reading the global domain follows the change
/// immediately (0.54s against KVO's 3.86s).
///
/// The configuration's integer is kept as the fallback, so if the preference is
/// missing or unparseable a style still resolves.
///
/// # Panics
///
/// Never, but note this must **not** be called synchronously from inside a
/// `UserDefaults` KVO callback: that callback runs while CFPrefs holds its own
/// lock, and reading a preference re-enters it. See [`StyleObserver`].
pub fn current() -> WidgetStyle {
    let cfg = workspace_config();
    let tint = cfg.as_deref().and_then(config_tint);

    CFPreferencesAppSynchronize(unsafe { kCFPreferencesAnyApplication });
    let raw = CFPreferencesCopyValue(
        &CFString::from_str(ICON_APPEARANCE_KEY),
        unsafe { kCFPreferencesAnyApplication },
        unsafe { kCFPreferencesCurrentUser },
        unsafe { kCFPreferencesAnyHost },
    )
    .and_then(|v| v.downcast::<CFString>().ok())
    .map(|s| s.to_string());

    let token = match raw.as_deref() {
        // An unrecognised string means the preference is garbage; an ABSENT key
        // means the user chose Default. Both land on `RegularLight`, but by two
        // different routes, and neither is `resolve`'s out-of-range fallback.
        Some(name) => name.parse().unwrap_or(IconAppearanceToken::RegularLight),
        None => cfg
            .as_deref()
            .and_then(config_theme)
            .map_or(IconAppearanceToken::RegularLight, resolve),
    };

    WidgetStyle::from_token(token, tint)
}

/// The global-domain keys whose changes drive a widget-style surface.
///
/// All three fire KVO on `NSUserDefaults.standard`.
///
/// The theme colour is written to **`AppleAccentColor`**, not to
/// `AppleIconAppearanceTintColor`.
pub const WATCHED_KEYS: &[&str] = &[
    ICON_APPEARANCE_KEY,
    "AppleAccentColor",
    "AppleInterfaceStyle",
];

/// State behind the observer.
struct ObserverIvars {
    on_change: Box<dyn Fn(&WidgetStyle)>,
    /// Not an `Option`: seeded with the live style at construction, so "the
    /// observer exists but has never observed anything" is unrepresentable
    /// rather than merely unreachable.
    applied: RefCell<WidgetStyle>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    /// Cleared by `StyleObserver::drop`.
    ///
    /// Removing the KVO registrations and invalidating the timer stops any
    /// FUTURE delivery, but a `reconcileDeferred` already enqueued by
    /// `performSelectorOnMainThread:` is past the point of cancellation and
    /// still runs on the next run-loop turn — firing `on_change` once after
    /// the caller believed it had unsubscribed. A flag catches that regardless
    /// of the enqueue mechanism, which a cancel call would not.
    alive: Cell<bool>,
}

impl fmt::Debug for ObserverIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `Box<dyn Fn>` cannot be printed, so this is deliberately partial.
        f.debug_struct("ObserverIvars")
            .field("applied", &self.applied)
            .field("ticking", &self.timer.borrow().is_some())
            .field("alive", &self.alive)
            .finish_non_exhaustive()
    }
}

define_class!(
    /// Watches the global domain and reports style changes.
    #[unsafe(super(NSObject))]
    // No `#[name = ...]` — see the note in `window.rs`; a library must let
    // objc2 generate the name so two semver-incompatible copies cannot collide.
    #[thread_kind = MainThreadOnly]
    #[ivars = ObserverIvars]
    #[derive(Debug)]
    struct Observer;

    unsafe impl NSObjectProtocol for Observer {}

    impl Observer {
        /// **Only enqueues. Never reads a preference.**
        ///
        /// KVO callbacks are delivered while CFPrefs holds the CFPrefsSource
        /// lock. Reading a preference from inside one re-enters that lock and
        /// the process is killed by `_os_unfair_lock_recursive_abort`.
        ///
        /// `performSelectorOnMainThread:…:waitUntilDone:NO` is used rather than
        /// a dispatch call because it is documented to enqueue *even when the
        /// current thread is already the main thread*. That property is the
        /// whole point: a "run it inline if we are already on main" helper —
        /// `dispatch2::run_on_main` is one — reads the preference inside the
        /// callback and hits the recursive lock.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            key_path: Option<&NSString>,
            _of_object: Option<&AnyObject>,
            _change: Option<&AnyObject>,
            _context: *mut core::ffi::c_void,
        ) {
            let Some(key) = key_path else { return };
            let key = key.to_string();
            if !WATCHED_KEYS.contains(&key.as_str()) {
                return;
            }
            // SAFETY: `reconcileDeferred` is defined on this class below, and
            // waitUntilDone:NO cannot re-enter the caller.
            unsafe {
                let _: () = msg_send![
                    self,
                    performSelectorOnMainThread: sel!(reconcileDeferred),
                    withObject: core::ptr::null::<AnyObject>(),
                    waitUntilDone: false,
                ];
            }
        }

        /// The deferred half of the KVO callback: CFPrefs has dropped its lock
        /// by the time the run loop gets here, so reading is safe.
        #[unsafe(method(reconcileDeferred))]
        fn reconcile_deferred(&self) {
            if !self.ivars().alive.get() {
                return;
            }
            self.reconcile(true);
        }

        /// The reconcile pass.
        ///
        /// Not redundant with KVO. Measured: after a settings change, the new
        /// value is visible to a forced-sync reader at 0.54s, but KVO delivers
        /// at 3.86s — CFPrefs coalesces cross-process global-domain
        /// notifications by roughly 3s. Forcing the sync is what makes the
        /// response prompt.
        #[unsafe(method(reconcileTick:))]
        fn reconcile_tick(&self, _timer: Option<&AnyObject>) {
            if !self.ivars().alive.get() {
                return;
            }
            self.reconcile(false);
        }
    }
);

impl Observer {
    fn reconcile(&self, force: bool) {
        let want = current();
        let mut applied = self.ivars().applied.borrow_mut();
        if !force && *applied == want {
            return;
        }
        *applied = want.clone();
        // Released before the callback: the callback may re-enter this object
        // and a held RefCell borrow would panic rather than merely conflict.
        drop(applied);
        (self.ivars().on_change)(&want);
    }
}

/// How hard to work at noticing a change.
///
/// How hard to work at noticing a change.
///
/// The forced-sync pass is what makes the response prompt. With the default,
/// a change is delivered in **138–671 ms (median 335)** over five measured
/// changes — a uniform spread across the 750 ms interval, which is what a poll
/// of that period predicts.
///
/// Named rather than an `Option<Duration>` because with an `Option` there is no
/// way to spell "the recommended setting" — `None` means *off* — so every
/// caller ends up hardcoding the same magic number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reconcile {
    /// KVO only, with no forced sync.
    ///
    /// **Not recommended, and may never deliver.** Without the forced sync this
    /// process does not re-read the global domain, so a change written by
    /// another process can go unnoticed: measured against `defaults write`,
    /// two changes over 80 s produced no callback at all. CFPrefs also
    /// coalesces cross-process global-domain notifications by roughly 3 s, so
    /// even when KVO does fire it is seconds behind.
    ///
    /// Use it only when something else in your process already forces a
    /// preference sync.
    KvoOnly,
    /// Also force a CFPreferences sync on this interval.
    ///
    /// Clamped to a 50 ms floor — see [`StyleObserver::new`].
    Every(Duration),
}

impl Default for Reconcile {
    /// `Every(750ms)`, the interval the reference app uses; with it, a
    /// preference change was measured reaching this crate within 1s.
    fn default() -> Self {
        Self::Every(Duration::from_millis(750))
    }
}

/// Watches the Icon & widget style and invokes a callback when it changes.
///
/// Use this when you draw from the style — a theme tint, a dimming layer,
/// anything that branches on [`WidgetStyle::token`]. If all you need is for a
/// [`GlassWindow`] to match the system light/dark, `GlassWindow::follow_icon_style`
/// does that and owns the observer for you, so there is nothing to hold.
///
/// The callback fires once immediately on construction with the current style,
/// so a caller does not have to separately prime itself.
///
/// [`GlassWindow`]: crate::window::GlassWindow
///
/// Dropping this removes the KVO registrations and invalidates the timer.
/// **Something must hold it** — a `StyleObserver` that is not bound to a
/// variable stops delivering changes immediately. Nothing dangles: `Drop`
/// unregisters before the underlying observer is released.
///
/// Note the `#[must_use]` below catches only a *discarded* observer. It cannot
/// catch `let _ = StyleObserver::new(…)`, and nothing else does either — `let _`
/// drops immediately, which is precisely the mistake. Bind a real name.
#[must_use = "dropping a StyleObserver unregisters it and change delivery stops; \
              bind it for as long as you want notifications"]
#[derive(Debug)]
pub struct StyleObserver {
    observer: Retained<Observer>,
}

impl StyleObserver {
    /// Register for changes.
    ///
    /// `reconcile` selects the forced-sync backstop;
    /// [`Reconcile::default()`][Reconcile::default] is the recommended setting
    /// and [`Reconcile::KvoOnly`] switches the pass off. An interval shorter
    /// than 50 ms is raised to 50 ms — see the clamp in the body for why.
    ///
    /// With the default, a change is delivered in 138–671 ms (median 335 over
    /// five measured changes). With [`Reconcile::KvoOnly`] it may not be
    /// delivered at all — see that variant.
    ///
    /// `on_change` is [`Fn`], not [`FnMut`], and that is deliberate. The
    /// callback can re-enter this object — `performSelectorOnMainThread:`
    /// enqueues in common modes, so a reconcile runs inside any nested run loop
    /// the callback starts, such as an alert or a tracking loop — and an
    /// `FnMut` would need a mutable borrow held across the call-out, which
    /// panics on that path. block2 declines to ship the `FnMut` adapter for the
    /// same reason, leaving the choice of `Cell`/`RefCell`/`UnsafeCell` to the
    /// consumer, whose use case determines which is sound.
    ///
    /// The pass is scheduled in `NSDefaultRunLoopMode` (NSTimer.h), so it does
    /// not tick during a live resize drag or while a menu is tracking, and
    /// latency falls back to KVO's for the duration. The KVO hop itself
    /// survives tracking, because `performSelectorOnMainThread:` uses common
    /// modes (NSThread.h).
    ///
    /// # Panics
    ///
    /// Never directly — but a panic escaping `on_change` on the KVO or timer
    /// path unwinds into run-loop frames with no Rust frame to catch it and
    /// aborts (`fatal runtime error: Rust panics must be rethrown`). The
    /// priming call in `new` is made from Rust, so a panic *there* unwinds out
    /// of `new` — which is why `Self` is constructed first. Either way
    /// `reconcile` records the style as applied *before* calling out, so
    /// swallowing the panic would leave the observer permanently claiming a
    /// style it never applied.
    pub fn new(
        mtm: MainThreadMarker,
        reconcile: Reconcile,
        on_change: impl Fn(&WidgetStyle) + 'static,
    ) -> Self {
        // Read ONCE. `applied` is seeded from this and the priming call-out
        // below reuses it, rather than seeding here and letting `reconcile`
        // read again — that cost two `CFPreferencesAppSynchronize` calls plus
        // two workspace reads per construction, for a value overwritten
        // immediately.
        let initial = current();

        let this = Observer::alloc(mtm).set_ivars(ObserverIvars {
            on_change: Box::new(on_change),
            applied: RefCell::new(initial.clone()),
            timer: RefCell::new(None),
            alive: Cell::new(true),
        });
        let observer: Retained<Observer> = unsafe { msg_send![super(this), init] };

        let defaults = NSUserDefaults::standardUserDefaults();
        for key in WATCHED_KEYS {
            // SAFETY: the observer is removed in `Drop` before it is
            // deallocated, so KVO never holds a dangling pointer.
            unsafe {
                defaults.addObserver_forKeyPath_options_context(
                    &observer,
                    &NSString::from_str(key),
                    NSKeyValueObservingOptions::New,
                    core::ptr::null_mut(),
                );
            }
        }

        // Clamped HERE, in the library, not in the caller. `NSTimer` treats an
        // interval <= 0 as 0.1 ms, and each tick is a forced
        // `CFPreferencesAppSynchronize` plus a workspace-configuration read:
        // `Duration::ZERO` runs ~9600 ticks/sec and costs ~6% of a core,
        // against 0.0% at the 0.75s default. The reference app applies the
        // same 50 ms floor, but a consumer using `icon-style` directly does
        // not go through it.
        const FLOOR: Duration = Duration::from_millis(50);
        let interval = match reconcile {
            Reconcile::KvoOnly => None,
            Reconcile::Every(d) => Some(d.max(FLOOR)),
        };

        if let Some(interval) = interval {
            // SAFETY: `reconcileTick:` is defined on Observer and takes the
            // timer as its single argument.
            let timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    interval.as_secs_f64(),
                    &observer,
                    sel!(reconcileTick:),
                    None,
                    true,
                )
            };
            observer.ivars().timer.replace(Some(timer));
        }

        // Construct `Self` BEFORE priming, so the cleanup guard exists before
        // anything can unwind past it.
        //
        // `on_change` is arbitrary caller code. Priming first means a panic in
        // it unwinds out of `new` with the three KVO registrations live and the
        // timer scheduled, but with no `StyleObserver` ever constructed — and
        // `Drop` is the only thing that unregisters. The run loop retains the
        // timer, the timer retains the observer, so the orphan survives with
        // its registrations intact; the next watched-key change re-enters the
        // already-panicking closure and the second panic unwinds through KVO's
        // Objective-C frames into `Rust panics must be rethrown, aborting`.
        //

        // The call-out is made directly rather than through `reconcile(true)`
        // so that the read above is not repeated. Both properties the `# Panics`
        // section rests on are preserved: `Self` exists before any caller code
        // runs, and `applied` already holds `initial` before the call-out, so a
        // panic cannot leave the observer claiming a style it never applied.
        let this = Self { observer };
        (this.observer.ivars().on_change)(&initial);
        this
    }

    /// The most recently observed style.
    ///
    /// A cached clone — cheap, and safe to call from inside the callback. This
    /// is **not** the same operation as the free [`current`], which forces a
    /// CFPreferences sync, performs a workspace read, and must not be called
    /// from a KVO callback. The receiver is the difference; prefer this one
    /// whenever you hold an observer.
    #[must_use]
    pub fn current(&self) -> WidgetStyle {
        self.observer.ivars().applied.borrow().clone()
    }
}

impl Drop for StyleObserver {
    fn drop(&mut self) {
        // First, so an already-enqueued deferred reconcile becomes a no-op.
        self.observer.ivars().alive.set(false);
        let defaults = NSUserDefaults::standardUserDefaults();
        for key in WATCHED_KEYS {
            // SAFETY: these are exactly the registrations made in `new`.
            unsafe {
                defaults.removeObserver_forKeyPath(&self.observer, &NSString::from_str(key));
            }
        }
        if let Some(timer) = self.observer.ivars().timer.borrow_mut().take() {
            timer.invalidate();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The lenient integer path, as `current` uses it. Kept reachable from the
    /// tests deliberately: `resolve` is private, and if the only route to it
    /// were `current` — which forces a CFPreferences sync and reads
    /// `NSWorkspace` — the out-of-range fallback would be untestable.
    fn style(theme: i64) -> WidgetStyle {
        WidgetStyle::from_token(resolve(theme), None)
    }

    #[test]
    fn every_token_maps_to_its_documented_enum() {
        let expected = [
            ("RegularAutomatic", 0),
            ("RegularLight", 1),
            ("RegularDark", 2),
            ("ClearAutomatic", 3),
            ("ClearLight", 4),
            ("ClearDark", 5),
            ("TintedAutomatic", 6),
            ("TintedLight", 7),
            ("TintedDark", 8),
        ];
        for (name, raw) in expected {
            let token: IconAppearanceToken = name.parse().expect("a documented token");
            assert_eq!(token.raw(), raw, "{name}");
            assert_eq!(token.name(), name, "round-trip");
            assert_eq!(IconAppearanceToken::try_from(raw), Ok(token), "{raw}");
        }
    }

    /// `ALL` must be in preference-value order and complete, since `try_from`
    /// and `from_str` both search it.
    #[test]
    fn all_is_complete_and_in_preference_order() {
        assert_eq!(IconAppearanceToken::ALL.len(), 9);
        for (i, token) in IconAppearanceToken::ALL.iter().copied().enumerate() {
            assert_eq!(token.raw(), i as i64, "{token} is out of order");
        }
    }

    /// The plausible-looking string is NOT the Dark token; `RegularDark` is.
    /// A previous investigation tried ~15 candidate names and never found it.
    #[test]
    fn dark_is_not_a_token() {
        let err = "Dark".parse::<IconAppearanceToken>().unwrap_err();
        assert_eq!(err.found(), "Dark");
    }

    #[test]
    fn an_unrecognised_token_string_has_no_enum() {
        assert!("Nonsense".parse::<IconAppearanceToken>().is_err());
    }

    /// The two failure domains are separate types, so neither has to stringify
    /// the other's input to report it.
    #[test]
    fn the_two_error_domains_report_their_own_input() {
        let bad_name = "Nonsense".parse::<IconAppearanceToken>().unwrap_err();
        assert_eq!(
            bad_name.to_string(),
            r#"unknown icon appearance token "Nonsense""#
        );

        let bad_value = IconAppearanceToken::try_from(9999).unwrap_err();
        assert_eq!(bad_value.found(), 9999);
        assert_eq!(
            bad_value.to_string(),
            "icon appearance theme 9999 is out of range 0..=8"
        );
    }

    /// Out-of-range integers land on Regular ▸ **Auto**, which displays as
    /// "Dark ▸ Auto" — NOT on enum 1 / "Default". The two fallbacks differ, and
    /// Asserting only `style == Regular` would pass for either.
    #[test]
    fn out_of_range_integers_fall_back_to_regular_automatic() {
        let s = style(99);
        assert_eq!(s.style(), IconStyle::Regular);
        assert_eq!(s.mode(), ThemeMode::Auto);
        assert_eq!(s.to_string(), "Dark ▸ Auto");
        assert!(!s.is_tinted());
    }

    /// The lenient integer fallback (enum 0) and the lenient STRING fallback
    /// (enum 1) are different, and this is the only test that puts them side by
    /// side. `current` applies the string one; `resolve` applies the integer one.
    #[test]
    fn the_string_and_integer_fallbacks_differ() {
        assert_eq!(resolve(9999), IconAppearanceToken::RegularAutomatic);
        assert_eq!(
            "garbage"
                .parse::<IconAppearanceToken>()
                .unwrap_or(IconAppearanceToken::RegularLight),
            IconAppearanceToken::RegularLight
        );
        assert_ne!(
            IconAppearanceToken::RegularAutomatic,
            IconAppearanceToken::RegularLight
        );
    }

    #[test]
    fn only_the_tinted_family_carries_a_tint() {
        for theme in 0..=5 {
            assert!(!style(theme).is_tinted());
        }
        for theme in 6..=8 {
            assert!(style(theme).is_tinted());
        }
    }

    /// Enum 0 and enum 1 are both Regular but are different UI selections, and
    /// displaying both as "Default" hid that difference.
    #[test]
    fn regular_automatic_and_regular_light_display_differently() {
        assert_eq!(style(0).to_string(), "Dark ▸ Auto");
        assert_eq!(style(1).to_string(), "Default");
        assert_eq!(style(2).to_string(), "Dark");
    }

    #[test]
    fn display_carries_the_mode_for_clear_and_tinted() {
        assert_eq!(style(3).to_string(), "Clear ▸ Auto");
        assert_eq!(style(4).to_string(), "Clear ▸ Light");
        assert_eq!(style(5).to_string(), "Clear ▸ Dark");
        assert_eq!(style(8).to_string(), "Tinted ▸ Dark");
    }

    /// The whole suite passed with `NSAppearanceNameAqua` and
    /// `NSAppearanceNameDarkAqua` swapped in `appearance()` — "Clear ▸ Light
    /// renders dark and Clear ▸ Dark renders light", the most visible
    /// regression this crate can have, in the function the test below is named
    /// for. Asserting `is_some()` does not pin *which* appearance.
    #[test]
    fn forced_appearance_names_match_the_mode() {
        let aqua = unsafe { NSAppearanceNameAqua };
        let dark = unsafe { NSAppearanceNameDarkAqua };
        for theme in [1, 4, 7] {
            let a = style(theme)
                .appearance()
                .unwrap_or_else(|| panic!("theme {theme} must force an appearance"));
            assert_eq!(&*a.name(), aqua, "theme {theme} must force Aqua");
        }
        for theme in [2, 5, 8] {
            let a = style(theme)
                .appearance()
                .unwrap_or_else(|| panic!("theme {theme} must force an appearance"));
            assert_eq!(&*a.name(), dark, "theme {theme} must force DarkAqua");
        }
    }

    /// `Display` collapses `(Regular, _)` and `(RegularDark, _)`, so the mode of
    /// enums 1 and 2 is invisible to every other test — and enum 1 is the most
    /// common user selection, whose mode is the only thing deciding whether the
    /// window forces Aqua.
    #[test]
    fn each_theme_enum_maps_to_its_documented_family_and_mode() {
        use IconStyle::*;
        use ThemeMode::*;
        let expected = [
            (0, Regular, Auto),
            (1, Regular, Light),
            (2, RegularDark, Dark),
            (3, Clear, Auto),
            (4, Clear, Light),
            (5, Clear, Dark),
            (6, Tinted, Auto),
            (7, Tinted, Light),
            (8, Tinted, Dark),
            (99, Regular, Auto),
            (-1, Regular, Auto),
        ];
        for (theme, want_style, want_mode) in expected {
            let s = style(theme);
            assert_eq!(s.style(), want_style, "theme {theme} family");
            assert_eq!(s.mode(), want_mode, "theme {theme} mode");
        }
    }

    /// `PartialEq` gates the timer-driven reconcile: `reconcile(false)`
    /// early-returns on it, while the KVO path forces. A wrong `eq` means style
    /// changes silently stop repainting. Covers the `(Some, Some)` tint arm,
    /// which no other test reaches.
    #[test]
    fn widget_style_eq_separates_mode_and_tint() {
        assert_ne!(style(4), style(5), "Clear/Light must not equal Clear/Dark");

        let red = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.0, 0.0, 1.0);
        let red2 = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.0, 0.0, 1.0);
        let blue = NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 1.0, 1.0);

        let t1 = WidgetStyle::from_token(IconAppearanceToken::TintedDark, Some(red));
        let t2 = WidgetStyle::from_token(IconAppearanceToken::TintedDark, Some(red2));
        let t3 = WidgetStyle::from_token(IconAppearanceToken::TintedDark, Some(blue));
        let t4 = WidgetStyle::from_token(IconAppearanceToken::TintedDark, None);

        assert_eq!(t1, t2, "equal colours must compare equal");
        assert_ne!(t1, t3, "different colours must compare unequal");
        assert_ne!(t1, t4, "Some(tint) must not equal None");
    }

    /// `only_the_tinted_family_carries_a_tint` passes `None` for all nine
    /// themes, so it asserts the flag and never that the colour is threaded
    /// through to the tinted family.
    #[test]
    fn the_tint_reaches_exactly_the_tinted_themes() {
        for theme in [0, 1, 2, 3, 4, 5, 6, 7, 8, 99] {
            let colour = NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 1.0, 0.0, 1.0);
            let s = WidgetStyle::from_token(resolve(theme), Some(colour));
            assert_eq!(
                s.tint().is_some(),
                (6..=8).contains(&theme),
                "theme {theme}: tint must reach exactly the Tinted family"
            );
        }
    }

    #[test]
    fn auto_inherits_rather_than_forcing_an_appearance() {
        assert!(style(0).appearance().is_none());
        assert!(style(3).appearance().is_none());
        assert!(style(6).appearance().is_none());
        assert!(style(4).appearance().is_some());
        assert!(style(5).appearance().is_some());
    }

    /// `appearance()` returns `None` for Auto, so a consumer that only has that
    /// cannot tell light from dark. `is_dark` resolves it against the ambient
    /// appearance, and forced modes must IGNORE the ambient one.
    #[test]
    fn is_dark_resolves_auto_but_forced_modes_ignore_the_ambient() {
        let light = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameAqua }).unwrap();
        let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }).unwrap();

        // Auto follows the ambient appearance.
        assert!(!style(3).is_dark(&light));
        assert!(style(3).is_dark(&dark));

        // Forced modes do not.
        assert!(!style(4).is_dark(&dark), "Clear ▸ Light stays light");
        assert!(style(5).is_dark(&light), "Clear ▸ Dark stays dark");
    }

    /// The default is the interval the reference app uses, and the docs quote
    /// it in prose; a drift between the two would silently mislead.
    #[test]
    fn the_default_reconcile_is_the_measured_interval() {
        assert_eq!(
            Reconcile::default(),
            Reconcile::Every(Duration::from_millis(750))
        );
    }
}
