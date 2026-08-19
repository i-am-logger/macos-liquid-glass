# Measurements — macOS 27.0 (26A5388g), 2026-08-05

Every number here came from a capture on this machine, at a measured scale
(capture px ÷ screen pt = 2.0), with live window bounds read per run. Values I
could not establish are marked UNKNOWN rather than guessed.

## 0. Metric validation (done before any conclusion)

Rec.709 luma, `L = 0.2126R + 0.7152G + 0.0722B`, read from `magick … txt:-`.

An earlier probe used `magick -crop … -colorspace gray -format '%[fx:mean*255]'`
and reported **144.4** for a pixel whose true value is `srgba(21,35,59)` →
**33.8**. That method round-trips through a linear grey colourspace and inflates
the number. It was discarded, along with the four readings taken with it.

`magick compare -metric AE` is also unusable in this ImageMagick build: it
reported `1.2127e+08` differing pixels for a 360×360 (129,600 px) image. MAE is
correct and is what is used below.

## 1. The "Icon & widget style" setting

### 1.1 The full string ↔ enum mapping (recovered)

Driving the real System Settings UI and reading `AppleIconAppearanceTheme` back:

| String | `iconAppearanceTheme` |
|---|---|
| *(key absent)* = Default | 1 |
| **`RegularDark`** | **2** |
| `ClearAutomatic` | 3 |
| `ClearLight` | 4 |
| `ClearDark` | 5 |
| `TintedAutomatic` | 6 |
| `TintedLight` | 7 |
| `TintedDark` | 8 |

`RegularDark` is the string for enum 2. A previous investigation tried ~15
candidate names and never found it; the string `"Dark"` maps to **1**, not 2.
Choosing "Default" *removes* the key rather than writing `"Default"`.

### 1.2 `defaults write` does not apply — the UI path does

`defaults write -g AppleIconAppearanceTheme ClearLight` changes the value and it
reads back correctly through `NSWorkspace.currentIconAppearanceConfiguration`
(5 → 4 → 1 → 5 observed) — but the live Notes widget was **pixel-identical**
across the change (MAE 0). The system stores the preference and never applies it.

Driving the System Settings controls with `AXUIElementPerformAction(kAXPressAction)`
**does** apply: Clear → Tinted changed the widget's band from a neutral grey
gradient to solid blue and its text from white to blue, and
`resolvedIconTintColor` went from `nil` to `systemBlueColor`.

A synthetic `CGEventPost` click was *not* used: Safari (z=3) sat in front of
System Settings (z=6) over most of the pane, so a click at the control's
coordinates would have landed on Safari. An AX press addresses the control
directly and is immune to occlusion. Every change is still verified by readback.

Three controls share the description "Dark" (system appearance button y≈140,
icon style button y≈577, sub-appearance radio y≈628), so the tool disambiguates
by y-range; a first-match search silently presses the wrong one.

### 1.3 What actually differs between the styles

Live Notes desktop widget, 180×180pt window, captured per style:

| style | band luma | body luma | step | body RGB |
|---|---|---|---|---|
| Default | 112.5 | 100.9 | −29.5 | (82,103,139) |
| ClearLight | 112.5 | 100.9 | −29.5 | (82,103,139) |
| ClearAutomatic | 112.5 | 100.9 | −29.5 | (82,103,139) |
| RegularDark | 72.6 | 41.1 | −41.7 | (28,42,67) |
| ClearDark | 72.6 | 41.1 | −41.7 | (28,42,67) |
| TintedLight / TintedAutomatic | 114.4 | 103.7 | −28.9 | (75,108,150) |
| TintedDark | 59.1 | 41.0 | −18.7 | (27,42,68) |

- **Default ≡ Clear** and **RegularDark ≡ ClearDark**, to MAE 0 / 4.5e−07.
  For a *desktop* widget these are the same rendering. This is not a dead probe:
  the control comparison ClearDark vs ClearLight gives MAE 0.151.
- `*Automatic` resolves to the **system** appearance (Light here), so it equals
  the `*Light` variant on this machine. UNKNOWN whether Automatic tracks the
  system appearance live — not yet tested with the system set to Dark.
- **Tinted** changes the *content* colour, not the surface: in dark mode the
  body is unchanged (27,42,68 vs 28,42,67) while the text goes from
  (170,175,184) to blue (87,107,140).

## 2. The widget's structure — band vs body

Widget body: **330×328 px = 165×164 pt**, centred in its 180×180pt window with
~7.5pt of margin. The paper's top edge sits 88px (44pt) below the widget top.

The decisive measurement, a column through the widget centre vs a column of bare
wallpaper, same rows:

| region | inside the widget | bare wallpaper alongside |
|---|---|---|
| above the widget | 74 → 101 (steep rise) | 76 → 106 |
| **band** | 62 → 76, smooth and monotonic | 104 → 110, wobbling |
| **body** | **flat 33–35** | 110–118, varying |

So the **band is bare glass** — its gradient is the material sampling the
wallpaper, continuing the wallpaper's own trend behind it — and the **body is a
near-opaque fill**. The body varies ~3 luma across its width while nearby
wallpaper varies ~14.

This is why the window paints **nothing** in the title-bar area. Drawing a
gradient there would be a fitted constant reproducing what the material already
does for free.

