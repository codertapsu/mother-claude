//! Both ends on this machine, one scenario per relay setting / way of finding
//! the peer. What it proves: the stack works end to end on this OS, each D10
//! choice connects, and the numbers that do not depend on a second machine.

use std::time::Duration;

use anyhow::{anyhow, Result};
use iroh::{EndpointAddr, SecretKey};
use serde_json::{json, Value};

use crate::{
    args::Args,
    bench::{self, Suite},
    http::{self, Shared},
    net::{self, ms, NetOpts, Relay},
};

const SCENARIOS: &[&str] = &["direct", "mdns", "own-relay", "public-relay", "n0-dns"];

pub async fn cmd_local(a: &Args) -> Result<()> {
    let only: Option<Vec<String>> = a
        .get("only")
        .map(|o| o.split(',').map(str::to_string).collect());
    let mib = a.num("mib", 256)?;
    let relay_mib = a.num("relay-mib", 4)?;
    let mut all = Vec::new();
    for sc in SCENARIOS {
        if let Some(o) = &only {
            if !o.iter().any(|x| x == sc) {
                continue;
            }
        }
        eprintln!("== {sc}");
        let v = match run(sc, mib, relay_mib).await {
            Ok(v) => v,
            Err(e) => json!({ "scenario": sc, "error": format!("{e:#}") }),
        };
        println!("{}", serde_json::to_string(&v)?);
        all.push(v);
    }
    Ok(())
}

async fn run(sc: &str, mib: u64, relay_mib: u64) -> Result<Value> {
    #[allow(unused_mut)]
    let mut relay_server: Option<Box<dyn std::any::Any + Send>> = None;
    // (server opts, client opts, dial by id alone, MiB per transfer)
    let (so, co, by_id, m) = match sc {
        // LAN-like: no relay, the server's direct addresses in the ticket.
        "direct" => (
            NetOpts::plain(Relay::None),
            NetOpts::plain(Relay::None),
            false,
            mib,
        ),
        // The "No relay" setting: found by mDNS from the id alone.
        "mdns" => {
            let o = NetOpts {
                mdns: true,
                ..NetOpts::plain(Relay::None)
            };
            (o.clone(), o, true, mib)
        }
        // "Your own relay": a relay server started here (self-signed), every packet through it.
        #[cfg(feature = "local-relay")]
        "own-relay" => {
            let (_map, url, server) = iroh::test_utils::run_relay_server()
                .await
                .map_err(|e| anyhow!("local relay: {e}"))?;
            relay_server = Some(Box::new(server));
            let o = NetOpts {
                relay_only: true,
                insecure_relay_tls: true,
                ..NetOpts::plain(Relay::Own(vec![url]))
            };
            (o.clone(), o, true, mib)
        }
        // "Public relays": n0's relays, every packet through them (few MiB, to be polite).
        "public-relay" => {
            let o = NetOpts {
                relay_only: true,
                ..NetOpts::plain(Relay::Public)
            };
            (o.clone(), o, false, relay_mib)
        }
        // The default setting: id alone, found through n0's DNS, then upgraded to direct.
        "n0-dns" => {
            let o = NetOpts {
                n0: true,
                ..NetOpts::plain(Relay::Public)
            };
            (o.clone(), o, true, relay_mib)
        }
        _ => return Err(anyhow!("unknown scenario {sc}")),
    };

    let server = net::bind(SecretKey::generate(), &so, true).await?;
    net::wait_online(&server, &so, 20).await?;
    let accept = tokio::spawn(http::accept_loop(server.clone(), Shared::open_for_tests()));
    let client = net::bind(SecretKey::generate(), &co, false).await?;
    net::wait_online(&client, &co, 20).await?;

    let addr = if by_id {
        // With your own relay the id is completed with the relay, as `target_addr` does.
        let mut a = EndpointAddr::new(server.id());
        if let Relay::Own(urls) = &co.relay {
            a = a.with_relay_url(urls[0].clone());
        }
        a
    } else {
        server.addr()
    };
    let dialled = format!("{addr:?}");
    let (conn, connect) = bench::connect(&client, addr, Duration::from_secs(45)).await?;
    let at_connect = net::paths(&conn);
    let changes = bench::watch_path(&conn, if sc == "n0-dns" { 5 } else { 1 }).await;
    let suite = bench::run_suite(
        &conn,
        &Suite {
            mib: m,
            requests: 50,
            ws_msgs: 200,
            concurrent: 32,
        },
    )
    .await?;
    conn.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
    accept.abort();
    drop(relay_server);
    Ok(json!({
        "scenario": sc,
        "net": so.describe(),
        "dialled": dialled,
        "connectMs": ms(connect),
        "pathsAtConnect": at_connect,
        "pathChanges": changes,
        "suite": suite,
    }))
}
