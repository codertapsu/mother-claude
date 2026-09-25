//! The LAN TLS listeners (phones at home).
//!
//! * Bound only on **private** IPv4 interface addresses (RFC 1918 and
//!   169.254/16, plus Tailscale's 100.64/10 when "Also listen on Tailscale" is
//!   on) — never on a public IPv4 or any IPv6 address. Skipped interfaces are
//!   logged and shown on the Remote page.
//! * Every 30 s the address set is re-read: new addresses get a listener,
//!   vanished ones are shut down, and the leaf certificate is re-issued (and
//!   hot-reloaded into every listener) when its SANs no longer cover them
//!   (`tls::ensure_cert`).
//! * `MOTHER_CLAUDE_HOST` set to one specific private address binds only that
//!   one; a loopback host disables the LAN listeners.
//!
//! Every listener serves `super::router_for(state, Ingress::Lan)` with
//! connect info, so the context middleware sees the real TCP peer.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum_server::tls_rustls::RustlsConfig;

use super::context::Ingress;
use super::remote::{LanAddr, LanStatus, SkippedAddr};
use super::tls;
use crate::state::AppState;

/// How often the interface list is re-read.
const REFRESH_EVERY: Duration = Duration::from_secs(30);

/// Listener handles and the shared TLS config (held in `RemoteState`).
#[derive(Default)]
pub struct LanRuntime {
    config: Option<RustlsConfig>,
    bound: HashMap<IpAddr, axum_server::Handle<SocketAddr>>,
    started: bool,
}

impl std::fmt::Debug for LanRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanRuntime")
            .field("bound", &self.bound.keys().collect::<Vec<_>>())
            .finish()
    }
}

fn cert_dir(state: &AppState) -> std::path::PathBuf {
    state
        .auth
        .config_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("mother-claude"))
        .join("certs")
}

/// The addresses to bind now, and the ones skipped.
fn desired(state: &AppState) -> (Vec<LanAddr>, Vec<SkippedAddr>) {
    let port = state.config.port;
    let allow_cgnat = state.remote.settings().bind_tailscale;
    let host = state.config.host.trim();
    let wildcard = host == "0.0.0.0" || host == "::" || host == "[::]";
    if wildcard {
        return tls::lan_candidates(port, allow_cgnat);
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => match tls::is_bindable(ip, allow_cgnat) {
            Ok(()) => (
                vec![LanAddr {
                    ip: ip.to_string(),
                    interface: "configured".into(),
                    url: format!("https://{ip}:{port}"),
                }],
                Vec::new(),
            ),
            Err(reason) => (
                Vec::new(),
                vec![SkippedAddr {
                    ip: ip.to_string(),
                    interface: "configured".into(),
                    reason: reason.to_string(),
                }],
            ),
        },
        // A hostname: bind every private address.
        Err(_) => tls::lan_candidates(port, allow_cgnat),
    }
}

/// Start the TLS listeners on the LAN addresses when the configured host is
/// non-loopback, and keep them in step with the interfaces. Returns once the
/// first set is bound.
pub async fn start_lan(state: &AppState) {
    if !state.config.is_non_loopback() {
        state.remote.set_lan(LanStatus {
            enabled: false,
            port: state.config.port,
            ..Default::default()
        });
        return;
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    refresh(state).await;
    let mut rt = state.remote.lan_rt.lock().await;
    if rt.started {
        return;
    }
    rt.started = true;
    drop(rt);
    let st = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(REFRESH_EVERY).await;
            refresh(&st).await;
        }
    });
}

/// Re-read the interfaces; bind new addresses, drop vanished ones, re-issue
/// and hot-reload the certificate when its SANs are stale.
pub async fn refresh(state: &AppState) {
    if !state.config.is_non_loopback() {
        return;
    }
    let (addrs, skipped) = desired(state);
    let ips: Vec<IpAddr> = addrs.iter().filter_map(|a| a.ip.parse().ok()).collect();
    let dir = cert_dir(state);
    let wanted = ips.clone();
    let bundle = match tokio::task::spawn_blocking(move || tls::ensure_cert(&dir, &wanted)).await {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            tracing::error!(error = %e, "could not prepare the LAN TLS certificate");
            let mut lan = state.remote.lan();
            lan.enabled = true;
            lan.error = Some(format!("Could not prepare the TLS certificate: {e}"));
            state.remote.set_lan(lan);
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "certificate task failed");
            return;
        }
    };

    let mut rt = state.remote.lan_rt.lock().await;
    let config = match &rt.config {
        Some(cfg) => {
            if bundle.regenerated {
                if let Err(e) = cfg
                    .reload_from_pem(
                        bundle.cert_pem.clone().into_bytes(),
                        bundle.key_pem.clone().into_bytes(),
                    )
                    .await
                {
                    tracing::error!(error = %e, "could not reload the LAN certificate");
                } else {
                    tracing::info!("LAN certificate re-issued for new addresses");
                }
            }
            cfg.clone()
        }
        None => match RustlsConfig::from_pem(
            bundle.cert_pem.clone().into_bytes(),
            bundle.key_pem.clone().into_bytes(),
        )
        .await
        {
            Ok(c) => {
                rt.config = Some(c.clone());
                c
            }
            Err(e) => {
                tracing::error!(error = %e, "could not build the TLS config");
                return;
            }
        },
    };

    // Drop listeners on addresses that went away.
    let gone: Vec<IpAddr> = rt
        .bound
        .keys()
        .filter(|ip| !ips.contains(ip))
        .copied()
        .collect();
    for ip in gone {
        if let Some(h) = rt.bound.remove(&ip) {
            h.shutdown();
            tracing::info!(%ip, "LAN listener closed (address gone)");
        }
    }

    // Bind new ones.
    let port = state.config.port;
    let mut failed = Vec::new();
    for ip in &ips {
        if rt.bound.contains_key(ip) {
            continue;
        }
        let addr = SocketAddr::new(*ip, port);
        let handle = axum_server::Handle::new();
        let app = super::router_for(state.clone(), Ingress::Lan);
        let cfg = config.clone();
        let h = handle.clone();
        tokio::spawn(async move {
            if let Err(e) = axum_server::bind_rustls(addr, cfg)
                .handle(h)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await
            {
                tracing::error!(%addr, error = %e, "LAN TLS listener stopped");
            }
        });
        match tokio::time::timeout(Duration::from_secs(3), handle.listening()).await {
            Ok(Some(_)) => {
                tracing::info!(%addr, "LAN TLS listener bound");
                rt.bound.insert(*ip, handle);
            }
            _ => {
                handle.shutdown();
                failed.push(*ip);
            }
        }
    }
    for s in &skipped {
        tracing::debug!(ip = %s.ip, interface = %s.interface, reason = %s.reason, "LAN address not bound");
    }
    let bound_addrs: Vec<LanAddr> = addrs
        .into_iter()
        .filter(|a| {
            a.ip.parse::<IpAddr>()
                .is_ok_and(|ip| rt.bound.contains_key(&ip))
        })
        .collect();
    drop(rt);

    *state.fingerprint.write().await = Some(bundle.fingerprint.clone());
    state.remote.set_lan(LanStatus {
        enabled: true,
        port,
        addresses: bound_addrs,
        skipped,
        fingerprint: Some(bundle.fingerprint),
        ca_fingerprint: Some(bundle.ca_fingerprint),
        ca_der: Some(bundle.ca_der),
        error: (!failed.is_empty()).then(|| {
            format!(
                "Could not listen on {} (port {port} in use?)",
                failed
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }),
    });
}
