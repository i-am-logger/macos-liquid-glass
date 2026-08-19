# Measurements

Why every calibration constant in this repository has the value it has, and what
the reference surface — a live Notes desktop widget — actually looks like.

Figures were captured on macOS 27.0 (26A5388g) at capture px ÷ screen pt = 2.0,
with live window bounds read per run. Where a number is a preference rather than
a match, it says so.

## Units and preconditions

- **Luma** is Rec.709, `L = 0.2126R + 0.7152G + 0.0722B`, applied to sRGB
  component values, reported 0–255 unless stated.
- **MAE** is mean absolute error over the compared region, 0–1. Use MAE, not
  `magick compare -metric AE`: the `AE` metric is broken in this ImageMagick
  build and reports 1.2127e+08 differing pixels for a 360×360 (129,600 px)
  image. It fails by returning a plausible-looking number, not by erroring.
- **B/R** is the blue:red channel ratio, used as a hue handle because an
  achromatic darkening cannot change it.
- The system's Liquid Glass tint slider (System Settings ▸ Appearance) is the
  `NSGlassTintAmount` global default, and was **0** for every figure here.
  Nothing in the crate reads it, but a non-zero value changes what the material
  renders, so a re-measurement must confirm it is still 0.
- Glass samples whatever is behind it, so absolute luma is comparable only
  between captures taken at the same position over a **pixel-identical**
  backdrop. Equal mean luma is not enough: a desktop widget repainting behind the
  glass moved 2028 pixels by more than 20/255 while moving the mean by 0.06.
- The window must be **key** for a capture to be comparable — see "Focus changes
  the material".

## The Icon & widget style preference

### Nine tokens behind a four-option UI

`AppleIconAppearanceTheme` in the global domain encodes a style family and a
light/dark/automatic axis in one string. The four-option pane writes nine
distinct values. Recovered by driving the real System Settings controls and
reading the key back after each selection:

| token | `iconAppearanceTheme` | UI selection |
|---|---|---|
| `RegularAutomatic` | 0 | Dark ▸ Auto |
| *(key absent)* → `RegularLight` | 1 | Default |
| `RegularDark` | 2 | Dark |
| `ClearAutomatic` | 3 | Clear ▸ Auto |
| `ClearLight` | 4 | Clear ▸ Light |
| `ClearDark` | 5 | Clear ▸ Dark |
| `TintedAutomatic` | 6 | Tinted ▸ Auto |
| `TintedLight` | 7 | Tinted ▸ Light |
| `TintedDark` | 8 | Tinted ▸ Dark |

Two consequences for anyone branching on this:

- Choosing **Default removes the key** rather than writing a string, so an
  absent key is a meaningful state, not a missing one. Enum 1 is therefore
  inferred: no value can ever be read back for it.
- Enum 0 is the **Dark** style with its Auto sub-option, not Default. Pressing
  the Dark style button while Auto is still selected produces
  `RegularAutomatic`. Labelling 0 and 1 alike hides a real distinction.

`"Dark"` is not a token; an unrecognised string resolves to `RegularLight`.

### `defaults write` does not apply this setting

`defaults write -g AppleIconAppearanceTheme ClearLight` changes the stored value,
and it reads back correctly through
`NSWorkspace.currentIconAppearanceConfiguration` (5 → 4 → 1 → 5 observed) — but
the live Notes widget was **pixel-identical across the change, MAE 0**. The
system stores the preference and never applies it.

Driving the System Settings controls with
`AXUIElementPerformAction(kAXPressAction)` does apply: Clear → Tinted changed the
widget's band from a neutral grey gradient to solid blue and its text from white
to blue, and `resolvedIconTintColor` went from `nil` to `systemBlueColor`.

So any test loop that changes this setting has to drive the real UI and verify by
reading the key back. Three constraints on doing that safely:

- Anchor the control lookup to the "Icon & widget style" **label**, not to screen
  coordinates. Three controls in the pane share the description "Dark" (the
  system appearance button, the icon style button, and the sub-appearance
  radio), positions shift when the pane scrolls, and a y-range press intended
  for the icon-style row can land on the system appearance buttons and change
  the user's Light/Dark setting.
