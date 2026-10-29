//! Compiles `swift/AppTransaction.swift` (StoreKit 2 `AppTransaction`) into a static library for App Store builds
//! on Apple targets. Everything else builds without Swift.

use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=swift/AppTransaction.swift");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_STORE").is_none() {
        return;
    }
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os != "macos" && os != "ios" {
        return;
    }
    let target = env::var("TARGET").expect("TARGET");
    let arch = if target.starts_with("aarch64") { "arm64" } else { "x86_64" };
    let (sdk_name, triple) = if os == "macos" {
        let min = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "13.0".into());
        ("macosx", format!("{arch}-apple-macosx{min}"))
    } else {
        let min = env::var("IPHONEOS_DEPLOYMENT_TARGET").unwrap_or_else(|_| "16.0".into());
        if target.ends_with("-sim") || target.starts_with("x86_64") {
            ("iphonesimulator", format!("{arch}-apple-ios{min}-simulator"))
        } else {
            ("iphoneos", format!("{arch}-apple-ios{min}"))
        }
    };
    let sdk = run("xcrun", &["--sdk", sdk_name, "--show-sdk-path"]);
    let sdk = sdk.trim();
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let lib = out.join("libmt_store_swift.a");
    let status = Command::new("xcrun")
        .args(["--sdk", sdk_name, "swiftc", "-target", &triple, "-sdk", sdk])
        .args(["-parse-as-library", "-swift-version", "5", "-O", "-module-name", "MtStoreSwift"])
        .args(["-emit-library", "-static", "-o"])
        .arg(&lib)
        .arg("swift/AppTransaction.swift")
        .status()
        .expect("failed to run xcrun swiftc");
    assert!(status.success(), "swiftc failed for {triple}");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo:rustc-link-lib=static=mt_store_swift");
    println!("cargo:rustc-link-lib=framework=StoreKit");
}

fn run(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().unwrap_or_else(|e| panic!("{program}: {e}"));
    assert!(out.status.success(), "{program} {args:?} failed");
    String::from_utf8(out.stdout).expect("utf-8 output")
}
