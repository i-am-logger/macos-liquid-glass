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
dev-ci
```

This is not an approximation of CI — `.github/workflows/ci.yml` runs
`devenv test`, which runs the same `tasks."test:*"` set defined in
`devenv.nix`. A green `dev-ci` locally means a green PR check, because both
execute the same nix-defined suite. If you want to run one piece while
iterating, `enterShell` lists the individual scripts (`dev-fmt`, `dev-lint`,
`dev-check`, `dev-test`).

**Add a check by adding a task in `devenv.nix`, never by adding a step to the
workflow.** A step that exists only in CI is a step you cannot run locally, and
that is exactly the divergence this setup exists to prevent.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/). release-plz reads
them to compute the next version and write `CHANGELOG.md`, so the prefix
decides the release:

- `fix:` → patch
- `feat:` → minor
- `feat!:` / `BREAKING CHANGE:` in the body → major

`chore:`, `docs:`, `test:`, `refactor:` and `ci:` do not trigger a release.

## Releases

Releases are automated and gated. release-plz opens a `chore: release` PR that
bumps the version and writes the changelog; **merging that PR is what
authorises the publish to crates.io.** Do not bump versions by hand and do not
publish manually — release-plz reads its baseline from crates.io, and a manual
publish or a hand-edited version desynchronises it.

The one exception is the **first** published version. With nothing on crates.io
there is no baseline to bump from, so release-plz proposes whatever `Cargo.toml`
already says. That version is therefore set by hand, in an ordinary PR, before
the first release PR is merged.

## Code of Conduct

This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