- Section labels carry their text in `AXValue`, not `AXTitle`/`AXDescription`.
- An AX press addresses the control directly and is immune to occlusion. A
  synthetic `CGEventPost` click at the same coordinates lands on whatever window
  is in front — during these runs Safari sat over most of the pane.

### What actually differs between the styles

Live Notes desktop widget, 180×180pt, captured once per style. Absolute luma is
tied to this wallpaper and position; the comparisons within the table are the
usable part.

| style | band luma | body luma | body RGB |
|---|---|---|---|
| Default ≡ Clear ▸ Light ≡ Clear ▸ Auto | 112.5 | 100.9 | (82, 103, 139) |
| Dark ≡ Clear ▸ Dark | 72.6 | 41.1 | (28, 42, 67) |
| Tinted ▸ Light ≡ Tinted ▸ Auto | 114.4 | 103.7 | (75, 108, 150) |
| Tinted ▸ Dark | 59.1 | 41.0 | (27, 42, 68) |

- **Default and Clear render identically** for a desktop widget (MAE 0), as do
  Dark and Clear ▸ Dark (MAE 4.5e−07). Not a dead probe: the control comparison
  Clear ▸ Dark against Clear ▸ Light gives MAE 0.151. The family therefore only
  has to distinguish untinted from tinted.
- **`*Automatic` follows the system appearance and keeps following it.**
  Verified with the system appearance set to Dark, where Clear ▸ Auto rendered
  dark on both the widget and this window.
- **Tinted washes the band, not the body.** Measure it with a theme colour that
  contrasts with the wallpaper — a blue theme over a blue wallpaper makes the
  wash invisible and reads as no tint at all. With a **green** theme over water:

  | region | RGB | G/((R+B)/2) |
  |---|---|---|
  | widget band | (28.9, 53.8, 54.2) | **1.30** — green |
  | widget body | (20.3, 39.4, 56.6) | **1.02** — still the water's blue |

  The widget tints its band clearly and leaves the content area essentially the
  backdrop's own colour. This window matches it band-weighted rather than
  uniformly: after the fix, band G-dominance 1.14 against the widget's 1.09, and
  body 1.02 against 1.12.

### Content is white in every style

Apple's accented-rendering rule "tints primary and accented content white in iOS
and macOS", and the reference obeys it: the widget's text is light under
Clear ▸ Light too, over a light surface. Resolving text through `labelColor`
gets this backwards, because `labelColor` is black in Aqua.

## The reference widget: band versus body

Widget body **330×328 px = 165×164 pt**, centred in its 180×180pt window with
~7.5pt of margin. The paper's top edge sits 88px (44pt) below the widget top.

A column through the widget centre against a column of bare wallpaper on the same
rows:

| region | inside the widget | bare wallpaper alongside |
|---|---|---|
| above the widget | 74 → 101 (steep rise) | 76 → 106 |
| **band** | 62 → 76, smooth and monotonic | 104 → 110, wobbling |
| **body** | **flat 33–35** | 110–118, varying |

The **band is bare glass** — its gradient is the material sampling the wallpaper,
continuing the wallpaper's own trend behind it — and the **body is a near-opaque
fill**: it varies ~3 luma across its width where nearby wallpaper varies ~14.

This is why the window paints **no gradient** in the title-bar area. One drawn
there would be a fitted constant reproducing what the material already does for
free. The band is not untouched, though: under Tinted it takes the theme wash at
full strength while the body takes a quarter of it.

The widget's own corner radius is not established: a circle fit to the corner
trace gives ~61px = 30.5pt, but the system draws a continuous (squircle) corner,
for which a circle fit overestimates. It is not needed — the window uses the
system's window radius, **16.0**, read from `effectiveCornerRadii` on a real
titled window. A borderless window uses `NSNextStepFrame`, which does not
implement that selector, so 16.0 is the system's value rather than a read of
this window's own.

### Why the darkening is split between the material and the paper

