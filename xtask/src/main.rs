//! Build, bundle and launch the `borderless` example as a real `.app`.
//!
//! This replaces the `run.sh` the project carried over from its Swift life. It
//! is not a convenience wrapper: it is the thing that decides whether a
//! measurement is allowed to happen, and every guard in it exists because its
//! absence already produced a false reading.
//!
//! ```text
//! cargo xtask run [tag]     build, verify, bundle, launch
//! cargo xtask bundle        build and bundle, without launching
//! ```
//!
//! # Why a bundle at all
//!
//! An unbundled, background-launched process cannot activate on macOS 14+, so
//! its window never becomes key — and `NSGlassEffectView` renders *differently*
//! when its window is not key. Every capture of an unbundled build silently
//! showed a non-key window. Launching a real `.app` through `open` is what
//! fixes that, which is why this bundles rather than just running the binary.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::SystemTime;

const EXAMPLE: &str = "borderless";
const BUNDLE_ID: &str = "com.logger.macosliquidglass.borderless";
/// The binary is stamped for macOS 27 and LaunchServices refuses a bundle whose
/// declared floor it cannot honour. Note this is the *app's* floor, not the
/// library's — `macos-liquid-glass` itself needs only macOS 26 at runtime.
const MIN_SYSTEM_VERSION: &str = "27.0";

/// The knobs forwarded into the launched app.
///
/// `open` does not inherit the caller's environment, so anything the harness
/// sets has to be passed explicitly. A knob missing from this list is a knob
/// that silently does nothing when swept — the failure mode R19 records.
const KNOBS: &[&str] = &[
    "GN_ORIGIN",
    "GN_GLASS",
    "GN_SHADOW",
    "GN_TINT_SPACE",
    "GN_TINT",
    "GN_PAPER_L",
    "GN_PAPER_D",
    "GN_GTINT",
    "GN_GT_D",
    "GN_GT_L",
    "GN_HUE",
    "GN_HSAT",
    "GN_TBODY",
    "GN_POLL",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, tag) = match args.split_first() {
        Some((c, rest)) => (c.as_str(), rest.first().cloned()),
        None => ("run", None),
    };

    let root = repo_root();
    let result = match cmd {
        "run" => bundle(&root).and_then(|app| launch(&app, tag.as_deref().unwrap_or("run"))),
        "bundle" => bundle(&root).map(|_| ()),
        other => Err(format!(
            "unknown task {other:?}; expected `run` or `bundle`"
        )),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn repo_root() -> PathBuf {
    // The xtask crate lives at <root>/xtask.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is a workspace member")
        .to_path_buf()
}

/// Build the example, verify the result, and assemble the `.app`.
fn bundle(root: &Path) -> Result<PathBuf, String> {
    let status = Command::new(std::env::var("CARGO").as_deref().unwrap_or("cargo"))
        .current_dir(root)
        .args(["build", "--release", "--example", EXAMPLE])
        .status()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if !status.success() {
        return Err("BUILD FAILED — nothing bundled".into());
    }

    let bin = root.join("target/release/examples").join(EXAMPLE);

    // A failed build leaves the previous binary in place and it runs happily.
    // Checked separately from staleness because the staleness comparison needs
    // this file to exist as its reference point — when it does not, the
    // comparison silently answers "nothing is newer", which reads as "fresh".
    if !bin.is_file() {
        return Err(format!(
            "BINARY MISSING — cargo reported success but produced nothing at {}",
            bin.display()
        ));
    }

    let sources = [
        root.join("src"),
        root.join("examples"),
        root.join("Cargo.toml"),
    ];
    if let Some(newer) = newer_than(&bin, &sources)? {
        return Err(format!(
            "BINARY STALE — {} is newer than the build; refusing",
            newer.display()
        ));
    }

    let app = root.join("build").join(format!("{EXAMPLE}.app"));
    let macos = app.join("Contents/MacOS");
    fs::create_dir_all(&macos).map_err(|e| format!("could not create {}: {e}", macos.display()))?;
    fs::copy(&bin, macos.join(EXAMPLE)).map_err(|e| format!("could not copy binary: {e}"))?;
    fs::write(app.join("Contents/Info.plist"), info_plist())
        .map_err(|e| format!("could not write Info.plist: {e}"))?;

    // Ad-hoc signature. Failure is not fatal — an unsigned bundle still
    // launches — so this reports rather than refuses.
    let signed = Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&app)
        .output();
    match signed {
        Ok(o) if !o.status.success() => {
            eprintln!("xtask: codesign failed (continuing unsigned)");
        }
        Err(e) => eprintln!("xtask: could not run codesign ({e}); continuing unsigned"),
        Ok(_) => {}
    }

    Ok(app)
}

/// The first file under `roots` that is newer than `reference`, if any.
///
/// Regular files only. Directory mtimes change whenever a file is added to or
/// removed from them, so including directories made an unrelated scratch file
/// mark a perfectly fresh binary stale.
fn newer_than(reference: &Path, roots: &[PathBuf]) -> Result<Option<PathBuf>, String> {
    let cutoff = mtime(reference)?;
    let mut stack: Vec<PathBuf> = roots.iter().filter(|p| p.exists()).cloned().collect();

    while let Some(path) = stack.pop() {
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            let entries = fs::read_dir(&path)
                .map_err(|e| format!("could not read {}: {e}", path.display()))?;
            for entry in entries.flatten() {
                stack.push(entry.path());
            }
        } else if meta.is_file()
            && matches!(
                path.extension().and_then(OsStr::to_str),
                Some("rs" | "toml")
            )
            && meta.modified().map(|m| m > cutoff).unwrap_or(false)
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn mtime(path: &Path) -> Result<SystemTime, String> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(|e| format!("could not stat {}: {e}", path.display()))
}

fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>{EXAMPLE}</string>
  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
  <key>CFBundleName</key><string>{EXAMPLE}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>LSMinimumSystemVersion</key><string>{MIN_SYSTEM_VERSION}</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
"#
    )
}

/// Replace any running instance and launch the bundle.
fn launch(app: &Path, tag: &str) -> Result<(), String> {
    let _ = Command::new("pkill")
        .arg("-f")
        .arg(format!("{EXAMPLE}.app/Contents/MacOS"))
        .status();

    // /tmp explicitly, NOT std::env::temp_dir(). On macOS that returns the
    // per-user $TMPDIR (/var/folders/...), which is per-user and per-session
    // and so is not a stable path for anything reading the log from outside
    // this process.
    let log = PathBuf::from("/tmp").join(format!("{EXAMPLE}_{tag}.log"));
    let _ = fs::write(&log, b"");

    let mut env: BTreeMap<&str, String> = BTreeMap::new();
    // Self-terminate by default, so a sweep cannot leave a drift of orphaned
    // windows behind.
    env.insert(
        "GN_SECS",
        std::env::var("GN_SECS").unwrap_or_else(|_| "600".into()),
    );
    for knob in KNOBS {
        if let Ok(v) = std::env::var(knob)
            && !v.is_empty()
        {
            env.insert(knob, v);
        }
    }

    let mut cmd = Command::new("open");
    cmd.arg("-n").arg("--stderr").arg(&log);
    for (k, v) in &env {
        cmd.arg("--env").arg(format!("{k}={v}"));
    }
    cmd.arg(app);

    let status = cmd
        .status()
        .map_err(|e| format!("could not run open: {e}"))?;
    if !status.success() {
        return Err(format!("open failed with {status}"));
    }

    println!("launched; log={}", log.display());
    Ok(())
}