Corner radius: a circle fit to the corner trace gives ~61px = 30.5pt, but the
system draws a *continuous* (squircle) corner, for which a circle fit
overestimates. The true value is UNKNOWN; the window uses the window-level
radius (16) read from `effectiveCornerRadii`, not the widget's.

## 3. Values taken from the API rather than typed

| value | source | reading |
|---|---|---|
| titlebar height | `contentView.height − contentLayoutRect.height` | 32 (not the pre-Solarium 28) |
| window corner radius | `frameView.effectiveCornerRadii` (public, readonly, 27.0) | 16 |
| Liquid Glass slider | `AXSlider "Liquid Glass Tint Amount"` ⇒ `NSGlassTintAmount` | 0 |

The Liquid Glass slider in System Settings **is** the `NSGlassTintAmount`
default — previously logged as an unexplained key.

## 4. Still open

- The paper alpha (`GN_PAPER`) is a **calibrated** number, not an API-owned one.
  AppKit has no "fill over a material" constant. It is exposed as an env var and
  swept, so it is measured rather than eyeballed — but it is still fitted, and
  is stated as such.
- Whether `*Automatic` follows the system appearance live: **not yet tested**.
- Whether `NSGlassEffectViewStyle.clear` or `.regular` better matches the
  reference surface: **being measured**, not yet settled.
- The current build's paper is **too transparent** — the wallpaper reads clearly
  through the terminal body, where the reference body is flat. Known defect.

---

# Session 2 — corrections and additions

## R1. RETRACTED: "Tinted leaves the surface alone"

Session 1 concluded that Tinted changed only the content, from
`TintedDark` body (27,42,68) vs `ClearDark` (28,42,67). **That measurement was
confounded**: the theme colour was Blue and the wallpaper is blue, so a blue
tint is nearly invisible. Re-measured with the theme colour set to **Purple**:

| region | ClearDark | TintedDark (purple) |
|---|---|---|
| band | (54,65,86) sat 0.231 | **(51,38,86) sat 0.384** |
| body | (23,38,63) sat 0.469 | (27,33,63) sat 0.393 |

Green falls, red rises — the hue moves. Across all eight states with a purple
tint:

| state | band | body |
|---|---|---|
| Default ≡ Clear▸Light ≡ Clear▸Auto | (106,124,155) | (82,103,140) |
| Dark ≡ Clear▸Dark | (55,67,89) | (25,39,63) |
| Tinted▸Light ≡ Tinted▸Auto | **(126,113,168)** | **(105,91,154)** |
| Tinted▸Dark | **(51,38,86)** | (27,33,63) |

**Tinted washes the whole surface, title bar included.** The Default ≡ Clear and
RegularDark ≡ ClearDark equalities from session 1 are re-confirmed here.

Note the tint shifts *hue* without raising saturation much (Clear▸Light body
sat 0.263 vs Tinted▸Light 0.259), so the wash must be modest.

## R2. Content is WHITE in every style

Apple's accented-rendering spec — "tints primary and accented content white in
iOS and macOS"
(developer.apple.com/documentation/widgetkit/optimizing-your-widget-for-accented-rendering-mode-and-liquid-glass)
— and the reference obeys it: the widget's text is light even under
Clear ▸ Light, over a light surface. Resolving text through `labelColor` is
backwards, because `labelColor` is black in Aqua.

## R3. The content area darkens by different amounts per appearance

Against a ~115-luma wallpaper: body 100.9 under Clear ▸ Light (**~12%** darker)
and 41.1 under Clear ▸ Dark (**~64%** darker). A single alpha cannot be both — at
one legible value the light terminal renders as a flat washed slab. Both
appearances *darken*, so the fill is black in both and only the strength changes.

`textBackgroundColor` and `windowBackgroundColor` are both wrong here: they are
white/light grey in Aqua, so they *lighten*, and they paint the glass out.

## R4. `NSGlassEffectView.tintColor` floods

Documented as "the color the glass effect view uses to tint the background and
glass effect toward" — a bias. In practice, setting it to the resolved tint
renders the surface a flat saturated slab. The tint is applied as a drawn wash
in the content layer instead. `tintColor` stays nil.

## R5. The workspace configuration lags; read the preference string

`NSWorkspace.currentIconAppearanceConfiguration` is correct *eventually* but its
cached value lagged a real settings change by several seconds — the window still
showed `Clear ▸ Dark` a full 2s after the system had moved to `ClearLight`.
Forcing `CFPreferencesAppSynchronize` and reading the `AppleIconAppearanceTheme`
string directly cut the observed latency from **~5s to ~0.03s**. The
configuration is still used for `resolvedIconTintColor`, which has no
preference-key equivalent.

## R6. Key vs non-key is large, and is the material's own behaviour

Measured with the probe verified (the first attempt captured two *non-key*
frames and reported a 0.00 delta):

| region | key | non-key | delta |
|---|---|---|---|
| title bar | 95.95 | 128.78 | **+32.83** |
| body | 26.04 | 34.39 | **+8.35** |

No code does this; `NSGlassEffectView` adapts on resign-key. There is no public
knob to disable it. A real widget never changes, because it is never key.
**Open decision for the user:** accept native window behaviour, or suppress it
(which needs a hand-fitted compensating overlay — the approach Ghostty takes,
with 0.35/0.85 constants its author calls "may not be ideal").

