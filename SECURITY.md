# Security Policy

`macos-liquid-glass` is a macOS-only library crate. It opens no network
connections, parses no document formats, and exposes no CLI or binary — on
every other target it compiles to nothing. The security-relevant surface is:

- **Objective-C FFI** — about twenty `unsafe` blocks in library code, spread
  across `src/icon_style.rs`, `src/window.rs`, `src/lib.rs` and `src/glass.rs`.
  Ten are hand-written `msg_send!` signatures, where a wrong arity, argument
  type or return type is undefined behaviour rather than a compile error. The
  rest are `extern` static reads (`NSAppearanceName*`, `kCFPreferences*`),
  `define_class!` method overrides, and the observer registrations below. Each
  carries a `SAFETY` comment naming the invariant it depends on.
- **Observer lifetime** — `StyleObserver` registers KVO observers on
  `NSUserDefaults.standard` and may schedule a repeating `NSTimer` targeting
  itself. `Drop` unregisters both; a missed unregistration leaves the
  Objective-C runtime holding a dangling pointer.
- **Uncatchable aborts** — an Objective-C exception crossing back into Rust
  aborts the process and cannot be caught: `fatal runtime error: Rust cannot
  catch foreign exceptions`. Any path that can raise one is a denial of service
  in the host application. The known raising inputs are guarded — non-finite
  sizes are rejected before `initWithContentRect:`, and private properties are
  reached by `respondsToSelector:` rather than by KVC, which raises on an
  absent key.
- **`private-spi`** — off by default. It sends undocumented AppKit selectors
  (`currentIconAppearanceConfiguration`, `resolvedIconTintColor`,
  `iconAppearanceTheme`) that carry no availability contract, so a macOS point
  release can change their behaviour without notice. App Store Review Guideline
  2.5.1 permits public APIs only, which makes enabling it a submission
  liability. Cargo features unify across the dependency graph: if any crate in
  the build turns it on, it is on for every crate in that build. Audit with
  `cargo tree -e features -i macos-liquid-glass`.
- **Preference input** — `AppleIconAppearanceTheme` is read from the current
  user's global CFPreferences domain, and `src/accessibility.rs` reads the
  `NSWorkspace` accessibility display settings. These are the crate's external
  inputs; an unrecognised preference value resolves to a known token rather
  than escaping.
- **Dependency vulnerabilities** — `RUSTSEC` advisories in the `objc2` tree.

## Reporting a Vulnerability

Please report security issues privately rather than opening a public issue —
[GitHub Security Advisories](https://github.com/i-am-logger/macos-liquid-glass/security/advisories/new)
if private vulnerability reporting is enabled on the repo, otherwise contact
the maintainer directly via GitHub ([@i-am-logger](https://github.com/i-am-logger)).

Include the affected version or commit, a minimal reproduction, and the impact
as you see it. There is no bug bounty — this is a solo-maintained open-source
project — but reports are taken seriously and credited unless you ask
otherwise.

## Supported Versions

Only the latest published release is supported. There is no LTS branch.
