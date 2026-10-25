//! Embeds `assets/{fonts,icons,themes}` into the binary as a table sorted by
//! path, so lookups can binary-search it.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let output = env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is missing"))?;
    let mut source = String::from("static ASSETS: &[(&str, &[u8])] = &[\n");
    // Directory order matches the byte order of the keys: the table stays sorted.
    for directory in ["fonts", "icons", "themes"] {
        let mut names = Vec::new();
        for entry in fs::read_dir(format!("assets/{directory}"))? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| io::Error::other("non-UTF-8 asset name"))?
                .to_owned();
            if entry.file_type()?.is_file() && !name.starts_with('.') && !name.ends_with(".txt") {
                names.push(name);
            }
        }
        names.sort();
        for name in names {
            let key = format!("{directory}/{name}");
            let path = format!("/assets/{key}");
            writeln!(
                source,
                "({key:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {path:?}))),"
            )
            .map_err(io::Error::other)?;
        }
    }
    source.push_str("];\n");
    fs::write(PathBuf::from(output).join("assets.rs"), source)?;
    writeln!(io::stdout(), "cargo:rerun-if-changed=assets")
}