## R7. Private `_variant` was tested and NOT adopted

`set_variant:` with variant 4 (`widgets` in a reverse-engineered table) does
change the material — body transmission 0.05 vs 0.38 for public `.clear`. But it
renders flat and dead next to `.regular`/`.clear`, and it is unsupported SPI with
no availability contract. It is gated behind `GN_VARIANT`, guarded by
`responds(to:)`, off by default, and reported rather than shipped.

## R8. Driving System Settings safely

Absolute screen-y ranges break silently when the pane scrolls — one press
intended for the icon-style row landed on the **system appearance** buttons and
switched the user's Light/Dark setting (restored immediately). Control lookup is
now anchored to the "Icon & widget style" label's position. Section labels carry
their text in `AXValue`, not `AXTitle`/`AXDescription`.

## R9. Tooling traps hit this session

- `magick compare -metric AE` reports impossible values in this build
  (1.2127e+08 differing pixels on a 129,600-pixel image). MAE is correct.
- **zsh does not word-split** unquoted parameters: `set -- $cfg` yields one
  field, so a six-run parameter sweep silently ran the *same* config six times.
- Window position is not stable across relaunches, so two captures can sit over
  different wallpaper — absolute luma is not comparable between them.

## Still open

- The window is more transparent overall than the widget. The widget uses a
  widget-specific material that public API does not expose.
- Corner radius of the widget: circle-fit ~30.5pt, but the corner is a
  continuous squircle for which a circle fit overestimates. UNKNOWN.
- Whether `*Automatic` tracks the system appearance live — the system was Light
  throughout, so Automatic was never observed resolving to Dark.

## R10. The body's COLOUR cannot be reproduced with public API

The user observed the window's background looked black/white while the widget's
takes the wallpaper's colour. Measured, Clear ▸ Dark:

| surface | body RGB | chroma | hue B/R |
|---|---|---|---|
| **widget (reference)** | (24.6, 38.8, 63.4) | **38.8** | 2.58 |
| black dim @0.64 | (21.1, 25.0, 32.3) | 11.2 | 1.53 |
| wallpaper-average tint | (57.1, 56.7, 61.2) | **4.4** | ~1.07 |
| **private `_variant`=4 + no paper** | (14.4, 22.6, 37.5) | **23.1** | **2.60** |

Three findings:

1. **The observation is real.** A black dimming layer scales every channel by the
   same factor, so it darkens without preserving colour — chroma 11.2 against the
   widget's 38.8.
2. **No AppKit colour can supply the hue.** Resolved under both appearances,
   `windowBackgroundColor`, `underPageBackgroundColor`, `controlBackgroundColor`
   and `textBackgroundColor` are all **achromatic — chroma 0.0** — even with
   "Tint window background with wallpaper color" enabled. Desktop tinting happens
   at composite time, not in the colour.
3. **Averaging the wallpaper is worse, not better** (chroma 4.4): a whole photo
   averages to near-grey, and raising saturation of a near-grey hue does nothing
   — 1.6 -> 2.4 made it *worse* (4.4 -> 3.1). Abandoned rather than tuned further.

The widget's body is also MORE saturated than its own glass band (38.8 vs 30.6),
i.e. it darkens *and* boosts saturation. No plain fill does that.

