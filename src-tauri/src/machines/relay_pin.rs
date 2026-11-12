//! Your own relay's certificate, pinned (`RelaySetting::cert_sha256`,
//! docs/MACHINES.md "Your own relay"). iroh lets an endpoint bring its own
//! TLS check for every connection that is not a peer's — the relay, its QUIC
//! address discovery, an HTTPS probe (`Builder::ca_tls_config`). With a pin,
//! the relay's host must present exactly that certificate, whoever signed it
//! (a self-signed one too), and still prove it holds its key (the handshake's
//! signature, checked as usual); any other host is checked as iroh checks it
//! by default (Mozilla's roots). Connections between your computers never
//! depend on it: the machine keys authenticate them end to end.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use iroh::tls::CaTlsConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};

use super::store::RelaySetting;

/// What the pinned relay last presented when it was not the pinned
/// certificate: the setup's relay check says so (a renewed certificate
/// needs its new SHA-256).
#[derive(Debug, Default)]
pub struct PinSeen(Mutex<Option<String>>);

impl PinSeen {
    /// The SHA-256 of the certificate the relay presented instead of the
    /// pinned one; `None` until then, and again once it matched.
    pub fn other(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn saw(&self, other: Option<String>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = other;
    }
}

/// The relay's host, as TLS names it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Host {
    Name(String),
    Ip(IpAddr),
}

impl Host {
    fn of(url: &str) -> Option<Host> {
        match url::Url::parse(url).ok()?.host()? {
            url::Host::Domain(d) => Some(Host::Name(d.trim_end_matches('.').to_ascii_lowercase())),
            url::Host::Ipv4(ip) => Some(Host::Ip(IpAddr::V4(ip))),
            url::Host::Ipv6(ip) => Some(Host::Ip(IpAddr::V6(ip))),
        }
    }

    fn is(&self, name: &ServerName<'_>) -> bool {
        match (self, name) {
            (Host::Name(h), ServerName::DnsName(d)) => {
                d.as_ref().trim_end_matches('.').eq_ignore_ascii_case(h)
            }
            (Host::Ip(h), ServerName::IpAddress(ip)) => IpAddr::from(*ip) == *h,
            _ => false,
        }
    }
}

/// The TLS check iroh uses for everything but your computers: `None`
/// without a pin (iroh's default).
pub fn tls_config(relay: &RelaySetting, seen: Arc<PinSeen>) -> Result<Option<CaTlsConfig>, String> {
    let Some(pin) = relay.pin() else {
        return Ok(None);
    };
    let host = relay
        .url
        .as_deref()
        .and_then(Host::of)
        .ok_or("Your relay's address is not valid.")?;
    Ok(Some(CaTlsConfig::custom_server_cert_verifier(Arc::new(
        move |provider| {
            let usual = CaTlsConfig::embedded().server_cert_verifier(provider)?;
            Ok(Arc::new(Pinned {
                host: host.clone(),
                pin: pin.clone(),
                usual,
                seen: seen.clone(),
            }) as Arc<dyn ServerCertVerifier>)
        },
    ))))
}

