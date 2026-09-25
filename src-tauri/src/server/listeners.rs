//! The LAN TLS listeners (phones at home).
//!
//! Owner: **B1 (security/remote)**. Moved here verbatim from `server/mod.rs` by
//! the foundation so B1 can change binding without touching `mod.rs`. What B1
//! changes (design §2.8): bind ALL non-loopback IPv4 addresses (not just the
//! 8.8.8.8-route one) and regenerate the self-signed cert when its SANs don't
//! cover the current IPs.
//!
//! Every listener serves `super::router_for(state, Ingress::Lan)` with
//! connect info, so the context middleware sees the real TCP peer.

use std::net::SocketAddr;

use super::context::Ingress;
use super::tls;
use crate::state::AppState;

/// Start the TLS listeners on the LAN addresses when the configured host is
/// non-loopback. Returns once they are spawned.
pub async fn start_lan(state: &AppState) {
    if !state.config.is_non_loopback() {
        return;
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cert_dir = state
        .auth
        .config_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("mother-claude"))
        .join("certs");
    let bundle = match tls::ensure_cert(&cert_dir) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "could not prepare TLS cert");
            return;
        }
    };
    *state.fingerprint.write().await = Some(bundle.fingerprint.clone());
    let tls_config = match axum_server::tls_rustls::RustlsConfig::from_pem(
        bundle.cert_pem.into_bytes(),
        bundle.key_pem.into_bytes(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "could not build TLS config");
            return;
        }
    };
    let port = state.config.port;
    for ip in tls::local_ips() {
        let addr = SocketAddr::new(ip, port);
        let app = super::router_for(state.clone(), Ingress::Lan);
        let cfg = tls_config.clone();
        tokio::spawn(async move {
            if let Err(e) = axum_server::bind_rustls(addr, cfg)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
            {
                tracing::error!(%addr, error = %e, "TLS server stopped");
            }
        });
    }
}