The content area darkens by different amounts per appearance: against a
~115-luma wallpaper the body reads 100.9 under Clear ▸ Light (~12% darker) and
41.1 under Clear ▸ Dark (~64% darker). Both appearances *darken*, so the fill is
black in both and only the strength changes — and one alpha cannot serve both.
`textBackgroundColor` and `windowBackgroundColor` are wrong here for the same
reason: they are white or light grey in Aqua, so they lighten, and they paint the
glass out.

The surface darkening is applied to the **material**, through
`NSGlassEffectView.tintColor`, and the body then takes a small extra dim on top.
That difference is what makes the title bar read as a distinct band. Neither
extreme works: darkening entirely by `tintColor` leaves no title bar at all,
because there is nothing for it to contrast against, and darkening entirely by an
opaque paper paints the glass out, so the surface stops looking like glass.

`tintColor` is documented as a bias toward a colour, and at full strength with a
saturated colour it does flood the surface to a flat slab. Black at a sub-unity
alpha is a different thing, and is the mechanism this build relies on.

### Why the darkening colour is not pure black

Measured at 0.50 / 0.70 / 0.85 alpha, a **black** tint left B/R pinned at
**1.45** at every strength. Pure black scales all three channels by the same
factor, so the harder it darkens the less colour survives, and the hue cannot
move at all. Darkening toward a slightly saturated cool colour instead keeps a
cast while still darkening, which is why the shipped darkening carries a hue and
a saturation rather than being black.

The darkening colour stays neutral-cool in **every** style, Tinted included.
Pushing it to the theme colour tints the whole surface — with a green theme it
turned the body green, where the reference body keeps the backdrop's own colour.
The theme colour reaches the surface as the band-weighted wash described above,
not as the darkening hue.

## The shipped constants

Values as they are in `examples/common/mod.rs`. Every one is overridable by the
named environment variable, which is how they were swept.

| knob | value | what it is | how it was set |
|---|---|---|---|
| `GN_GT_D` | 0.54 | material darkening, Dark | fitted against the live widget |
| `GN_GT_L` | 0.14 | material darkening, Light | fitted against a **backdrop-controlled** capture |
| `GN_HUE` | 0.60 | darkening hue | fitted with `GN_HSAT` |
| `GN_HSAT` | 0.80 | darkening saturation | fitted against a backdrop-controlled capture |
| `GN_PAPER_D` | 0.22 | extra body dim, Dark | fitted against the live widget |
| `GN_PAPER_L` | 0.12 | extra body dim, Light | fitted against the live widget |
| `GN_TINT` | 0.20 | theme wash on the band under Tinted | inherited from the reference build; **not measured** |
| `GN_TBODY` | 0.25 | share of the band's wash the body gets | from the green-theme band/body split above |


The body dim is measured as its share of the **band** — which is what the paper
composites over — not of the wallpaper. Comparing the body to the wallpaper
instead yields ~0.64 and over-darkens the surface.

Read from the API rather than typed:

| value | source | reading |
|---|---|---|
| window corner radius | `frameView.effectiveCornerRadii` (public, readonly, 27.0) | 16 |
| Liquid Glass slider | `AXSlider "Liquid Glass Tint Amount"` ⇒ `NSGlassTintAmount` | 0 |

Chosen by eye and **not** measured: the 560×360pt window, 14pt padding, 12.5pt
font, 17pt line height, the 32pt title-bar height, and the text alphas (0.35
blend, 0.62 dim, 0.85 title). The 32pt title bar is not read from
`contentView.height − contentLayoutRect.height`: that formula returns 0 on a
plain titled window and 28 with `.fullSizeContentView`, and no configuration
yields 32. It is chosen for this window's proportions. The 0.35/0.85 pair that
circulates for unfocused-window compensation is Ghostty's hand-fit for a
different problem, quoted here as someone else's number — it is not the source
of these text alphas.

Drawn fills are built in explicit sRGB, which pins the colour space the constants
above belong to. The theme wash is used in the space the system hands it over in;
converting it to sRGB first measures worse, spreading a +0.11 luma error across
band, body and text (MAE 0.0011 against 0.00045).

`NSBezierPath::fillRect`, not `NSRectFill`. They are not interchangeable:
`NSRectFill` composites with Copy, which wipes the material instead of dimming
it — 18× worse whole-window MAE.

## Focus changes the material

