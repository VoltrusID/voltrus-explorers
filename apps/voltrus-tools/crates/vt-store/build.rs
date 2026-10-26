//! Store builds for macOS: compile the StoreKit 2 bridge
//! (`swift/StoreKitBridge.swift`) into a static library for the target
//! architecture and link it against the Swift runtime in macOS
//! (`/usr/lib/swift`). Other builds compile nothing.

use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

/// `LSMinimumSystemVersion` of the app: StoreKit 2 needs macOS 12, and from
/// 13 on the Swift concurrency runtime needs no back-deployment library.
const MACOS_MIN: &str = "13.0";
const BRIDGE: &str = "swift/StoreKitBridge.swift";

fn main() -> io::Result<()> {
    let mut out = io::stdout();
    writeln!(out, "cargo:rerun-if-changed={BRIDGE}")?;
    if env::var_os("CARGO_FEATURE_STORE").is_none()
        || env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
    {
        return Ok(());
    }
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => {
            return Err(io::Error::other(format!(
                "no StoreKit bridge for architecture {other:?}"
            )));
        }
    };
    let target = format!("{arch}-apple-macos{MACOS_MIN}");
    let out_dir = PathBuf::from(
        env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is missing"))?,
    );

    run(Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "swiftc",
            "-parse-as-library",
            "-emit-library",
        ])
        .args([
            "-static",
            "-module-name",
            "VtStoreKit",
            "-swift-version",
            "5",
            "-O",
        ])
        .args(["-target", &target, "-o"])
        .arg(out_dir.join("libvtstorekit.a"))
        .arg(BRIDGE))?;

    // The bridge's objects ask the linker for the Swift runtime libraries
    // (-lswiftCore, -lswift_Concurrency, ...): point it at the SDK's stubs and
    // the toolchain's static helpers.
    let info = run(Command::new("xcrun").args([
        "--sdk",
        "macosx",
        "swiftc",
        "-print-target-info",
        "-target",
        &target,
    ]))?;
    let info: serde_json::Value = serde_json::from_str(&info).map_err(io::Error::other)?;
    let sdk = run(Command::new("xcrun").args(["--sdk", "macosx", "--show-sdk-path"]))?;
    let mut search = vec![format!("{}/usr/lib/swift", sdk.trim())];
    for key in ["runtimeLibraryPaths", "runtimeLibraryImportPaths"] {
        search.extend(
            info["paths"][key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|path| path.as_str().map(str::to_owned)),
        );
    }
    search.push(out_dir.display().to_string());
    for path in search {
        writeln!(out, "cargo:rustc-link-search=native={path}")?;
    }
    writeln!(out, "cargo:rustc-link-lib=static=vtstorekit")?;
    writeln!(out, "cargo:rustc-link-lib=framework=StoreKit")?;
    writeln!(out, "cargo:rustc-link-lib=framework=Foundation")
}

/// Run a tool and return its standard output; a failure fails the build.
fn run(command: &mut Command) -> io::Result<String> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{command:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}
