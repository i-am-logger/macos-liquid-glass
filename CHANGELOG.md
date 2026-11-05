# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0-beta.8](https://github.com/i-am-logger/macos-liquid-glass/compare/v1.0.0-beta.7...v1.0.0-beta.8) - 2026-11-05

### Feat

- titlebar accessories, which travel with the traffic lights

## [1.0.0-beta.7](https://github.com/i-am-logger/macos-liquid-glass/compare/v1.0.0-beta.6...v1.0.0-beta.7) - 2026-11-03

### Docs

- key releases, Caps Lock and the 13.3 lookup say what they do

### Feat

- a key release reaches the responder
- a modifier key alone reaches the responder
- the first click into an inactive window reaches the view
- dim_flashing_lights, read at run time beside reduce_motion
- ModifierKey and ModifierChange are non_exhaustive
- an app delegate for more than one window, and a File menu

### Changed behaviour

- `acceptsFirstMouse:` now returns YES, so the click that activates an inactive window reaches `Responder::mouse`; AppKit's default drops it.
- Close (Command-W) moves from the Window menu to a new File menu, beside New Window (Command-N), which is disabled unless the app installs `AppDelegate`.

## [1.0.0-beta.6](https://github.com/i-am-logger/macos-liquid-glass/compare/v1.0.0-beta.5...v1.0.0-beta.6) - 2026-10-18

### Feat

- a standard-material surface over NSVisualEffectView

## [1.0.0-beta.5](https://github.com/i-am-logger/macos-liquid-glass/compare/v1.0.0-beta.4...v1.0.0-beta.5) - 2026-09-20

### Docs

- name the types the default features do not build, rather than linking to them

### Feat

- the surface a terminal needs -- appearance, colour space, menu, icon
- a Waker that reaches the view from any thread, and a wake for the responder
- Responder::focus takes Focus, not a bool
- Surface, a BGRA IOSurface with a locked write
- a drawable content view, a display link, a responder, and three window helpers

## [1.0.0-beta.4](https://github.com/i-am-logger/macos-liquid-glass/compare/v1.0.0-beta.3...v1.0.0-beta.4) - 2026-08-19

### Build

- use the macOS 26 SDK, not nixpkgs' 14.4 default

### Docs

- replace the inherited latency figures with measured ones

### Feat

- [**breaking**] poll at 50ms and take the interval off the public API

## [1.0.0-beta.3](https://github.com/i-am-logger/macos-liquid-glass/compare/v1.0.0-beta.2...v1.0.0-beta.3) - 2026-08-19

### Docs

- say plainly that IconStyle is a lossy grouping

### Feat

- let a window follow the icon style without holding the observer

## [1.0.0-beta.2](https://github.com/i-am-logger/macos-liquid-glass/compare/v1.0.0-beta.1...v1.0.0-beta.2) - 2026-08-19

### Docs

- rewrite the documentation to state what is, not how it got here
- restore the downloads badge
- match rav's centred README header

### Refactor

- name the examples after what they demonstrate

## [1.0.0-beta.1]

First published release. A pre-release: the API is what 1.0.0 is intended to
freeze, and this exists to exercise the release pipeline before that promise is
made.

### Added

- `GlassSurface` — a Liquid Glass (`NSGlassEffectView`) material surface, with
  `is_supported()` for the macOS 26 runtime check and `GlassError` for the
  failure it reports.
- `GlassWindow` — a transparent window hosting one surface. `new` builds a
  titled window; `borderless` builds one with no chrome, and documents the
  behaviours that costs.
- `StyleObserver` and `WidgetStyle` — live tracking of **System Settings ▸
  Appearance ▸ Icon & widget style**, including the nine-token
  `IconAppearanceToken` behind the four-option UI.
- `macos_liquid_glass::is_dark` — light/dark resolution that handles vibrant and
  accessibility appearances, available without any feature.
- `macos_liquid_glass::accessibility` — the four `NSWorkspace` display settings, notably
  Reduce Transparency.
- Re-exports of `objc2`, `objc2-app-kit`, `objc2-core-foundation` and
  `objc2-foundation` at the versions this crate was compiled against.

### Notes

- macOS only. The glass surface additionally requires macOS 26 at runtime, which
  is checked rather than assumed; on other platforms the crate is empty rather
  than absent, so it can be depended on unconditionally.
- The public API is parameterised by `objc2` types, which are pre-1.0. A
  semver-incompatible `objc2` release therefore forces a new major version of
  this crate even when its own API is unchanged.
- `private-spi` is off by default and reaches one undocumented AppKit selector.
  Cargo features unify across the dependency graph — see `Cargo.toml`.
