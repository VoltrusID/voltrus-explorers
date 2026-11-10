//! Finding the fx2lafw firmware. The app does **not** ship it (it is GPL-2.0-or-later; see
//! `apps/logic-analyzer/FIRMWARE.md`). It is taken, in order, from a file the user chose, the app's
//! own firmware folder (filled by an explicit "Download from sigrok.org" click), or a sigrok
//! firmware folder installed on the system (a sigrok-firmware-fx2lafw package, or PulseView).
//!
//! Images of the official 0.1.7 release are checked against their published SHA-256; any other
//! image (a newer release, a local build) is accepted with a warning, since it is the user's file.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{LogicError, Result};

pub const RELEASE: &str = "0.1.7";
pub const TARBALL_URL: &str = "https://sigrok.org/download/binary/sigrok-firmware-fx2lafw/sigrok-firmware-fx2lafw-bin-0.1.7.tar.gz";
pub const TARBALL_SHA256: &str = "c876fd075549e7783a6d5bfc8d99a695cfc583ddbcea0217d8e3f9351d1723af";
pub const SOURCE_URL: &str = "https://sigrok.org/download/source/sigrok-firmware-fx2lafw/sigrok-firmware-fx2lafw-0.1.7.tar.gz";
pub const LICENCE: &str = "GPL-2.0-or-later (parts LGPL-2.1-or-later)";

/// SHA-256 of the 8-bit logic analyzer images in the 0.1.7 binary release.
pub const KNOWN: [(&str, &str); 9] = [
    ("fx2lafw-saleae-logic.fw", "dbb9fc37e9cceaa1034f6f68d99d752e0570f449b3a6c1b7dec45df28e614863"),
    ("fx2lafw-cypress-fx2.fw", "db2f52ff5d79b771b0251cc90ba096b20bbb9511c37a88bc3028c89d3458862b"),
    ("fx2lafw-sigrok-fx2-8ch.fw", "b667d878d5455f854bd912704c68cc2cf25702032e72ff825393409890a86e37"),
    ("fx2lafw-sigrok-fx2-16ch.fw", "3415094905e9d37a59a1c91aaa0fd7697f8246178e08ca9a7957f2b60305b68c"),
    ("fx2lafw-cwav-usbeeax.fw", "b96fcce1da66f0b96a1d02ecb94c72ca976f92410d205207b2d200be7fa1d6ed"),
    ("fx2lafw-cwav-usbeedx.fw", "83a4417dd83700aebcc68295a4fa2fb2b7e9811dff64abdb0dd3d72a1b69d677"),
    ("fx2lafw-cwav-usbeesx.fw", "50cf095eb3024e60f4667553cce9a7398f9c0de842a312aecd5c9aac83eeb7fa"),
    ("fx2lafw-cwav-usbeezx.fw", "23f34be2dc77fefd604829358335c0b78f013327d73acb10f836c386e6ef602d"),
    ("fx2lafw-braintechnology-usb-lps.fw", "65fb22d459b1c2833471aaf2954fded442393bc59ddba7c7e97198c09485eb52"),
];

/// The FX2LP has 16 KB of program RAM.
pub const MAX_IMAGE: usize = 16 * 1024;

pub fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

pub fn check_image(b: &[u8]) -> Result<()> {
    if b.is_empty() || b.len() > MAX_IMAGE {
        return Err(LogicError::Firmware(format!("{} bytes is not an FX2 firmware image (expected up to 16 KB)", b.len())));
    }
    // Intel HEX images (`.ihx`) are text; fx2lafw's `.fw` files are raw binary.
    if b.starts_with(b":") && b.iter().take(64).all(|c| c.is_ascii()) {
        return Err(LogicError::Firmware("this is an Intel HEX file; choose the .fw file from the fx2lafw release".into()));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verified {
    /// Byte for byte the official 0.1.7 image.
    Official,
    /// Not a known image: used as the user's own file.
    Unknown,
}

pub fn verify(name: &str, b: &[u8]) -> Verified {
    let h = sha256_hex(b);
    if KNOWN.iter().any(|(n, k)| *n == name && *k == h) { Verified::Official } else { Verified::Unknown }
}

pub trait FirmwareSource: Send + Sync {
    fn load(&self, name: &str) -> Result<Vec<u8>>;
}

/// Firmware images in memory (tests).
pub struct MemoryFirmware(Vec<(String, Vec<u8>)>);

impl MemoryFirmware {
    pub fn new(files: &[(&str, Vec<u8>)]) -> Self {
        Self(files.iter().map(|(n, d)| (n.to_string(), d.clone())).collect())
    }
}

impl FirmwareSource for MemoryFirmware {
    fn load(&self, name: &str) -> Result<Vec<u8>> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, d)| d.clone()).ok_or_else(|| missing(name))
    }
}

