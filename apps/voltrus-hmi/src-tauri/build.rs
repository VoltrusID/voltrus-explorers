fn main() {
    tauri_build::build();
    #[cfg(target_os = "macos")]
    {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let plist = std::path::Path::new(&dir).join("Info.plist");
        println!("cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
    }
}
