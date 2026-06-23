//! Self-signed TLS for non-loopback binds.
//!
//! On first run a certificate is generated with [`rcgen`] (SANs: `localhost`,
//! loopback, and the detected LAN IPs) and persisted under the app config dir.
//! The SHA-256 fingerprint is shown in the UI so a phone can verify it. We use
//! the `ring` rustls provider to avoid the aws-lc C toolchain dependency.

use std::net::{IpAddr, UdpSocket};
use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// A PEM cert/key pair plus its fingerprint.
pub struct CertBundle {
    pub cert_pem: String,
    pub key_pem: String,
    pub fingerprint: String,
}

/// Best-effort discovery of this machine's primary LAN IP via the UDP-connect
/// trick (no packets are actually sent).
pub fn local_ips() -> Vec<IpAddr> {
    let mut ips = Vec::new();
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(local) = sock.local_addr() {
                ips.push(local.ip());
            }
        }
    }
    ips
}

fn sha256_fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Load the persisted certificate, or generate and persist a new one.
pub fn ensure_cert(dir: &Path) -> Result<CertBundle> {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    let fp_path = dir.join("fingerprint.txt");

    if cert_path.is_file() && key_path.is_file() {
        let cert_pem = std::fs::read_to_string(&cert_path)?;
        let key_pem = std::fs::read_to_string(&key_path)?;
        let fingerprint = std::fs::read_to_string(&fp_path)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        return Ok(CertBundle {
            cert_pem,
            key_pem,
            fingerprint,
        });
    }

    std::fs::create_dir_all(dir).context("create cert dir")?;

    let mut sans = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    for ip in local_ips() {
        sans.push(ip.to_string());
    }
    sans.sort();
    sans.dedup();

    let certified =
        rcgen::generate_simple_self_signed(sans).context("generate self-signed cert")?;
    let cert_pem = certified.cert.pem();
    let key_pem = certified.key_pair.serialize_pem();
    let fingerprint = sha256_fingerprint(certified.cert.der());

    std::fs::write(&cert_path, &cert_pem)?;
    std::fs::write(&key_path, &key_pem)?;
    let _ = std::fs::write(&fp_path, &fingerprint);
    restrict_permissions(&key_path);

    Ok(CertBundle {
        cert_pem,
        key_pem,
        fingerprint,
    })
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_reloads_cert() {
        let dir = tempfile::tempdir().unwrap();
        let a = ensure_cert(dir.path()).unwrap();
        assert!(a.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(a.key_pem.contains("PRIVATE KEY"));
        assert!(a.fingerprint.contains(':'));

        // Second call loads the persisted cert (same fingerprint).
        let b = ensure_cert(dir.path()).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.cert_pem, b.cert_pem);
    }
}