`NSGlassEffectView` lightens when its window resigns key. No code in this
repository does it, and there is no public knob to disable it:

| region | key | non-key | delta |
|---|---|---|---|
| title bar | 95.95 | 128.78 | **+32.83** |
| body | 26.04 | 34.39 | **+8.35** |

Two things follow. A capture taken while the window is not key is not comparable
with one taken while it is — that is a bigger difference than anything else in
this document, so a measurement harness has to refuse rather than record it. And
the reference widget never shows this, because a desktop widget is never key;
matching it in the unfocused state would need a hand-fitted compensating overlay,
which this build does not ship.

## What this window cannot match

Glass to glass over an **identical** backdrop, with the window parked on the same
water as the widget, the widget's bare band carries chroma 45.9 against this
window's 32.6 — a material difference of about 40%.

The hue is the part that cannot be closed with public API. Achromatic darkening —
whether a paper over the glass or a black `tintColor` — multiplies all three
channels by the same factor, so B/R cannot move off the public material's own
1.45. Every public route was tried:

| route | result |
|---|---|
| semantic background colours (`windowBackgroundColor`, `underPageBackgroundColor`, `controlBackgroundColor`, `textBackgroundColor`) | chroma **0.0** under both appearances, even with "Tint window background with wallpaper color" on — desktop tinting happens at composite time, not in the colour |
| whole-wallpaper average | chroma 4.4, and *worse* with more saturation — a photo averages to near-grey |
| wallpaper sampled under the window | warm cast; the desktop picture reads as `missing value`, because a dynamic wallpaper is not a plain file |
| black paper over the glass | B/R 1.53 |
| black `NSGlassEffectView.tintColor` | B/R 1.45 at 0.50, 0.70 **and** 0.85 |

With the shipped constants the hue ratio is essentially matched over a controlled
backdrop — body B/R 1.71 against the widget's 1.77 — and the residual is chroma,
not hue. The widget uses a widget-specific material that public AppKit does not
expose. This window therefore reads slightly greyer than a real widget. It is a
stated limitation, not a defect.

Note that the widget's B/R is 2.58 over rocks and 1.77 over water. Any figure for
this gap taken from captures at two different positions is measuring the
wallpaper.

## Following a change

### No notification centre posts one

Every route below was tested with each setting change verified by reading the
preference back afterwards, so a silent centre is distinguishable from a setting
that never changed:

| route | result |
|---|---|
| `NSWorkspaceIconAppearanceConfigurationDidChangeNotification` on `NSWorkspace.shared.notificationCenter` | never fires |
| the same, on `NotificationCenter.default` | never fires |
| the same, on `DistributedNotificationCenter.default()` | never fires |
| `AppleInterfaceThemeChangedNotification` (distributed) | never fires |
| six further candidate Darwin notification names | never fire |
| SkyLight's `kSLSCoordinatedIconAppearanceConfigurationChangeNotificationName`, resolved by `dlsym` and registered with `notify_register_dispatch` returning `NOTIFY_STATUS_OK`, as Darwin **and** distributed | never fires |

That SkyLight constant is self-named: its value is the literal string
`kSLSCoordinatedIconAppearanceConfigurationChangeNotificationName`.

KVO on the global-domain keys is the only event source that fires:
`AppleIconAppearanceTheme` 4/4 style changes, `AppleAccentColor` 3/3 theme-colour
changes, `AppleInterfaceStyle` 1/1 appearance changes. Apple documents this as
the mechanism for cross-process changes — `UserDefaults.didChangeNotification`
explicitly does not fire for another process's writes — with no latency
guarantee, and these are `NSGlobalDomain` keys written by System Settings, which
is outside "your app's settings" as the documentation words it.

The theme colour is written to **`AppleAccentColor`**, found by diffing
`defaults read -g` across a colour change, not to the plausible-sounding
`AppleIconAppearanceTintColor`.

### Never read a preference inside a KVO callback

KVO callbacks are delivered **while CFPrefs holds the CFPrefsSource lock**.
Reading a preference from inside one re-enters that lock and the process is
killed by `_os_unfair_lock_recursive_abort`:

