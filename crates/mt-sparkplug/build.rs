use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/sparkplug_b.proto");

    prost_build::Config::new()
        .compile_protos(&["proto/sparkplug_b.proto"], &["proto"])
        .map_err(|e| {
            if protoc_missing() {
                format!(
                    "protoc was not found: {e}. Install the protobuf compiler or point \
                     PROTOC at it (the pinned dev toolchain keeps it at /usr/bin/protoc)."
                )
            } else {
                e.to_string()
            }
        })?;
    Ok(())
}

/// True when prost-build has no way to find a protoc binary.
fn protoc_missing() -> bool {
    if env::var_os("PROTOC").is_some() {
        return false;
    }
    let Some(paths) = env::var_os("PATH") else {
        return true;
    };
    !env::split_paths(&paths).any(|dir| dir.join("protoc").is_file())
}
