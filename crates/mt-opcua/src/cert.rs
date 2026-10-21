use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::types::CertInfo;

/// Infer "client" vs "server" from the PKI directory name.
fn kind_of(pki: &Path) -> String {
    let s = pki
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if s.contains("server") {
        "server".to_string()
    } else {
        "client".to_string()
    }
}

/// Normalize a hex thumbprint for comparison (lowercase, digits only).
fn norm_hex(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_hexdigit()).flat_map(|c| c.to_lowercase()).collect()
}

fn parse_cert(path: &Path, bucket: &str, kind: &str) -> Option<CertInfo> {
    let der = fs::read(path).ok()?;
    let cert = opcua::crypto::X509::from_der(&der).ok()?;
    let thumbprint = cert.thumbprint().as_hex_string();
    let subject = cert.subject_name();
    // OPC UA app certs are almost always self-signed; issuer == subject for own certs.
    let issuer = subject.clone();
    let valid_from = cert.not_before().map(|d| d.to_rfc3339()).unwrap_or_default();
    let valid_to = cert
        .not_after()
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|_| String::new());
    Some(CertInfo {
        thumbprint: norm_hex(&thumbprint),
        subject,
        issuer,
        valid_from,
        valid_to,
        bucket: bucket.to_string(),
        kind: kind.to_string(),
        is_ca: false,
        path: path.to_string_lossy().to_string(),
    })
}

fn scan_bucket(out: &mut Vec<CertInfo>, pki: &Path, sub: &[&str], bucket: &str) {
    let kind = kind_of(pki);
    let mut dir = pki.to_path_buf();
    for s in sub {
        dir.push(s);
    }
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("der") {
                if let Some(info) = parse_cert(&p, bucket, &kind) {
                    out.push(info);
                }
            }
        }
    }
}

/// List own / trusted / rejected certificates across the given PKI directories.
pub fn cert_list(pkis: &[PathBuf]) -> Vec<CertInfo> {
    let mut out = Vec::new();
    for pki in pkis {
        // Own certificate.
        let own = pki.join("own").join("cert.der");
        if own.exists() {
            let kind = kind_of(pki);
            if let Some(info) = parse_cert(&own, "own", &kind) {
                out.push(info);
            }
        }
        scan_bucket(&mut out, pki, &["trusted", "certs"], "trusted");
        scan_bucket(&mut out, pki, &["rejected", "certs"], "rejected");
    }
    out
}

fn find_cert_by_thumbprint<'a>(
    pkis: &'a [PathBuf],
    bucket_parts: &[&str],
    thumbprint: &str,
) -> Option<(PathBuf, PathBuf)> {
    let needle = norm_hex(thumbprint);
    for pki in pkis {
        let mut dir = pki.clone();
        for s in bucket_parts {
            dir.push(s);
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) != Some("der") {
                    continue;
                }
                if let Ok(der) = fs::read(&p) {
                    if let Ok(cert) = opcua::crypto::X509::from_der(&der) {
                        if norm_hex(&cert.thumbprint().as_hex_string()) == needle {
                            return Some((p, dir));
                        }
                    }
                }
            }
        }
    }
    None
}

/// Move a trusted certificate to the rejected list.
pub fn cert_reject(pkis: &[PathBuf], thumbprint: &str) -> Result<()> {
    let (src, trusted_dir) =
        find_cert_by_thumbprint(pkis, &["trusted", "certs"], thumbprint)
            .ok_or_else(|| anyhow!("Trusted certificate not found for thumbprint {thumbprint}"))?;
    let pki_root = trusted_dir
        .ancestors()
        .nth(2)
        .ok_or_else(|| anyhow!("Cannot resolve PKI root"))?
        .to_path_buf();
    let rejected_dir = pki_root.join("rejected").join("certs");
    fs::create_dir_all(&rejected_dir)?;
    let dest = rejected_dir.join(src.file_name().unwrap());
    fs::rename(&src, &dest)?;
    Ok(())
}

/// Move a rejected certificate to the trusted list.
pub fn cert_trust(pkis: &[PathBuf], thumbprint: &str) -> Result<()> {
    let (src, rejected_dir) =
        find_cert_by_thumbprint(pkis, &["rejected", "certs"], thumbprint)
            .ok_or_else(|| anyhow!("Rejected certificate not found for thumbprint {thumbprint}"))?;
    let pki_root = rejected_dir
        .ancestors()
        .nth(2)
        .ok_or_else(|| anyhow!("Cannot resolve PKI root"))?
        .to_path_buf();
    let trusted_dir = pki_root.join("trusted").join("certs");
    fs::create_dir_all(&trusted_dir)?;
    let dest = trusted_dir.join(src.file_name().unwrap());
    fs::rename(&src, &dest)?;
    Ok(())
}

/// Remove the own certificate + private key so the SDK regenerates them on next start.
pub fn cert_generate(pkis: &[PathBuf]) -> Result<()> {
    let mut removed = 0u32;
    for pki in pkis {
        let cert = pki.join("own").join("cert.der");
        let key = pki.join("private").join("private.pem");
        for f in [cert, key] {
            if f.exists() {
                fs::remove_file(&f)?;
                removed += 1;
            }
        }
    }
    if removed == 0 {
        return Err(anyhow!("No own certificate/key found to regenerate"));
    }
    Ok(())
}