//! An API server beyond this computer (docs/CLAUDE-API.md "Access"): with
//! `access: lan` its port is also bound on each of the machine's private
//! addresses, with `access: all` on every address it has (public ones
//! included) — plain HTTP, the server's own routes
//! ([`crate::bridge::network_router`], ingress `ApiNetwork`: never local, no
//! Mother Claude token read). So a program on another computer (a backend
//! that calls the Claude API, Postman) reaches it at `http://<address>:<port>`
//! with no certificate to trust. What a caller there may do is
//! `require_bridge_auth`'s: with the server's key, everything; without one,
//! the Claude API.
//!
//! The addresses are re-read every 30 seconds, as the dashboard's LAN
//! listeners do (`server::listeners`): one that appears is bound, one that
//! goes is closed — so a server started at login before the network is up
//! is reachable once it is.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{oneshot, watch};

use crate::state::AppState;

/// How often the addresses are re-read.
const REFRESH_EVERY: Duration = Duration::from_secs(30);
/// How soon an address that could not be bound is tried again (a restart in
/// place frees the port a moment later).
const RETRY_EVERY: Duration = Duration::from_secs(2);

/// A server's listeners on the network, while it runs with them.
#[derive(Debug)]
pub struct NetworkApi {
    stop: watch::Sender<bool>,
    urls: Arc<Mutex<Vec<String>>>,
}

impl NetworkApi {
    /// The addresses it serves now (`http://<ip>:<port>`).
    pub fn urls(&self) -> Vec<String> {
        self.urls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Closes every listener.
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

impl Drop for NetworkApi {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts serving `server` on `port` of the addresses `ips` names (re-read
/// every 30 seconds) until it is stopped.
pub fn start<F>(state: AppState, server: String, port: u16, ips: F) -> NetworkApi
where
    F: Fn(&AppState) -> Vec<IpAddr> + Send + 'static,
{
    let (stop, mut stopped) = watch::channel(false);
    let urls = Arc::new(Mutex::new(Vec::new()));
    let shown = urls.clone();
    tokio::spawn(async move {
        let mut bound: BTreeMap<IpAddr, oneshot::Sender<()>> = BTreeMap::new();
        loop {
            let wanted = ips(&state);
            // Gone: closed.
            let gone: Vec<IpAddr> = bound
                .keys()
                .filter(|ip| !wanted.contains(ip))
                .copied()
                .collect();
            for ip in gone {
                if let Some(tx) = bound.remove(&ip) {
                    let _ = tx.send(());
                }
            }
            // New: bound (an address in use is tried again soon).
            let mut missed = false;
            for ip in wanted {
                if bound.contains_key(&ip) {
                    continue;
                }
                match bind(&state, &server, ip, port).await {
                    Ok(tx) => {
                        tracing::info!(%ip, port, server = %server, "API server listening on the network");
                        bound.insert(ip, tx);
                    }
                    Err(e) => {
                        missed = true;
                        tracing::warn!(%ip, port, error = %e, "could not open an API server on the network");
                    }
                }
            }
            *shown.lock().unwrap_or_else(|e| e.into_inner()) = bound
                .keys()
                .map(|ip| format!("http://{}", SocketAddr::new(*ip, port)))
                .collect();
            let wait = if missed { RETRY_EVERY } else { REFRESH_EVERY };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = stopped.changed() => break,
            }
            if *stopped.borrow() {
                break;
            }
        }
        for (_, tx) in bound {
            let _ = tx.send(());
        }
        shown.lock().unwrap_or_else(|e| e.into_inner()).clear();
    });
    NetworkApi { stop, urls }
}

/// One address's listener: its stop.
async fn bind(
    state: &AppState,
    server: &str,
    ip: IpAddr,
    port: u16,
) -> std::io::Result<oneshot::Sender<()>> {
    let socket = tokio::net::TcpListener::bind(SocketAddr::new(ip, port)).await?;
    let app = crate::bridge::network_router(state.clone(), server, port);
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let served = axum::serve(
            socket,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = rx.await;
        })
        .await;
        if let Err(e) = served {
            tracing::warn!(%ip, error = %e, "an API server's listener on the network stopped");
        }
    });
    Ok(tx)
}
