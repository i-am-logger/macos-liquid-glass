# Contributing

## Setup

This project uses [devenv](https://devenv.sh) to pin the exact Rust toolchain
and tool versions CI uses:

```bash
direnv allow   # or: devenv shell
```

Everything below assumes you are inside that shell (or run each command as
`devenv shell -- <command>`).

## Before opening a PR

Run the full local pipeline:

```bash
devenv test
```

This is not an approximation of the `Lint/UT` job: `.github/workflows/ci.yml`
runs `devenv test` as well, and `devenv test` runs the `tasks."test:*"` set
defined in `devenv.nix`, so both execute the same nix-defined suite by
construction. `MSRV` is a separate job — the shell pins one toolchain and ships
no rustup, so it cannot be a task.

While iterating, `dev-ci` is a faster subset — treefmt, clippy, `cargo check`
and `cargo test` at default features — and the individual scripts `enterShell`
lists (`dev-fmt`, `dev-lint`, `dev-check`, `dev-test`) are narrower still.
Neither covers `test:features`, `test:docs` or `test:package`, so neither
substitutes for `devenv test` before a PR.

`.github/workflows/ci.yml` runs `devenv test`, which runs the `tasks."test:*"`
set defined in `devenv.nix`: `test:fmt`, `test:clippy`, `test:check`,
`test:unit`, `test:features`, `test:docs` and `test:package`. Run `devenv test`
to reproduce the `Lint/UT` job exactly.

`dev-ci` is a faster subset — fmt, clippy, check and test at default features.
It skips the eight-combination feature matrix, the per-combination rustdoc
lints and `cargo package`, so a green `dev-ci` does not imply a green PR check.
To run one piece while iterating, `enterShell` lists the individual scripts
(`dev-fmt`, `dev-lint`, `dev-check`, `dev-test`).

CI also runs an `MSRV` job, the one check that is not a devenv task: the shell
pins a single toolchain and ships no rustup, so it cannot compile against the
second compiler that check needs. Nothing local covers it.

**Add a check by adding a task in `devenv.nix`, not by adding a step to the
workflow.** A step that exists only in CI is a step you cannot run locally, and
that is exactly the divergence this setup exists to prevent.

The one standing exception is the `MSRV` job. The dev shell pins a single
toolchain and ships no rustup, so it cannot compile against a second compiler;
verifying a different toolchain is the one thing a pinned shell cannot do. It
reads `rust-version` from `Cargo.toml`, so it tests the MSRV that is actually
declared.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/). release-plz reads
them to compute the next version and write `CHANGELOG.md`, so the prefix
decides the release:

- `fix:` → patch
- `feat:` → minor
- `feat!:` / `BREAKING CHANGE:` in the body → major

`release_commits` is unset, so release-plz considers every commit: anything
that is not `feat:` or a breaking change yields a patch bump. Only commits
touching packaged files count — a change confined to `.github/`, `devenv.*`,
`nix/` or `xtask/` produces no release, because nothing in the published
package moved.

## Releases

Releases are automated and gated on CI. When master carries a version that is
not yet on crates.io, and both `Lint/UT` and `MSRV` have passed on that exact
tree, release-plz publishes it, tags it and creates the GitHub release.

Normally you do not touch the version at all: land commits using
[Conventional Commits](https://www.conventionalcommits.org/), and release-plz
opens a `chore: release` PR that bumps `Cargo.toml` and writes `CHANGELOG.md`.
Merging that PR is what ships. Do not publish manually — release-plz reads its
baseline from crates.io, and a manual publish desynchronises it.

Hand-editing the version is reserved for the first release of a new version
series, where there is no baseline for release-plz to bump from.

## Code of Conduct

This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
