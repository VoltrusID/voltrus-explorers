fn main() {
    // Modbus RTU over serial ports exists on macOS and Windows; the virtual (pty) port only on macOS (cfg target_os).
    println!("cargo::rustc-check-cfg=cfg(serial)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "macos" || os == "windows" {
        println!("cargo::rustc-cfg=serial");
    }
    tauri_build::build();
}