fn missing(name: &str) -> LogicError {
    LogicError::Firmware(format!(
        "the fx2lafw firmware ({name}) is not on this computer. Download it from sigrok.org in the app (Device, Firmware), install sigrok-firmware-fx2lafw, or choose the file yourself."
    ))
}

/// Where firmware is looked for.
#[derive(Debug, Clone, Default)]
pub struct FirmwareDirs {
    /// A file the user picked: used for every board (it must match the board).
    pub user_file: Option<PathBuf>,
    /// The app's firmware folder (downloads go here).
    pub app_dir: Option<PathBuf>,
    pub system_dirs: Vec<PathBuf>,
}

/// Standard places sigrok firmware gets installed.
pub fn system_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = ["/usr/share/sigrok-firmware", "/usr/local/share/sigrok-firmware", "/opt/homebrew/share/sigrok-firmware", "/opt/local/share/sigrok-firmware"]
        .iter()
        .map(PathBuf::from)
        .collect();
    if let Some(home) = std::env::var_os("HOME") {
        v.push(Path::new(&home).join(".local/share/sigrok-firmware"));
    }
    #[cfg(target_os = "macos")]
    v.push(PathBuf::from("/Applications/PulseView.app/Contents/share/sigrok-firmware"));
    #[cfg(target_os = "windows")]
    v.push(PathBuf::from(r"C:\Program Files\sigrok\PulseView\share\sigrok-firmware"));
    v
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundFirmware {
    pub name: String,
    pub path: String,
    pub verified: Verified,
    pub source: &'static str,
}

impl FirmwareDirs {
    pub fn find(&self, name: &str) -> Option<FoundFirmware> {
        if let Some(f) = &self.user_file
            && let Ok(b) = std::fs::read(f)
            && check_image(&b).is_ok()
        {
            return Some(FoundFirmware { name: name.into(), path: f.display().to_string(), verified: verify(name, &b), source: "chosen file" });
        }
        let dirs = self.app_dir.iter().map(|d| (d.clone(), "app folder")).chain(self.system_dirs.iter().map(|d| (d.clone(), "installed sigrok firmware")));
        for (d, source) in dirs {
            let p = d.join(name);
            if let Ok(b) = std::fs::read(&p)
                && check_image(&b).is_ok()
            {
                return Some(FoundFirmware { name: name.into(), path: p.display().to_string(), verified: verify(name, &b), source });
            }
        }
        None
    }
}

impl FirmwareSource for FirmwareDirs {
    fn load(&self, name: &str) -> Result<Vec<u8>> {
        let f = self.find(name).ok_or_else(|| missing(name))?;
        let b = std::fs::read(&f.path)?;
        check_image(&b)?;
        Ok(b)
    }
}

/// Unpacks the official binary tarball into `dir`: the `.fw` images (each checked against
/// [`KNOWN`]) plus its README and licence files, which go with them.
pub fn install_tarball(tar_gz: &[u8], dir: &Path) -> Result<Vec<String>> {
    if sha256_hex(tar_gz) != TARBALL_SHA256 {
        return Err(LogicError::Firmware("the downloaded file is not the fx2lafw 0.1.7 release (checksum mismatch); nothing was installed".into()));
    }
    unpack(tar_gz, dir)
}

#[cfg(feature = "download")]
fn unpack(tar_gz: &[u8], dir: &Path) -> Result<Vec<String>> {
    use std::io::Read;
    std::fs::create_dir_all(dir)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(tar_gz));
    let mut installed = Vec::new();
    for entry in ar.entries()? {
        let mut e = entry?;
        let path = e.path()?.into_owned();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        let keep = name.ends_with(".fw") || matches!(name.as_str(), "COPYING" | "COPYING.LESSER" | "README" | "NEWS");
        if !keep {
            continue;
        }
        let mut b = Vec::new();
        e.read_to_end(&mut b)?;
        if name.ends_with(".fw") {
            if let Some((_, h)) = KNOWN.iter().find(|(n, _)| *n == name)
                && sha256_hex(&b) != *h
            {
                return Err(LogicError::Firmware(format!("{name} in the download does not match the release; nothing more was installed")));
            }
            installed.push(name.clone());
        }
        std::fs::write(dir.join(&name), &b)?;
    }
    std::fs::write(
        dir.join("SOURCE.txt"),
        format!("fx2lafw {RELEASE}, {LICENCE}.\nBinary release: {TARBALL_URL}\nCorresponding source: {SOURCE_URL}\nDownloaded by Voltrus Logic Analyzer at the user's request.\n"),
    )?;
    installed.sort();
    Ok(installed)
}

