//! `mc-spike` — the Phase 0 spike of docs/PLAN-MACHINES.md. Never shipped.
//!
//! S1 (transport): iroh endpoints addressed by key, the app's own HTTP stack
//! (axum + hyper, WebSocket included) served over QUIC streams, raw stream
//! throughput, and the three relay settings of D10 (n0's public relays, your
//! own relay, none). S5 (setup): the read-only detection of §5.14's checks.
//!
//! ```text
//! mc-spike local  [--only direct,mdns,own-relay,public-relay,n0-dns] [--mib 256] [--relay-mib 4]
//! mc-spike id     [--name spike | --key FILE]
//! mc-spike serve  [--name spike] [--relay public|none|URL[,URL]] [--lookup n0,mdns]
//!                 [--pair-code auto|CODE] [--allow ID]... [--open]
//!                 [--forward http://127.0.0.1:6725 --token-file auto|FILE]
//! mc-spike bench  TICKET|ID [--name client] [--relay …] [--lookup …] [--pair-code CODE] [--mib 64]
//! mc-spike get    TICKET|ID PATH [--name client] [--relay …] [--lookup …]
//! mc-spike checks
//! ```
//!
//! Logs: `MC_SPIKE_LOG=iroh=debug`. Keys live in `<data dir>/mc-spike/<name>.key`
//! (0600 on unix); nothing is written anywhere else.

mod args;
mod bench;
mod checks;
mod http;
mod local;
mod net;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: mc-spike local|id|serve|bench|get|checks [options] (see src/main.rs)";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("MC_SPIKE_LOG").unwrap_or_else(|_| EnvFilter::new("off")),
        )
        .with_writer(std::io::stderr)
        .init();
    let a = args::Args::parse(std::env::args().skip(1))?;
    match a.cmd.as_str() {
        "local" => local::cmd_local(&a).await,
        "id" => net::cmd_id(&a),
        "serve" => http::cmd_serve(&a).await,
        "bench" => bench::cmd_bench(&a).await,
        "get" => bench::cmd_get(&a).await,
        "checks" => checks::cmd_checks(&a).await,
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2)
        }
    }
}
