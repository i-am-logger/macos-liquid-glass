<div align="center">

[![crates.io](https://img.shields.io/crates/v/macos-liquid-glass?logo=rust&logoColor=white)](https://crates.io/crates/macos-liquid-glass)
[![Downloads](https://img.shields.io/crates/d/macos-liquid-glass?logo=rust&logoColor=white)](https://crates.io/crates/macos-liquid-glass)
[![MSRV](https://img.shields.io/crates/msrv/macos-liquid-glass?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![CI](https://img.shields.io/github/actions/workflow/status/i-am-logger/macos-liquid-glass/ci.yml?branch=master&label=CI&logo=githubactions&logoColor=white)](https://github.com/i-am-logger/macos-liquid-glass/actions/workflows/ci.yml)
[![docs.rs](https://img.shields.io/docsrs/macos-liquid-glass?logo=docsdotrs&logoColor=white)](https://docs.rs/macos-liquid-glass)

[![Nix](https://img.shields.io/badge/Nix-2b2b2b?logo=nixos&logoColor=white)](https://nixos.org)
[![devenv](https://img.shields.io/badge/devenv-2b2b2b?logo=nixos&logoColor=white)](https://devenv.sh)
[![macOS](https://img.shields.io/badge/macOS%2026%2B-2b2b2b?logo=apple&logoColor=white)](#requirements)
[![License: MIT](https://img.shields.io/badge/MIT-2b2b2b?logo=opensourceinitiative&logoColor=white)](LICENSE)

# macos-liquid-glass

**Liquid Glass windows on macOS** — that follow
**System Settings ▸ Appearance ▸ Icon & widget style**.

The setting that restyles desktop widgets, tracked live.

![Two windows over a desktop: a titled one with traffic lights, and a borderless one with none](https://raw.githubusercontent.com/i-am-logger/macos-liquid-glass/master/docs/macos-liquid-glass.jpg)

</div>

The same window under three of the nine Icon & widget style tokens — Default,
Dark, and Tinted taking the system accent — over an identical backdrop:

![The same window rendered under the Default, Dark and Tinted icon styles](https://raw.githubusercontent.com/i-am-logger/macos-liquid-glass/master/docs/styles.jpg)


Putting a window on `NSGlassEffectView` is the easy part. Making it *agree* with
the widget style, and keep agreeing as the user changes it, is not: the
preference holds nine tokens behind a four-option UI, no notification centre
posts a change, and the key-value observer that does fire must never read a
preference from inside its own callback.

This crate handles that, and gets out of the way — it hands you an `NSWindow`
and an `NSView`, so it composes with whatever you already have.

## Using it

```toml
[dependencies]
macos-liquid-glass = "1.0.0-beta.1"
```

The API takes and returns objc2 types — `MainThreadMarker` and `NSSize` from
`objc2-foundation`, `NSColor`, `NSView` and `NSAppearance` from `objc2-app-kit`,
`CGFloat` from `objc2-core-foundation`. You do **not** need to declare those
yourself: `macos-liquid-glass` re-exports the exact versions it was compiled against, so
`macos_liquid_glass::objc2_foundation::MainThreadMarker` cannot be a different type from
the one its own signatures use.

Declaring them separately also works, and is nicer to read — but then it is on
you to keep the versions unifying. A consumer on `objc2-foundation` 0.2 against
a `macos-liquid-glass` built on 0.3 gets `expected MainThreadMarker, found
MainThreadMarker`, which is a genuinely confusing error to receive.

```rust
use macos_liquid_glass::glass::{GlassStyle, GlassSurface};
use macos_liquid_glass::icon_style::{Reconcile, StyleObserver};
use macos_liquid_glass::window::GlassWindow;
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mtm = MainThreadMarker::new().expect("main thread");
    let size = NSSize::new(560.0, 360.0);
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);

    let window = GlassWindow::new(mtm, size, "example");
    let glass = GlassSurface::new(mtm, frame, GlassStyle::Clear, 16.0)?;
    window.set_content_view(glass.view());

    // `Clone` retains rather than copies — both handles address the one window.
    // The closure outlives this scope, so it needs its own.
    let win = window.clone();
    // Fires immediately with the current style, then on every change.
    let _observer = StyleObserver::new(mtm, Reconcile::default(), move |style| {
        win.set_appearance(style.appearance().as_deref());
        // style.token(), style.is_tinted(), style.tint(), style.to_string() ...
    });

    window.show();
    Ok(())
}
```

Hold the `StyleObserver`: KVO does not retain its observer, and dropping it is
what unregisters.

The same example is a compiled doctest in `src/lib.rs`, so the API it names
cannot drift.

### Two window shapes

`GlassWindow::new` is a **normal window** that happens to be made of glass:
titled, with a transparent titlebar and a hidden title, content extending
underneath. It looks undecorated and behaves like every other macOS window.

`GlassWindow::borderless` has no titlebar at all — for a HUD, a desktop widget,
or a measurement target. It gives up more than decoration, because the missing
behaviours live in `NSThemeFrame`. Measured on macOS 27.0 (26A5406e):

| behaviour | `borderless` | `new` |
|---|---|---|
| close, zoom | work | work |
| **minimise** | **does nothing** — adding `Miniaturizable` to the mask does not help | works |
| **double-click titlebar** | nothing tracks it | works, honouring your System Settings preference |
| **traffic lights** | **none** — `standardWindowButton` returns nil | AppKit creates and wires them |
| focus | needs a `canBecomeKeyWindow` override, which this crate applies | free |

Neither example fakes the missing chrome. Hand-adding detached standard buttons
to a borderless window is possible and they even look right, but the minimise
one is inert — so the borderless window shows no buttons at all rather than a cluster
where one third does nothing.

`examples/titled.rs` is the normal window; `examples/borderless.rs` is the
borderless one, and is the surface every measurement in `MEASUREMENTS.md`
was taken against.

### Features

| feature | default | what it brings |
|---|---|---|
| `glass` | yes | the `NSGlassEffectView` surface and its runtime availability guard |
| `window` | yes | a transparent window hosting one glass surface, titled or borderless |
| `icon-style` | yes | the Icon & widget style tracker — **usable on its own** |
| `private-spi` | **no** | two undocumented selectors — see below |

`macos_liquid_glass::is_dark` and `macos_liquid_glass::accessibility` are behind no feature at
all: the first is the crate's only light/dark resolver and both halves need it,
the second is an obligation rather than an option.

`icon-style` does not depend on `window`, so a consumer that already has an
`NSWindow` can take just the tracker:

```toml
macos-liquid-glass = { version = "1.0.0-beta.1", default-features = false, features = ["icon-style"] }
```

`private-spi` is **off by default** because App Store Review Guideline 2.5.1 is
"Apps may only use public APIs", with no `respondsToSelector:` exemption — a
crate reaching private selectors by default would hand every consumer a
submission liability they never opted into and cannot see. Without it
`icon-style` still tracks the style: the nine-token enum, the CFPreferences
read, KVO and the reconcile pass are all public API. Only `WidgetStyle::tint()`
becomes unavailable, because the theme colour has no public source.

### Accessibility

A translucent surface has to answer to **Reduce Transparency**. This crate
reports the setting and does **not** act on it for you:

```rust
if macos_liquid_glass::accessibility::reduce_transparency() {
    // Build the opaque variant — no GlassSurface.
}
```

That split is deliberate. `NSGlassEffectView` has no opacity control and its
whole function is to sample and blur what is behind the window, so honouring the
setting means *not using the material at all* and substituting opaque content —
with the colours, contrast and layout that go with it. That substitute is the
caller's content, not something a wrapper around one AppKit view can synthesise.

What it must not be is silent, which is what it was: a crate that spends most of
its size following a *cosmetic* preference and never mentions an *accessibility*
one has its priorities backwards, however the code is factored.

Unlike the Icon & widget style, these settings do post a notification —
`NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification`, on the
*workspace's* notification centre rather than `NSNotificationCenter.default`.
`increase_contrast()`, `differentiate_without_color()` and `reduce_motion()` are
there too.

## Requirements

macOS 26 or later at **runtime** — `NSGlassEffectView` does not exist before it,
which is why `glass::is_supported()` is a class lookup rather than a version
comparison. On every other platform the crate is empty rather than absent, so a
cross-platform consumer can depend on it unconditionally.

## Examples

Two runnable examples share one content module and differ only in which
constructor they call:

```sh
cargo run --example titled       # GlassWindow::new
cargo run --example borderless   # GlassWindow::borderless
```

Both render a mock terminal on glass and print the style they resolved. They
carry their own environment knobs for sweeping the calibration constants; those
are documented on each knob in `examples/common/mod.rs`, and are a property of
the examples, not of this library — the library reads no environment variables.

`cargo xtask run` builds, asserts freshness, bundles a real `.app` and launches
it, which is what the visual measurements are taken against.


## Development

```sh
direnv allow     # or: devenv shell
dev-ci           # treefmt + clippy --all-targets + check + test (incl. doctests)
```

Every job in `ci.yml` runs on macOS. On Linux the library is not *broken*, it is
**empty** — every item is `cfg(target_os = "macos")` — so a green ubuntu run
would build nothing and prove nothing, and `--all-targets` fails there anyway
because the example is not cfg-gated. The release jobs run on macOS for the same
reason: `semver_check` on Linux would compare two empty API surfaces and pass a
breaking change straight through.

## Known limitations

A macos-liquid-glass surface reads **less saturated than a system desktop widget**: on an
identical backdrop the widget's bare glass carries chroma 45.9 against this
window's 32.6, roughly a 40% difference. The *hue* matches closely — B/R 1.71
against the widget's 1.77 over the same backdrop — so the gap is saturation, not
colour. Closing it appears to need an undocumented material variant, which this
crate does not use.
