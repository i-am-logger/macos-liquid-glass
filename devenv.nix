{ pkgs
, config
, lib
, ...
}:
let
  cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
  packageName = cargoToml.package.name;
  packageVersion = cargoToml.package.version;
  packageDescription = cargoToml.package.description or "";
in
{
  # Set root explicitly for flake compatibility
  devenv.root = lib.mkDefault (builtins.toString ./.);

  dotenv.enable = true;
  imports = [
    ./nix/rust.nix
  ];

  # Additional packages for development
  packages = [
    pkgs.git
    pkgs.pkg-config
  ];

  # Development scripts
  scripts.dev-test.exec = ''
    echo "Running tests..."
    RUSTFLAGS="-D warnings" cargo test --all-features
  '';

  scripts.dev-fmt.exec = ''
    echo "Checking formatting..."
    treefmt --fail-on-change
  '';

  # --all-targets, because the app lives in examples/. Without it clippy sees
  # only the library and the reference app goes unlinted.
  scripts.dev-lint.exec = ''
    echo "Running clippy..."
    cargo clippy --quiet --all-targets -- -D warnings
  '';

  scripts.dev-check.exec = ''
    echo "Checking compilation..."
    cargo check --quiet
  '';

  scripts.dev-ci.exec = ''
    echo "Running full CI pipeline locally..."
    echo "=== fmt ==="
    treefmt --fail-on-change || { echo "FAILED: fmt"; exit 1; }
    echo "=== clippy ==="
    cargo clippy --quiet --all-targets -- -D warnings || { echo "FAILED: clippy"; exit 1; }
    echo "=== check ==="
    cargo check --quiet || { echo "FAILED: check"; exit 1; }
    echo "=== test ==="
    RUSTFLAGS="-D warnings" cargo test --quiet || { echo "FAILED: test"; exit 1; }
    echo "=== ALL PASSED ==="
  '';

  # The library has nothing to run; the `borderless` example is the reference
  # app. The xtask
  # does the part `cargo run` cannot: an unbundled, background-launched process
  # cannot activate on macOS 14+, so its window is never key, and the material
  # renders differently when the window is not key.
  scripts.dev-run.exec = ''
    cargo xtask run "$@"
  '';

  scripts.dev-build.exec = ''
    echo "Building ${packageName}..."
    cargo build --release
  '';

  # Environment variables
  env = {
    CARGO_TARGET_DIR = "./target";
  };

  # Development shell setup.
  #
  # The whole banner goes to stderr. `devenv shell -- <cmd>` runs this first, so
  # anything on stdout is prepended to that command's output - and CI runs
  # `devenv shell -- cargo metadata` as Swatinem/rust-cache's cmd-format probe,
  # where a banner in front of the JSON is unparseable. The action swallows that
  # error, keeps the step green, and caches nothing.
  enterShell = ''
    {
      clear
      ${pkgs.figlet}/bin/figlet "${packageName}"
      echo
      {
        ${pkgs.lib.optionalString (packageDescription != "") ''echo "• ${packageDescription}"''}
        echo -e "• \033[1mv${packageVersion}\033[0m"
        echo -e " \033[0;32m✓\033[0m Development environment ready"
      } | ${pkgs.boxes}/bin/boxes -d stone -a l -i none
      echo
      echo "Available scripts:"
      echo "  dev-ci        - Run full CI pipeline (treefmt + clippy + check + test)"
      echo "  dev-test      - Run tests"
      echo "  dev-fmt       - Check formatting"
      echo "  dev-lint      - Run clippy"
      echo "  dev-check     - Check compilation"
      echo "  dev-run       - Run the application"
      echo "  dev-build     - Build the application"
      echo ""
    } >&2
  '';

  # https://devenv.sh/integrations/treefmt/
  #
  # One formatter for the whole tree, not just Rust — a Nix or shell file that
  # CI reformats is as much a diff-noise source as an unformatted .rs. `treefmt`
  # is the single entry point for `dev-fmt`, `tasks."test:fmt"`, and the
  # pre-commit hook, so all three can only ever agree.
  treefmt = {
    enable = true;
    config = {
      settings.global.excludes = [
        ".devenv.flake.nix"
        ".devenv/"
      ];

      programs = {
        # Nix
        nixpkgs-fmt.enable = true;
        deadnix = {
          enable = true;
          no-underscore = true;
        };
        statix.enable = true;

        # Rust — use the devenv toolchain so the formatter understands the
        # same edition the compiler does.
        rustfmt = {
          enable = true;
          package = config.languages.rust.toolchainPackage;
        };

        # Shell
        shellcheck.enable = true;
        shfmt.enable = true;
      };
    };
  };

  # https://devenv.sh/git-hooks/
  git-hooks.settings.rust.cargoManifestPath = "./Cargo.toml";

  # Use the same Rust toolchain for git-hooks as for development
  git-hooks.tools = {
    cargo = lib.mkForce config.languages.rust.toolchainPackage;
    clippy = lib.mkForce config.languages.rust.toolchainPackage;
    rustfmt = lib.mkForce config.languages.rust.toolchainPackage;
  };

  # treefmt only. A clippy hook would run with its own default flags - no
  # --all-targets, no --all-features - which is a different cargo fingerprint
  # from the test:clippy task, so it compiles the crate under clippy a second
  # time and cannot fail anything that task does not. Formatting is the part
  # worth catching before the commit rather than after.
  git-hooks.hooks = {
    treefmt.enable = true;
  };

  # https://devenv.sh/tasks/
  #
  # These tasks ARE the check suite. `enterTest` runs them, so `devenv test`
  # locally and `devenv test` in .github/workflows/ci.yml execute the same
  # thing by construction — the workflow is a thin wrapper, and the two cannot
  # drift apart.
  #
  # The `after` edges are load-bearing. devenv runs tasks with no edge between
  # them concurrently, so without these, fmt, clippy, check and test all invoke
  # cargo at once and block on the single lock over ./target. Serialised, they
  # run cheapest-first — a formatting failure costs no compilation at all — and
  # each later task reuses what the previous one compiled.
  tasks = {
    "test:fmt" = {
      exec = "treefmt --fail-on-change";
    };

    "test:clippy" = {
      exec = "cargo clippy --quiet --all-targets -- -D warnings";
      after = [ "test:fmt" ];
    };

    "test:check" = {
      exec = "cargo check --quiet";
      after = [ "test:clippy" ];
    };

    "test:unit" = {
      exec = "RUSTFLAGS='-D warnings' cargo test --quiet";
      after = [ "test:check" ];
    };

    # Every feature combination, because `cargo check` at default features
    # cannot see a module that only compiles when a feature is off, and doc
    # examples are gated per feature set. Uses `test`, not `check`: `check`
    # does not compile doctests.
    "test:features" = {
      exec = ''
        set -eu
        for f in "" "window" "glass" "icon-style" "window,glass" \
                 "icon-style,private-spi" "window,glass,icon-style" \
                 "window,glass,icon-style,private-spi"; do
          echo "--- features: ''${f:-<none>}"
          if [ -z "$f" ]; then
            cargo test --quiet --no-default-features
          else
            cargo test --quiet --no-default-features --features "$f"
          fi
        done
      '';
      after = [ "test:unit" ];
    };

    # rustdoc-only lints. `broken_intra_doc_links` is reported by nothing else,
    # and a link into a feature-gated module resolves only when that feature is
    # on — so this runs per combination too.
    "test:docs" = {
      exec = ''
        set -eu
        for f in "" "window" "glass" "icon-style" "window,glass,icon-style,private-spi"; do
          echo "--- doc: ''${f:-<none>}"
          if [ -z "$f" ]; then
            RUSTDOCFLAGS='-D warnings' cargo doc --quiet --no-deps --lib --no-default-features
          else
            RUSTDOCFLAGS='-D warnings' cargo doc --quiet --no-deps --lib \
              --no-default-features --features "$f"
          fi
        done
      '';
      after = [ "test:features" ];
    };

    # NOTE: MSRV is deliberately NOT a task here. This shell pins exactly one
    # toolchain and ships no rustup, so `cargo +<version>` cannot resolve and
    # there is no second compiler to test against. It runs as its own CI job
    # instead — the one check that cannot be a task, because verifying a
    # different toolchain is the thing a pinned shell cannot do.

    # What crates.io will actually receive.
    "test:package" = {
      exec = "cargo package --quiet --allow-dirty";
      after = [ "test:docs" ];
    };
  };

  # https://devenv.sh/tests/
  enterTest = lib.mkForce ''
    devenv tasks run test:fmt test:clippy test:check test:unit \
                    test:features test:docs test:package
  '';
}