#[derive(Debug)]
struct Pinned {
    host: Host,
    /// Lowercase hex.
    pin: String,
    /// iroh's own check (Mozilla's roots): other hosts, and every
    /// handshake signature.
    usual: Arc<dyn ServerCertVerifier>,
    seen: Arc<PinSeen>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if !self.host.is(server_name) {
            return self.usual.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            );
        }
        let got = crate::claude::secrets::sha256_hex(end_entity.as_ref());
        if got == self.pin {
            self.seen.saw(None);
            Ok(ServerCertVerified::assertion())
        } else {
            tracing::warn!(presented = %got, "the relay presented another certificate than the pinned one");
            self.seen.saw(Some(got));
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.usual.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.usual.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.usual.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::store::RelayMode;
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

    fn provider() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    /// A relay's self-signed certificate for `name`: (DER, its key).
    fn certificate(name: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let c = rcgen::generate_simple_self_signed(vec![name.to_string()]).unwrap();
        (
            c.cert.der().clone(),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(c.signing_key.serialize_der())),
        )
    }

    fn relay(url: &str, pin: Option<String>) -> RelaySetting {
        RelaySetting {
            mode: RelayMode::Custom,
            url: Some(url.into()),
            cert_sha256: pin,
        }
    }

    /// A whole TLS handshake in memory: the client with `config`'s check,
    /// the server presenting `cert`.
    fn handshake(
        config: &CaTlsConfig,
        name: &str,
        cert: (CertificateDer<'static>, PrivateKeyDer<'static>),
    ) -> Result<(), rustls::Error> {
        let client_config = config.client_config(provider()).unwrap();
        let server_config = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.0], cert.1)
            .unwrap();
        let mut client = rustls::ClientConnection::new(
            Arc::new(client_config),
            ServerName::try_from(name.to_string()).unwrap(),
        )
        .unwrap();
        let mut server = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
        for _ in 0..10 {
            let mut buf = Vec::new();
            while client.wants_write() {
                client.write_tls(&mut buf).unwrap();
            }
            let mut rd = &buf[..];
            while !rd.is_empty() {
                server.read_tls(&mut rd).unwrap();
            }
            server.process_new_packets()?;
            let mut buf = Vec::new();
            while server.wants_write() {
                server.write_tls(&mut buf).unwrap();
            }
            let mut rd = &buf[..];
            while !rd.is_empty() {
                client.read_tls(&mut rd).unwrap();
            }
            client.process_new_packets()?;
            if !client.is_handshaking() && !server.is_handshaking() {
                return Ok(());
            }
        }
        panic!("the handshake did not finish");
    }

    #[test]
    fn without_a_pin_iroh_checks_as_it_does() {
        let seen = Arc::new(PinSeen::default());
        assert!(tls_config(&relay("https://relay.test", None), seen.clone())
            .unwrap()
            .is_none());
        let public = RelaySetting {
            mode: RelayMode::Public,
            ..relay("https://relay.test", Some("ab".repeat(32)))
        };
        assert!(tls_config(&public, seen).unwrap().is_none());
    }

    #[test]
    fn the_pinned_certificate_is_taken_self_signed_and_no_other() {
        let (der, key) = certificate("relay.test");
        let pin = crate::claude::secrets::sha256_hex(der.as_ref());
        let seen = Arc::new(PinSeen::default());
        let config = tls_config(
            &relay("https://relay.test:8443/", Some(pin.to_uppercase())),
            seen.clone(),
        )
        .unwrap()
        .expect("pinned");
        // Self-signed, yet exactly the pinned certificate: taken.
        handshake(&config, "relay.test", (der.clone(), key.clone_key())).expect("pinned one");
        assert_eq!(seen.other(), None);
        // Another certificate for the same name (a man in the middle, or a
        // renewal): refused, and the setup can say which it was.
        let (other, other_key) = certificate("relay.test");
        let e = handshake(&config, "relay.test", (other.clone(), other_key)).unwrap_err();
        assert!(
            matches!(
                e,
                rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
            ),
            "{e:?}"
        );
        assert_eq!(
            seen.other().as_deref(),
            Some(crate::claude::secrets::sha256_hex(other.as_ref()).as_str())
        );
        // The pinned one again: the mismatch is forgotten.
        handshake(&config, "relay.test", (der.clone(), key.clone_key())).unwrap();
        assert_eq!(seen.other(), None);
        // The pin is for the relay's host only: another host is checked as
        // usual, so a self-signed certificate there is refused even when it
        // is the pinned one.
        let (der2, key2) = certificate("elsewhere.test");
        let e = handshake(&config, "elsewhere.test", (der2, key2)).unwrap_err();
        assert!(
            matches!(
                e,
                rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer)
            ),
            "{e:?}"
        );
        assert_eq!(seen.other(), None, "not the relay");
    }

    #[test]
    fn a_relay_named_by_its_address_is_pinned_by_it() {
        let c = rcgen::generate_simple_self_signed(vec!["203.0.113.7".to_string()]).unwrap();
        let der = c.cert.der().clone();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(c.signing_key.serialize_der()));
        let pin = crate::claude::secrets::sha256_hex(der.as_ref());
        let config = tls_config(
            &relay("https://203.0.113.7", Some(pin)),
            Arc::new(PinSeen::default()),
        )
        .unwrap()
        .unwrap();
        handshake(&config, "203.0.113.7", (der, key)).expect("pinned by address");
        assert!(Host::Name("relay.test".into())
            .is(&ServerName::try_from("RELAY.test.".to_string()).unwrap()));
        assert!(!Host::Name("relay.test".into())
            .is(&ServerName::try_from("203.0.113.7".to_string()).unwrap()));
    }
}
