# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0-beta.2](https://github.com/i-am-logger/macos-liquid-glass/compare/v1.0.0-beta.1...v1.0.0-beta.2) - 2026-08-19

### Docs

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