#[cfg(not(feature = "download"))]
fn unpack(_: &[u8], _: &Path) -> Result<Vec<String>> {
    Err(LogicError::Firmware("this build cannot unpack the firmware download".into()))
}

/// Fetches the official release from sigrok.org and installs it into `dir`. Only called when
/// the user asks for it.
#[cfg(feature = "download")]
pub async fn download(dir: &Path) -> Result<Vec<String>> {
    let resp = reqwest::get(TARBALL_URL).await.map_err(|e| LogicError::Firmware(format!("cannot reach sigrok.org: {e}")))?;
    if !resp.status().is_success() {
        return Err(LogicError::Firmware(format!("sigrok.org answered {}", resp.status())));
    }
    let body = resp.bytes().await.map_err(|e| LogicError::Firmware(format!("download failed: {e}")))?;
    install_tarball(&body, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_images() {
        assert!(check_image(&[]).is_err());
        assert!(check_image(&vec![0; MAX_IMAGE + 1]).is_err());
        assert!(check_image(b":10000000021234\n").is_err());
        assert!(check_image(&[0x02, 0x10, 0x00]).is_ok());
        assert_eq!(verify("fx2lafw-saleae-logic.fw", &[1, 2, 3]), Verified::Unknown);
    }

    #[test]
    fn rejects_a_wrong_tarball() {
        let dir = std::env::temp_dir().join(format!("mt-logic-fw-{}", std::process::id()));
        assert!(install_tarball(b"not the release", &dir).is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn search_order() {
        let base = std::env::temp_dir().join(format!("mt-logic-fw-order-{}", std::process::id()));
        let (app, sys) = (base.join("app"), base.join("sys"));
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(sys.join("fx2lafw-saleae-logic.fw"), [2, 0, 0]).unwrap();
        let dirs = FirmwareDirs { user_file: None, app_dir: Some(app.clone()), system_dirs: vec![sys.clone()] };
        assert_eq!(dirs.find("fx2lafw-saleae-logic.fw").unwrap().source, "installed sigrok firmware");
        std::fs::write(app.join("fx2lafw-saleae-logic.fw"), [2, 0, 1]).unwrap();
        assert_eq!(dirs.load("fx2lafw-saleae-logic.fw").unwrap(), vec![2, 0, 1]);
        let user = base.join("mine.fw");
        std::fs::write(&user, [9, 9]).unwrap();
        let dirs = FirmwareDirs { user_file: Some(user), ..dirs };
        assert_eq!(dirs.load("fx2lafw-saleae-logic.fw").unwrap(), vec![9, 9]);
        assert!(dirs.find("x.fw").is_some(), "a chosen file is used for any board");
        let none = FirmwareDirs::default();
        assert!(none.load("fx2lafw-saleae-logic.fw").unwrap_err().to_string().contains("sigrok.org"));
        std::fs::remove_dir_all(base).unwrap();
    }

    /// The real release, when it is in /tmp (fetched while writing this driver). Skipped otherwise.
    #[test]
    fn installs_the_real_release_if_present() {
        let p = Path::new("/tmp/fx2dl/sigrok-firmware-fx2lafw-bin-0.1.7.tar.gz");
        let Ok(b) = std::fs::read(p) else { return };
        let dir = std::env::temp_dir().join(format!("mt-logic-fw-real-{}", std::process::id()));
        let got = install_tarball(&b, &dir).unwrap();
        assert!(got.contains(&"fx2lafw-saleae-logic.fw".to_string()));
        let img = std::fs::read(dir.join("fx2lafw-saleae-logic.fw")).unwrap();
        assert_eq!(verify("fx2lafw-saleae-logic.fw", &img), Verified::Official);
        assert!(dir.join("COPYING").exists() && dir.join("SOURCE.txt").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