Private `_variant = 4` — named `widgets` in a reverse-engineered table — does.
It reproduces the hue almost exactly (B/R 2.60 vs the widget's 2.58). An earlier
test called it "flat and dead"; **that test was confounded** by a 0.82 black
paper drawn on top of it, which destroyed the colour it was producing. It
remains OFF by default and behind `GN_VARIANT`, because it is unsupported SPI.

Two measurement traps hit while establishing this, both producing plausible
numbers from nothing:
- Two calibration runs were measured against a dark-mode target while the system
  was actually in **ClearLight**. Void.
- An env-prefix written as `$2` did not word-split under zsh, so `run.sh` never
  launched and three configs returned **identical** values from a stale window.

## R11. The public search is now exhausted

Asked directly whether private API is the only route to the widget's background
colour. Every public avenue has now been tested and eliminated:

| route | result | verdict |
|---|---|---|
| semantic background colours | chroma **0.0**, both appearances | achromatic, cannot carry hue |
| whole-wallpaper average | chroma **4.4**, worse with more saturation | photo averages to grey |
| wallpaper sampled under the window | warm cast; desktop picture reads as `missing value` | dynamic wallpaper is not a plain file |
| black paper over glass | chroma 10-21, B/R **1.53** | proportional scaling |
| black `NSGlassEffectView.tintColor` | B/R **1.45 at 0.50, 0.70 AND 0.85** | invariant — darkens, cannot shift hue |
| private `set_variant:(4)` | chroma 23.1, B/R **2.60** | matches (widget 2.58) — declined |

The decisive observation is the **invariant B/R**. Any achromatic darkening —
paper or tintColor — multiplies all three channels by the same factor, so the
blue:red ratio cannot move. 1.45 is the public glass material's own hue. The
widget's material sits at 2.6-2.8, i.e. it is far more blue-selective, and it
also ends up MORE saturated than its own glass band (38.8 vs 30.6). Nothing in
the public surface does that.

Conclusion: on macOS 27.0 (26A5388g), the widget's background colour is **not
reachable through public AppKit**. The user chose public-only, so the shipped
window reads greyer than a real widget. This is a stated limitation, not a bug.

## R12. Shipped surface model — darken the MATERIAL, not a layer over it

The surface darkening is applied through `NSGlassEffectView.tintColor` (black at
a per-appearance alpha) rather than by painting an opaque rectangle. The body
then takes a small extra dim on top, and that difference is what makes the title
bar read as a distinct band.

This matters: with the darkening done entirely by tintColor and **no** body
paper, the window has no title bar at all — there is nothing for it to contrast
against. With the darkening done entirely by an opaque paper, the glass is
painted out and the surface stops looking like glass. Both were built and looked
at; the split is the fix.

Tuned with the user, on their wallpaper:

| knob | value | meaning |
|---|---|---|
| `GN_GT_D` | 0.38 | surface darkening, Dark |
| `GN_GT_L` | 0.36 | surface darkening, Light |
| `GN_PAPER_D` | 0.22 | extra body dim, Dark |
| `GN_PAPER_L` | 0.12 | extra body dim, Light |

Note `tintColor` was earlier dismissed as "floods the surface". That was true of
a full-strength saturated colour; a **black, sub-unity alpha** tint is a
different thing and is the mechanism the shipped build relies on. The earlier
blanket dismissal was too broad.

## R13. Final tuning, and why darkening costs colour

Darkening with pure black scales all three channels by the same factor, so the
harder you darken the less colour survives — measured, B/R pinned at 1.45 at
every strength. Darkening toward a slightly saturated colour instead keeps a
cast while still darkening. Under Tinted the darkening hue follows the THEME
colour, because a fixed blue fought a green theme and muddied it.

Tuned with the user on their wallpaper, Clear:

| knob | value |
|---|---|
| `GN_GT_D` surface darkening, Dark | **0.54** |
| `GN_GT_L` surface darkening, Light | **0.26** |
| `GN_PAPER_D` extra body dim, Dark | 0.22 |
| `GN_PAPER_L` extra body dim, Light | 0.12 |
| `GN_HUE` / `GN_HSAT` darkening colour | 0.60 / 0.45 (theme colour when Tinted) |

Result against the reference, Clear > Dark:

| | luma | chroma |
|---|---|---|
| widget | 37.6 | 38.8 |
| glassterm | **32.6** | **15.5** |

Luma now matches closely. Chroma does not, and cannot — see R11.

**A caveat that partly undercuts R11's framing.** The comparison is not
backdrop-controlled: the widget sits over blue water and the window sits over
grey rocks. Some of the remaining chroma gap is *position*, not material. The
fair test — parking the window exactly where the widget is and comparing over
identical backdrop — was NOT run. R11's conclusion (hue is unreachable by
achromatic darkening) still holds on its own evidence, because B/R is invariant
by construction, but the SIZE of the material gap is overstated by these
numbers.

## R14. The backdrop-controlled comparison — R11's magnitude was WRONG

R13 flagged that the widget and the window sat over different wallpaper. The
user then moved the window onto the same water, and the fair test was run.
Widget at (8,581), window at (13,774), Clear resolving Light:

| surface | luma | chroma | B/R |
|---|---|---|---|
| widget band (bare glass) | 112.9 | 45.9 | 1.47 |
| **window title bar (bare glass)** | **109.7** | **32.6** | **1.33** |
| widget body | 99.7 | 61.2 | **1.77** |
| window body (before retune) | 81.7 | 37.4 | **1.55** |

**The widget's B/R is 2.58 over rocks but 1.77 over water.** Most of the gap
reported in R11/R12 was the *backdrop*, not the material. The claim that the
widget's colour is dramatically beyond public API was overstated, and is
corrected here.

What survives: glass-to-glass on an IDENTICAL backdrop, the widget's bare band
carries chroma 45.9 against the window's 32.6. That is a real material
difference of about 40%, not the 2-3x implied earlier. The invariant-B/R
argument in R11 is still valid as far as it goes — achromatic darkening cannot
shift hue — but it was used to support a magnitude the evidence did not carry.

Retuned against the controlled target: `GN_GT_L` 0.26 -> **0.14** and `GN_HSAT`
0.45 -> **0.80**, giving body luma 80.6 and B/R **1.71** against the widget's
1.77 — the hue ratio is now essentially matched.

Method note: the first attempt at this capture was correctly DISCARDED by the
occlusion guard (`loginwindow` in front, z=1-3). Without the guard it would have
photographed the lock screen and reported it as the window.

## R15. Tinted washes the BAND, not the body — corrects R1

R1 concluded "Tinted washes the whole surface, title bar included". Measured
again with a GREEN theme over water, that is wrong about the body:

| surface | RGB | G-dominance G/((R+B)/2) |
|---|---|---|
| widget band | (28.9, 53.8, 54.2) | **1.30** — green |
| widget body | (20.3, 39.4, **56.6**) | **1.02** — still BLUE, i.e. the water |
| ours, before | (26.7, **50.7**, 35.8) | **1.62** — green everywhere |
| ours, after | (20.7, 30.5, 37.0) | **1.06** |

The widget tints its band clearly and leaves the content area essentially the
backdrop's own colour. Two things were wrong here:

1. The theme wash was applied uniformly over the whole surface. It is now
   band-weighted (`GN_TBODY`, default 0.25 of the band's strength).
2. The material's darkening hue had been pushed to the theme colour. That
   tinted everything green and is reverted — the darkening colour stays
   neutral-cool in every style.

Why R1 got it wrong: it was measured with a PURPLE theme over a BLUE wallpaper,
where purple and the backdrop's blue are close enough in hue that a body holding
the backdrop's colour looks like a body taking the tint. Green against blue
separates them.

Tinted > Light after the fix: band G-dom 1.14 vs the widget's 1.09; body 1.02 vs
1.12.

**Caveat, again not backdrop-controlled:** the window sat at x=199 and the widget
at x=8, so the luma comparison in Tinted > Light (body 46.8 vs 108.0) mixes in a
darker patch of wallpaper under the window. The G-dominance ratios above are the
reliable part; the luma figures are not.

## R16. Final all-modes test — and two unknowns closed

All eight selections driven through the real pane, each verified by reading the
preference back, capturing the live widget and glassterm together each time.
7/8 matched the expected token first time; the eighth is explained below.

| selection | preference | app |
|---|---|---|
| Clear ▸ Light | ClearLight | Clear ▸ Light |
| Clear ▸ Dark | ClearDark | Clear ▸ Dark |
| Clear ▸ Auto | ClearAutomatic | Clear ▸ Auto |
| Tinted ▸ Light | TintedLight | Tinted ▸ Light, tinted |
| Tinted ▸ Dark | TintedDark | Tinted ▸ Dark, tinted |
| Tinted ▸ Auto | TintedAutomatic | Tinted ▸ Auto, tinted |
| Dark | **RegularAutomatic** | Dark ▸ Auto |
| Default | *(key removed)* | Default |

**Closed unknown 1 — `*Automatic` does track the system appearance.** Every
earlier session ran with the system in Light, so Automatic was never observed
resolving to Dark and this was recorded as untested. The system appearance was
Dark for part of this run, and `Clear ▸ Auto` rendered dark on both the widget
and the window.

**Closed unknown 2 — enum 0 is the Dark style's Auto sub-option.** Pressing the
Dark style button while Auto was still selected produced `RegularAutomatic`, not
`RegularDark`. That confirms the previously-hypothesised mapping: the Dark style
carries Always/Auto, giving `RegularDark` (2) and `RegularAutomatic` (0), and
`Default` is `RegularLight` (1) / key-absent. The single "MISMATCH" in the run
was my expectation being wrong, not the app: the sub-option was still Auto from
the previous case.

Consequence fixed: enum 0 and enum 1 were both labelled "Default", hiding a real
distinction. Enum 0 now reads "Dark ▸ Auto".

## R17. Event-driven instead of polling — and a crash it exposed

Asked whether polling was really necessary. It is not, for the main inputs.

### Which mechanism actually fires

Every setting change verified by preference readback:

| mechanism | result |
|---|---|
| `NSWorkspaceIconAppearanceConfigurationDidChangeNotification` on `NSWorkspace.shared.notificationCenter` | never fired |
| …on `NotificationCenter.default` | never fired |
| …on `DistributedNotificationCenter.default()` | never fired |
| `AppleInterfaceThemeChangedNotification` (distributed) | never fired |
| **KVO on `UserDefaults.standard` / `AppleIconAppearanceTheme`** | **4/4 style changes** |
| **KVO / `AppleAccentColor`** | **3/3 theme-colour changes** |
| **KVO / `AppleInterfaceStyle`** | **1/1 appearance changes** |

This settles the long-open "which centre posts it" question: **none of the
three**. KVO on the global-domain keys is the working route.

The theme colour is written to **`AppleAccentColor`**, found by diffing
`defaults read -g` across a colour change — not to `AppleIconAppearanceTintColor`.

**A false negative caught:** the first run of this experiment showed *nothing*
firing, including the positive control. System Settings was not running, so no
setting ever changed — indistinguishable from a notification that never fires.
Re-run with every change readback-verified.

### The crash that fix caused, and why

KVO callbacks are delivered **while CFPrefs holds the CFPrefsSource lock**.
Reading a preference inside the callback re-enters that lock and the process is
killed:

```
_os_unfair_lock_recursive_abort
-[NSWorkspace(SLSIconAppearance) currentIconAppearanceConfiguration]
SystemStyle.current()
Controller.observeValue(forKeyPath:of:change:context:)
NSKeyValueNotifyObserver
-[CFPrefsSource _notifyObserversOfChangeFromValuesForKeys:toValuesForKeys:]
```

The first version took a synchronous fast path when already on the main thread —
which is exactly where KVO arrives — and died on the first style change.
`DispatchQueue.main.async` unconditionally lets CFPrefs drop the lock first.

**Never read defaults synchronously inside a UserDefaults KVO callback.**

### The trade-off, measured

| approach | latency | cost |
|---|---|---|
| 0.35s poll + forced `CFPreferencesAppSynchronize` | **0.03s** | 3 prefs syncs/sec forever |
| KVO | **~3s** | ~zero |

The old latency came from *forcing* the sync 3x/second. KVO waits for cfprefsd
to propagate, which measured ~3s. The 5s timer is now a reconciliation safety
net for inputs with no known key, not the mechanism.

## R18. Why KVO is slow — it is CFPrefs, not our code

Asked whether the ~3s KVO latency meant we were doing something wrong. Measured
by racing a forced-sync reader against the app, from the same AX press:

| observer | when it saw the change |
|---|---|
| forced `CFPreferencesAppSynchronize` every 50ms | **0.54s** |
| the app, via KVO | **3.86s** |

The write lands promptly; **CFPrefs sits on the cross-process notification for
~3.3s**. That is coalescing of foreign global-domain changes, not a mistake in
the observer setup.

Searched exhaustively for a faster event source. None exists that fires:

| route | result |
|---|---|
| `NSWorkspace.shared.notificationCenter` | never fired |
| `NotificationCenter.default` | never fired |
| `DistributedNotificationCenter.default()` | never fired |
| 6 candidate Darwin notification names | never fired |
| SkyLight's `kSLSCoordinatedIconAppearanceConfigurationChangeNotificationName`, resolved by `dlsym` and registered with `notify_register_dispatch` returning `NOTIFY_STATUS_OK`, as Darwin **and** distributed | never fired |

That constant is self-named — its value is the literal string
`kSLSCoordinatedIconAppearanceConfigurationChangeNotificationName`.

**Conclusion: KVO is the only working event source, and forcing the sync is the
only way to react promptly.** The shipped build is a hybrid — KVO plus a
`GN_POLL` reconcile pass, default 0.75s — giving **0.03-0.04s** response. The
forced sync measured 0.0% CPU in both the app and cfprefsd even at 3/sec, so the
cost of the poll is not the reason to avoid it; the reason to keep KVO is that it
catches changes the timer would otherwise wait on.

## R19. Code review — 33 raised, 26 confirmed, 7 refuted

A multi-lane review (correctness, SDK conformance, comment-vs-code, dead code)
with every finding adversarially verified before being accepted.

### Corrections to THIS document

Two claims above are stale and are corrected here rather than edited in place:

- **R7 (~line 226) and R10 (~line 288)** say in the present tense that the
  private `set_variant:` path is gated behind `GN_VARIANT`, off by default. No
  such reader exists any more, so R10's variant-4 row is not reproducible from
  current source.
- **§4 "Still open" (~line 127)** names `GN_PAPER` as the live, swept paper
  alpha. The paper alpha is `GN_PAPER_D` / `GN_PAPER_L`.

### Defects fixed

| what | why it mattered |
|---|---|
| The stderr line printed `paper=<GN_PAPERMODE>@<GN_PAPER>` | Neither knob reached any drawing code. The harness's only per-run record reported an alpha never applied — a sweep over `GN_PAPER` would have read as "inert knob" or mis-attributed a wallpaper/appearance difference. It now prints the values actually used. |
| `GN_PAPER`, `GN_PAPERMODE` declarations | Dead; only the log read them. Removed, and dropped from `run.sh`. |
| `Controller.cycle()` | Unreachable — no caller, not `@objc`, no mouse handling anywhere. The click-to-cycle behaviour its comments promised never happened and `styleOverride` was permanently nil. Removed along with `styleOverride` and the `source=click` branch. |
| Light/dark decided twice | `apply()` used `NSApp.effectiveAppearance` while `paperAlphaForAppearance` used the view's. They could disagree mid-change, darkening the material for one appearance and the paper for the other. Both now read the window's appearance. |
| `layout()` ran once from `init` | The window is `.resizable`; after any edge drag the centred title and the monospaced rows kept their original 560pt frames — mis-centred or clipped. The content view now drives a relayout on `setFrameSize`. |
| `LSMinimumSystemVersion` 26.0 | `swiftc` with no `-target` stamps minos 27.0, so LaunchServices refuses the bundle below 27.0 regardless. The plist advertised a floor the binary cannot honour. Now 27.0. |
| Three contradictory comment blocks | Superseded rationale had been left stacked above its replacement: `paperColor` documented wallpaper sampling that no longer exists; a block argued "Regular for every style" directly above the switch selecting `.clear`; and "tintColor stays nil even under Tinted" sat above the line that always sets it. All three deleted. |

### On KVO — what Apple actually documents

`UserDefaults.didChangeNotification`:

> "If a different process changes your app's settings, the system doesn't
> generate this notification. To detect changes made by another process,
> register a key-value observer on the UserDefaults object. Key-value observing
> reports all updates to setting values, regardless of which process made the
> change."

So KVO **is** the documented mechanism for cross-process changes, and
`didChangeNotification` would have been the wrong choice. Apple states no
latency guarantee. One caveat: that wording covers *"your app's settings"*,
while this app observes **NSGlobalDomain** keys written by System Settings —
outside what is actually promised, which may be why delivery is coalesced.

Also disproven: the hypothesis that our own forced `CFPreferencesAppSynchronize`
was contending the CFPrefs lock and causing the delay. With the reconcile pass
fully disabled, pure KVO was **slower** (13s vs 3.9s), so the sync was helping.
`GN_POLL=0` now genuinely disables the timer — it previously clamped to 0.05s,
i.e. 20 syncs/sec, the opposite of what the name implied.

# Session 3 — the Rust port

## R20. Port parity — measured, not asserted

*Not reproducible from this repository: every figure in this section — the
guards, the backdrop drift, the results, the eliminations, the stability run —
was produced by an out-of-tree harness (`capture.sh`, `ab.sh`, `sweep.sh`,
`winlist.swift`) driving `build/glassnote.app`, which nothing in-tree can build
any more. Recover the Swift oracle with the commands under "To continue this
investigation" before trying to re-run them.*

The Swift app was rewritten in Rust on `objc2` as the `glasspane` library plus a
`glassterm` example. This section is the evidence that the rewrite renders what
the original did.

### Method

Both builds launched in turn, pinned to the same screen rect, captured, and
compared with **MAE** (`-metric AE` remains unusable in this ImageMagick build —
see section 0). `GN_SHADOW=always` throughout, so the one deliberate behavioural
divergence is held constant and anything remaining is a porting difference.

Five guards, each added after it caught a wrong reading:

| guard | the reading it caught |
|---|---|
| refuse if the window is **absent** | a crop of the bare desktop, which looks like a result |
| refuse if the window is **occluded** | a full-screen terminal over the window read the body as **exactly 0.0** |
| refuse if the window is **not key** | the material renders differently unfocused — real numbers, wrong question |
| refuse if the window **moved** mid-run | the crop lands somewhere else entirely |
| refuse if the **backdrop drifted**, pixel-wise | see below |

### The backdrop is the whole problem

The glass samples what is behind it, so a comparison of two captures taken at
different times is only valid if the backdrop was identical at both. With an
animated wallpaper the bare desktop under the window measured **33.7 → 29.9 →
7.5 luma over 20 seconds**. Every sequential A/B taken against it was measuring
the wallpaper. An early run reported MAE 0.175 and "Rust is 16–22 luma brighter";
that reading is **withdrawn** — the same Rust binary measured 15.8 and 35.5
minutes apart.

A *mean*-luma drift guard is not enough. A desktop widget repainting behind the
glass changed **2028 pixels by >20/255 while moving the mean by 0.06**. The
guard compares the bare captures pixel-wise.

The test rect also has to avoid live widgets. At 120,120 it overlapped the
`City Digital` clock widget (x 8..188, y 41..221), whose minute tick discarded
runs at random. Measurements below use 200,740, which is clear of every widget
and of the terminal window.

### Results

Backdrop pixel-identical across all three checkpoints (bare-vs-bare MAE exactly
0). Window 560x360 pinned at 200,740. Deltas are Rust minus Swift, luma 0–100.

| token | MAE | band | body | text | |
|---|---|---|---|---|---|
| RegularLight | 0.000113 | +0.0000 | +0.0000 | +0.0000 | pixel-identical |
| RegularDark | 0.000161 | +0.0000 | +0.0000 | +0.0000 | pixel-identical |
| ClearLight | 0.000113 | +0.0000 | +0.0000 | +0.0000 | pixel-identical |
| ClearDark | 0.000161 | +0.0000 | +0.0000 | +0.0000 | pixel-identical |
| TintedLight | 0.000447 | **−0.3979** | +0.0000 | +0.0000 | band residual |
| TintedDark | 0.000522 | **−0.4188** | +0.0000 | +0.0000 | band residual |

**Control:** bare-vs-bare MAE `0`; bare-vs-window MAE `0.029`. So the largest
difference between the two implementations is **1.8% of the effect the window
has on its own backdrop**.

### The one open residual

Under **Tinted only**, the title band reads ~0.40–0.42 luma darker in the Rust
build. Body and text are exact. The band is the only region with no black paper
over it, so anything affecting the bare material shows there and is masked
elsewhere.

Note this is **not** the same as the band being bare. Under Tinted — the only
family with a residual — the band carries the theme wash at full `GN_TINT`
alpha, which is itself a candidate the four eliminations below did not isolate.

Four candidate causes were tested and **eliminated**. The residuals recorded
against those tests (−0.4328 / −0.4312) do **not** match the Results table's
−0.3979 / −0.4188; the two sets were taken under different conditions and the
discrepancy is unresolved, so no single "unchanged at" figure is stated here:

| hypothesis | test | result |
|---|---|---|
| material darkening colour built in sRGB rather than calibrated HSB | `GN_GT_L=0` on both builds | −0.4328, unchanged — **Tinted ▸ Light only**; `GN_GT_D` gates the Dark half and was never run |
| `NSBezierPath::fillRect` vs the Swift's `NSRect.fill()` | swapped to `NSRectFill` | −0.4312 unchanged, and whole-window MAE **18x worse** (0.0082) — `NSRectFill` composites with Copy and wipes the material |
| forcing layer-backing on the glass view | removed `setWantsLayer` | byte-identical MAE; it is a no-op when the surface is the window's content view |
| converting the system tint colour to sRGB | `GN_TINT_SPACE=native` vs `srgb` | native is better and is now the default — see below |

The cause is **not established**. It is bounded, confined to one region under
one style family, and below the 0.0005 MAE threshold for five of six tokens (only TintedDark, at
0.000522, exceeds it).

**To continue this investigation** the Swift oracle would be needed as a second
build to A/B against. It is no longer part of this repository. The A/B harness
also lives outside the repo (`capture.sh`, `ab.sh`, `sweep.sh`,
`winlist.swift`).

### Decisions this measurement settled

**`GN_TINT_SPACE=native` is the default.** Converting the system theme colour to
sRGB before washing with it spread a **+0.11 luma error across band, body and
text** (MAE 0.00112 against 0.00045). Native is what the reference was fitted
against: the system renders its own widgets with the colour natively.

**sRGB for everything else is safe but unproven.** All *drawn* colours are built
in explicit sRGB rather than the device space the Swift used. Measured, this
makes no difference on this display — which means it costs nothing here, not
that its benefit is demonstrated. Proving display-independence needs a second
display with a different profile, which this machine does not have.

**`NSRectFill` is not a drop-in for `NSBezierPath::fillRect`.** It composites
with Copy, wiping the material rather than dimming it: 18x worse MAE.

### A bug the harness found

The style was applied by the observer callback, which was constructed **after**
`window.show()`. The window was therefore on screen for one or more frames with
no material tint. A capture that raced that gap read the band **3.9 luma
brighter** — roughly 10x the real difference between the two implementations,
and it would have been read as a porting error. With the observer constructed
before `show()`, five consecutive launches measure byte-identical where before
they varied and two of five were discarded outright.

### R17's crash, avoided rather than re-hit

The KVO handler only ever enqueues, via
`performSelectorOnMainThread:…:waitUntilDone:NO`, which is documented to defer
even when already on the main thread. **`dispatch2::run_on_main` would
reproduce the crash exactly** — its documentation states "if the current thread
is the main thread, this runs the closure", and KVO arrives on the main thread.
That crate is deliberately not a dependency.

Live following was measured end to end: writing `ClearDark` to the key was
picked up within **1s**, and the restore likewise, with no crash.

## R21. The API rework is visually a no-op — measured, not assumed

Three reviews (macOS/AppKit, SDK durability, Rust API Guidelines) produced a
large breaking change to the public API: `WidgetStyle` re-keyed on a token type,
`GlassSurface::new` returning `Result`, `clone_handle` replaced by `Clone`,
`label()` replaced by `Display`, `Option<Duration>` replaced by `Reconcile`, and
the light/dark resolver moved to the crate root.

Two of those touch the *rendering* path in `examples/common/`, so "it is only an
API change" needed checking rather than asserting:

* `resolve_is_dark` was a 12-line hand-rolled copy of the resolver; it now calls
  `glasspane::is_dark`. Same algorithm, but a different call site.
* the tint went from `style.tint.as_ref().filter(..).map(|c| c.clone())` to
  `style.tint().map(|c| c.retain())` — a borrow plus an explicit retain instead
  of cloning a `Retained`.

**Method.** Built `glassterm` at the pre-rework and post-rework revisions in release,
swapped the binary inside one `.app` so the bundle identity was constant, and
captured each with the same window pinned to the same screen position over the
same static wallpaper. The capture refuses rather than guesses: it verifies the
process is frontmost, that the window actually reached the requested position,
that no window listed in front of it intersects its rectangle, and that the
bounds are unchanged after the screenshot.

**Result — byte-identical.**

| | before | after |
|---|---|---|
| mean luma | 37.5271 | 37.5271 |
| sha256 | `c11af699…1eb111a3` | `c11af699…1eb111a3` |
| MAE | — | **0** |
| max per-pixel difference | — | **0** |

Identical SHA-256, so the two PNGs are the same file content. MAE 0 and a
maximum per-pixel difference of 0 confirm it independently of the hash, which
matters because the `AE` metric is broken in this ImageMagick build and only
`MAE` and an explicit difference composite are trustworthy here.

### What this does NOT cover — the condition the result holds under

**Both captures ran under `Clear ▸ Light` with default features**, and that
matters, because in that state `WidgetStyle::tint()` is `None` and the closure
that changed never executes. The tint is `Some` only under the three Tinted
tokens, and only with `private-spi` on — without it the theme colour has no
public source and is always `None`.

So of the two rendering-path changes this record set out to check, the A/B
covers `resolve_is_dark` and **does not cover the tint**. The equivalence there
rests on reading rather than measurement: `Message::retain(&self) ->
Retained<Self>` on a borrowed `&NSColor` performs the same retain that
`Retained::clone` did, so the colour reaching `setTintColor:` is the same object
either way.

Closing that gap needs a Tinted capture, which needs the real Settings UI —
§1.1's finding that `defaults write` does not apply this setting is what makes
it awkward, and it has not been done. Stated rather than left implied: a result
whose precondition is unwritten reads as broader than it is, which is the exact
defect the provenance audit corrected ~40 instances of.

The record line also differs exactly as expected and nowhere else:
`poll=Some(750ms)` before, `poll=Every(750ms)` after — the `Reconcile` enum's
`Debug`, which is the only user-visible string the rework changed.

### Occlusion, again

The existing `capture.sh` refused both runs with `DISCARDED: window is
occluded`, and the diagnosis was wrong: the window was on screen, frontmost, and
unobstructed. `winlist -occ` simply failed to match the target and reported its
"no windows matched" message through the occlusion path, which reads as
"something is covering it". A second window *was* overlapping (WezTerm, sharing
x 912–960) but behind, which is harmless.

Worth recording because it is the same failure shape as R20's: a guard that
refuses is only useful if its refusal names the real reason. This one cost a
false "cannot verify" before the geometry was checked by hand.
