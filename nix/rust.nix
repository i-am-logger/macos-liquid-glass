_:

{
  # Rust language configuration
  languages.rust = {
    enable = true;
    # https://devenv.sh/reference/options/#languagesrustchannel
    channel = "stable";

    components = [
      "rustc"
      "cargo"
      "clippy"
      "rustfmt"
      "rust-analyzer"
    ];

    # Warnings are errors for every cargo invocation in the shell, matching CI.
    # devenv folds this into the RUSTFLAGS env var (languages.rust.rustflags).
    # Cargo passes `--cap-lints allow` to registry/git dependencies, so only
    # this project's own crates are held to it — a warning in a third-party
    # dependency never breaks the build.
    rustflags = "-D warnings";

    # Add cross-compilation targets here, e.g.
    #   targets = [ "wasm32-unknown-unknown" ];
    # They are fetched by rust-overlay alongside the toolchain, so a
    # `cargo build --target <triple>` in CI needs no extra setup step.
  };
}