```
_os_unfair_lock_recursive_abort
-[NSWorkspace(SLSIconAppearance) currentIconAppearanceConfiguration]
SystemStyle.current()
observeValue(forKeyPath:of:change:context:)
NSKeyValueNotifyObserver
-[CFPrefsSource _notifyObserversOfChangeFromValuesForKeys:toValuesForKeys:]
```

The callback must only enqueue. `performSelectorOnMainThread:…:waitUntilDone:NO`
is documented to defer even when already on the main thread, which is where KVO
arrives; anything that runs the closure immediately when already on the main
thread — `dispatch2::run_on_main` states exactly that behaviour — reproduces the
crash. That is why the crate does not depend on it.

### Why the reconcile pass exists

KVO is correct but slow, and the delay is CFPrefs, not the observer. Racing a
forced-sync reader against the app from the same AX press:

| observer | when it saw the change |
|---|---|
| forced `CFPreferencesAppSynchronize` every 50 ms | **0.54 s** |
| the app, via KVO | **3.86 s** |

The write lands promptly; CFPrefs sits on the cross-process notification for
~3.3 s, coalescing foreign global-domain changes.

The shipped design is a hybrid: KVO plus a forced-sync pass on a fixed 50 ms
interval. Neither half is exposed; the interval is a private constant.

Measured at a 750 ms interval, timing from a `defaults write` to the observer's
callback over five changes: 138 ms fastest, 335 median, 671 slowest — a uniform
spread across the interval, which is what a poll of that period predicts. The
sync, not KVO, is what delivers. At the shipped 50 ms the same distribution
gives 0–50 ms, median ~25 ms.

**KVO alone delivered nothing** in the same test: with the pass disabled, two
changes 40 s apart produced no callback beyond the priming one at construction.
Without the forced sync the process never re-reads the global domain, so a
foreign write goes unnoticed. That is why the interval is not a knob — there is
no useful setting other than "on".

The caveat that applies to any `defaults write` result here: it is not the path
a user takes, and a change driven through the Settings UI may reach KVO by a
route this test does not exercise.

AppKit exports `NSWorkspaceIconAppearanceConfigurationDidChangeNotification`,
which appears in no public header. It resolves at runtime. Whether it is posted
for a real Settings change is not established; it did not fire for a
`defaults write`, which is consistent with that write never being applied.

The forced sync measures 0.0% CPU in both this process and cfprefsd even at
3 syncs/second.

`NSWorkspace.currentIconAppearanceConfiguration` is correct eventually but its
cached value lags: it still reported Clear ▸ Dark a full 2 s after the system had
moved to `ClearLight`. Read the `AppleIconAppearanceTheme` string directly and
keep the configuration only for `resolvedIconTintColor`, which has no
preference-key equivalent.

### Apply the style before the window is shown

The style has to be applied before `window.show()`, not from an observer
constructed after it. Otherwise the window is on screen for one or more frames
with no material tint, and a capture that races that gap reads the band 3.9 luma
brighter — an order of magnitude larger than any real difference measured here.
With the observer constructed first, five consecutive launches measure
byte-identical.

## Measuring this surface again

- Pin the window to a fixed screen rect and check it arrived. Window position is
  not stable across relaunches, and two captures over different wallpaper are not
  comparable.
- Refuse the run, rather than recording it, if the window is absent, occluded,
  not key, moved mid-run, or if the bare backdrop drifted pixel-wise between
  captures. Each of those produced a plausible wrong number before it was
  guarded: a full-screen terminal over the window read the body as exactly 0.0,
  and an animated wallpaper took the bare desktop from 33.7 → 29.9 → 7.5 luma
  over 20 seconds.
- A refusal is only useful if it names the real reason. An occlusion guard that
  reports "no windows matched" through the occlusion path reads as "something is
  covering it" and costs a false negative.
- Keep the test rect clear of live widgets. At 200,740 it is clear of every
  widget and of the terminal window; at 120,120 it overlapped the `City Digital`
  clock, whose minute tick discarded runs at random.
- Verify the probe itself. A capture pair that accidentally sampled two unfocused
  frames reported a 0.00 delta, and a run whose environment prefix failed to
  word-split under zsh returned identical values for six different
  configurations from one stale window.