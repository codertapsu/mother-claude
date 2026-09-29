//! Performance measurements of the data layer against the REAL `~/.claude`
//! (read-only: nothing is written there; checkpoints go to a temp dir or to
//! `$MC_MEASURE_CACHE_DIR`). `#[ignore]`d so `cargo test` never runs them.
//!
//! ```text
//! MC_MEASURE_CACHE_DIR=/tmp/mc-cache cargo test --release --test measure \
//!     measure_cold -- --ignored --nocapture
//! MC_MEASURE_CACHE_DIR=/tmp/mc-cache cargo test --release --test measure \
//!     measure_warm -- --ignored --nocapture
//! ```
//!
//! `measure_startup_settings` follows the app's real startup path instead
//! (`monitor::run` with the background index worker, a dashboard on the
//! WebSocket) and samples each live session's ultracode / effort / thinking
//! at fixed times after launch; see its doc comment.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use mother_claude_lib::claude::index::TranscriptIndex;
use mother_claude_lib::claude::live::LiveRoster;
use mother_claude_lib::claude::ClaudeHome;
use mother_claude_lib::server::auth::Auth;
use mother_claude_lib::server::monitor;
use mother_claude_lib::state::{Inner, ServerConfig};

/// Peak resident set size of this process, in MB.
fn peak_rss_mb() -> f64 {
    // SAFETY: getrusage fills a caller-owned struct.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let raw = ru.ru_maxrss as f64;
    if cfg!(target_os = "macos") {
        raw / (1024.0 * 1024.0) // bytes
    } else {
        raw / 1024.0 // kilobytes
    }
}

/// Current resident set size, in MB (`ps -o rss=`).
fn rss_mb() -> f64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<f64>()
                .ok()
        })
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

fn cache_dir() -> (PathBuf, Option<tempfile::TempDir>) {
    match std::env::var("MC_MEASURE_CACHE_DIR") {
        Ok(d) if !d.is_empty() => {
            std::fs::create_dir_all(&d).unwrap();
            (PathBuf::from(d), None)
        }
        _ => {
            let t = tempfile::tempdir().unwrap();
            (t.path().to_path_buf(), Some(t))
        }
    }
}

fn total_bytes(home: &ClaudeHome, idx: &TranscriptIndex) -> (u64, usize) {
    let _ = home;
    let mut bytes = 0;
    let mut files = 0;
    for sid in idx.session_ids() {
        if let Some(f) = idx.session_files(&sid) {
            bytes += f.main_size;
            files += f.main.is_some() as usize;
            for a in f.subagents.values() {
                bytes += a.size;
                files += 1;
            }
        }
    }
    (bytes, files)
}

fn live_ids(home: &ClaudeHome) -> (Vec<String>, Duration) {
    let roster = LiveRoster::new();
    let t = Instant::now();
    roster.refresh(home);
    let cold = t.elapsed();
    let ids = roster
        .all()
        .into_iter()
        .filter_map(|s| s.session_id)
        .collect();
    (ids, cold)
}

fn tick(
    idx: &TranscriptIndex,
    home: &ClaudeHome,
    roster: &LiveRoster,
    live: &[String],
) -> Duration {
    // What the monitor's 2 s tick does: pidfiles, then a stat of each live
    // session's main transcript and recently written subagent files.
    let t = Instant::now();
    roster.refresh(home);
    let now = chrono::Utc::now().timestamp_millis();
    let paths = idx.tick_paths(live, now, Some(15 * 60 * 1000));
    idx.update(&paths);
    t.elapsed()
}

fn sweep(idx: &TranscriptIndex, live: &[String]) -> Duration {
    let t = Instant::now();
    let now = chrono::Utc::now().timestamp_millis();
    let paths = idx.tick_paths(live, now, None);
    idx.update(&paths);
    t.elapsed()
}

#[test]
#[ignore]
fn measure_cold() {
    let home = ClaudeHome::resolve().unwrap();
    let (dir, _keep) = cache_dir();
    let _ = std::fs::remove_file(dir.join("index-cache.json"));
    let rss_start = rss_mb();
    let t = Instant::now();
    let idx = TranscriptIndex::new(Some(&dir));
    idx.load_cache();
    idx.discover(&home);
    let discover = t.elapsed();
    let (bytes, files) = total_bytes(&home, &idx);
    idx.queue_all();
    let jobs = idx.pending_jobs();
    idx.drain_jobs_blocking();
    let cold = t.elapsed();
    let peak = peak_rss_mb();
    let rss_after = rss_mb();
    let t = Instant::now();
    assert!(idx.persist(true));
    let persist = t.elapsed();
    let cache_size = std::fs::metadata(dir.join("index-cache.json"))
        .map(|m| m.len())
        .unwrap_or(0);

    let (live, roster_cold) = live_ids(&home);
    let roster = LiveRoster::new();
    roster.refresh(&home);
    let mut ticks = Vec::new();
    for _ in 0..20 {
        ticks.push(tick(&idx, &home, &roster, &live));
        std::thread::sleep(Duration::from_millis(50));
    }
    ticks.sort();
    let sweep_time = sweep(&idx, &live);
    println!("── cold scan (real ~/.claude, read-only) ──");
    println!(
        "sessions: {}, transcript files: {files}, bytes: {:.1} MB",
        idx.session_ids().len(),
        bytes as f64 / 1e6
    );
    println!("discovery: {:?}", discover);
    println!(
        "jobs: {jobs}; cold full index: {:?} ({:.0} MB/s)",
        cold,
        bytes as f64 / 1e6 / cold.as_secs_f64()
    );
    println!("rss start {rss_start:.1} MB, rss after {rss_after:.1} MB, PEAK rss {peak:.1} MB");
    println!(
        "checkpoint write: {:?}, index-cache.json {:.2} MB",
        persist,
        cache_size as f64 / 1e6
    );
    println!(
        "live sessions: {} (roster cold refresh {:?})",
        live.len(),
        roster_cold
    );
    println!(
        "incremental tick (roster + stat/scan of live files): median {:?}, max {:?}",
        ticks[ticks.len() / 2],
        ticks[ticks.len() - 1]
    );
    println!("sweep (stat every file of live sessions): {:?}", sweep_time);
}

#[test]
#[ignore]
fn measure_warm() {
    let home = ClaudeHome::resolve().unwrap();
    let (dir, _keep) = cache_dir();
    let rss_start = rss_mb();
    let t = Instant::now();
    let idx = TranscriptIndex::new(Some(&dir));
    idx.load_cache();
    let load = t.elapsed();
    idx.discover(&home);
    let (live, _) = live_ids(&home);
    for sid in &live {
        idx.ensure(sid, true);
    }
    let live_ready = t.elapsed();
    idx.queue_all();
    let jobs = idx.pending_jobs();
    let paths: Vec<PathBuf> = idx
        .session_ids()
        .iter()
        .flat_map(|s| idx.indexed_paths(s))
        .collect();
    idx.update(&paths);
    idx.drain_jobs_blocking();
    let warm = t.elapsed();
    println!("── warm restart from index-cache.json ──");
    println!(
        "cache load: {:?}; live sessions ready: {:?}",
        load, live_ready
    );
    println!(
        "jobs left after checkpoint adoption: {jobs}; everything current: {:?}",
        warm
    );
    println!(
        "rss start {rss_start:.1} MB, PEAK rss {:.1} MB",
        peak_rss_mb()
    );
}

/// The whole app path: `monitor::refresh_once` on an AppState over the real
/// home (cold, inline indexing), then warm refresh cycles (roster + index +
/// agents + summaries + snapshot diff).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn measure_app_cycle() {
    let home = ClaudeHome::resolve().unwrap();
    let st = Inner::new(
        home,
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
    );
    let t = Instant::now();
    monitor::refresh_once(&st).await;
    let cold = t.elapsed();
    let mut cycles = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        monitor::refresh_once(&st).await;
        cycles.push(t.elapsed());
    }
    cycles.sort();
    let mut ticks = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        monitor::tick_once(&st).await;
        ticks.push(t.elapsed());
    }
    ticks.sort();
    println!(
        "steady tick (roster + live files + summaries + diff): median {:?}, max {:?}",
        ticks[ticks.len() / 2],
        ticks[ticks.len() - 1]
    );
    let sessions = st.sessions().await;
    let live: HashSet<&str> = sessions
        .iter()
        .filter(|s| s.live)
        .map(|s| s.id.as_str())
        .collect();
    println!("── app refresh cycle ──");
    println!("cold refresh_once (index everything inline): {:?}", cold);
    println!(
        "warm cycle (discovery + roster + agents + {} summaries): median {:?}, max {:?}",
        sessions.len(),
        cycles[cycles.len() / 2],
        cycles[cycles.len() - 1]
    );
    println!("live: {}; peak rss {:.1} MB", live.len(), peak_rss_mb());
    for s in sessions.iter().filter(|s| s.live) {
        println!(
            "  {} {:?} {:?} cost={:?} ctx={:?} agents={}/{} wf={} bg={} mode={:?} model={:?}",
            &s.id[..8],
            s.status,
            s.activity.label,
            s.cost_usd.map(|c| (c * 100.0).round() / 100.0),
            s.context.as_ref().map(|c| c.pct),
            s.agents.running,
            s.agents.total,
            s.agents.workflows_running,
            s.agents.background_running,
            s.permission_mode,
            s.model.as_ref().map(|m| m.label.clone()),
        );
        println!(
            "      effort={:?} ({:?}) ultracode={:?} ({:?}) thinking={} last_thinking={:?} fast={:?} thinking_tokens={:?}",
            s.effort,
            s.effort_source,
            s.ultracode,
            s.ultracode_source,
            serde_json::to_string(&s.thinking).unwrap_or_default(),
            s.last_thinking_at,
            s.fast,
            s.usage.thinking,
        );
        let (agents, workflows) = st.agents.for_session(&s.id);
        for a in agents
            .iter()
            .filter(|a| a.status == mother_claude_lib::claude::model::AgentStatus::Running)
        {
            println!(
                "      running {:?} {} {:?} wf={:?} last={:?}",
                a.kind, a.id, a.label, a.workflow_run_id, a.last_activity_at
            );
        }
        for w in workflows
            .iter()
            .filter(|w| w.status == mother_claude_lib::claude::model::WorkflowStatus::Running)
        {
            println!(
                "      workflow {} {:?} started={:?} agents={:?}",
                w.run_id, w.name, w.started_at, w.agents
            );
        }
    }
}

/// Real-data smoke of the HTTP routes: session list, detail, transcript
/// pages (backwards through the biggest live transcript), agents, fleet.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn measure_routes() {
    let home = ClaudeHome::resolve().unwrap();
    let st = Inner::new(
        home,
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        Auth::ephemeral(),
    );
    monitor::refresh_once(&st).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = mother_claude_lib::server::router(st.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let base = format!("http://{addr}");
    let c = reqwest::Client::new();
    let token = st.auth.token.clone();
    let get = |url: String| {
        let c = c.clone();
        let token = token.clone();
        async move {
            let t = Instant::now();
            let r = c.get(url).bearer_auth(token).send().await.unwrap();
            let status = r.status();
            let bytes = r.bytes().await.unwrap();
            (status, bytes, t.elapsed())
        }
    };
    let (s, b, d) = get(format!("{base}/api/sessions?scope=all")).await;
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    println!(
        "GET /sessions?scope=all: {s} {} sessions {:.0} KB in {:?}",
        v["sessions"].as_array().unwrap().len(),
        b.len() as f64 / 1024.0,
        d
    );
    let (_, b, d) = get(format!("{base}/api/sessions")).await;
    println!(
        "GET /sessions (active): {:.0} KB in {:?}",
        b.len() as f64 / 1024.0,
        d
    );
    // The live session with the biggest transcript.
    let mut live: Vec<(u64, String)> = st
        .sessions()
        .await
        .into_iter()
        .filter(|s| s.live)
        .filter_map(|s| {
            let size = st.index.session_files(&s.id)?.main_size;
            Some((size, s.id))
        })
        .collect();
    live.sort();
    let Some((size, id)) = live.pop() else {
        return;
    };
    println!(
        "biggest live transcript: {} ({:.0} MB)",
        &id[..8],
        size as f64 / 1e6
    );
    let (s, b, d) = get(format!("{base}/api/sessions/{id}")).await;
    println!(
        "GET /sessions/{{id}}: {s} {:.0} KB in {:?}",
        b.len() as f64 / 1024.0,
        d
    );
    let mut cursor: Option<String> = None;
    let mut total = 0usize;
    for page in 0..8 {
        let url = match &cursor {
            Some(c) => format!("{base}/api/sessions/{id}/transcript?limit=150&before={c}"),
            None => format!("{base}/api/sessions/{id}/transcript?limit=150"),
        };
        let (s, b, d) = get(url).await;
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        let n = v["events"].as_array().map(|a| a.len()).unwrap_or(0);
        total += n;
        println!(
            "  page {page}: {s} {n} events, {:.0} KB, {:?}, before={:?}",
            b.len() as f64 / 1024.0,
            d,
            v["before"].as_str()
        );
        cursor = v["before"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    println!("  {total} events over the pages");
    let (s, b, d) = get(format!("{base}/api/sessions/{id}/agents")).await;
    println!(
        "GET /sessions/{{id}}/agents: {s} {:.0} KB in {:?}",
        b.len() as f64 / 1024.0,
        d
    );
    let (s, b, d) = get(format!("{base}/api/agents")).await;
    println!(
        "GET /agents: {s} {:.0} KB in {:?}",
        b.len() as f64 / 1024.0,
        d
    );
    let (s, b, d) = get(format!("{base}/api/inbox")).await;
    println!(
        "GET /inbox: {s} {:.0} KB in {:?}",
        b.len() as f64 / 1024.0,
        d
    );
    println!("peak rss {:.1} MB", peak_rss_mb());
}

/// Epoch ms as `YYYY-MM-DDTHH:MM:SSZ`, or `-`.
fn iso(ms: Option<i64>) -> String {
    ms.and_then(chrono::DateTime::from_timestamp_millis)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "-".into())
}

/// An ultracode replay in one line: what Claude Code sees, and the state as
/// of the last regular prompt.
fn ultra_line(r: &mother_claude_lib::claude::index::UltraReplay) -> String {
    let state = r.state.as_ref().map_or_else(
        || "none".to_string(),
        |s| {
            format!(
                "{}@{} ({}{})",
                match s.on {
                    Some(true) => "on".to_string(),
                    Some(false) => "off".to_string(),
                    None => format!("unknown: {:?}", s.unknown),
                },
                iso(s.at),
                if s.marker { "marker" } else { "no marker" },
                if s.seeded { ", unseeded" } else { "" }
            )
        },
    );
    format!("seen={:?} state={state}", r.seen)
}

/// The app's real startup path against the REAL `~/.claude` (read-only), as
/// `server::serve` runs it: `monitor::run` (its first cycle bootstraps the
/// live transcripts from a tail window; the background index worker then
/// runs their full passes and the history) with a dashboard connected to the
/// loopback router's WebSocket from the start. At fixed times after launch it
/// prints each live session's ultracode / effort / thinking as the dashboard
/// received them in `sessions` frames and as the index holds them, plus what
/// the ultracode derivation would give without its process-start rule.
/// Then: every change of a live session's ultracode in the frames, when each
/// live main transcript became complete, and what a checkpoint keeps.
///
/// ```text
/// # cold: no checkpoint (a first launch, or a CACHE_VERSION bump)
/// cargo test --test measure measure_startup_settings -- --ignored --nocapture
/// # warm: start from a COPY of a checkpoint file (the source is only read)
/// MC_TRACE_CACHE=".../dev.motherclaude.Mother-Claude/index-cache.json" cargo test …
/// # sample times in seconds (default 1,5,30,90)
/// MC_TRACE_AT=1,2,5,10,30,60,90 cargo test …
/// ```
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn measure_startup_settings() {
    use futures_util::{SinkExt, StreamExt};
    use mother_claude_lib::claude::summaries::{derive_settings, SettingsFacts};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio_tungstenite::tungstenite::Message;

    let home = ClaudeHome::resolve().unwrap();
    let data = tempfile::tempdir().unwrap();
    let mode = match std::env::var("MC_TRACE_CACHE") {
        Ok(src) if !src.is_empty() => {
            std::fs::copy(&src, data.path().join("index-cache.json")).unwrap();
            "warm (a copy of the given checkpoint file)"
        }
        _ => "cold (no checkpoint)",
    };
    let at: Vec<f64> = std::env::var("MC_TRACE_AT")
        .ok()
        .map(|s| {
            s.split(',')
                .filter_map(|x| x.trim().parse().ok())
                .collect::<Vec<f64>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![1.0, 5.0, 30.0, 90.0]);
    let mut auth = Auth::ephemeral();
    // Checkpoints go to the temp dir, as they go to the app's data dir.
    auth.config_dir = Some(data.path().to_path_buf());
    let st = Inner::new(
        home,
        ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        auth,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = mother_claude_lib::server::router(st.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let t0 = Instant::now();
    tokio::spawn(monitor::run(st.clone()));

    // The dashboard: the latest summary per id, as the `sessions` frames
    // (full, then upsert/remove diffs) built it.
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    let (mut tx, mut rx) = ws.split();
    tx.send(Message::Text(
        serde_json::json!({ "t": "auth", "token": st.auth.token })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let seen: Arc<Mutex<HashMap<String, Value>>> = Arc::default();
    let changes: Arc<Mutex<Vec<String>>> = Arc::default();
    {
        let (seen, changes) = (seen.clone(), changes.clone());
        tokio::spawn(async move {
            while let Some(Ok(msg)) = rx.next().await {
                let Message::Text(text) = msg else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if v["t"] == "ping" {
                    // The server drops a client silent for 60 s.
                    let _ = tx.send(Message::Text(r#"{"t":"pong"}"#.into())).await;
                    continue;
                }
                if v["t"] != "sessions" {
                    continue;
                }
                let secs = t0.elapsed().as_secs_f64();
                let full = v["full"] == true;
                let key = |s: &Value| {
                    (
                        s["ultracode"].clone(),
                        s["ultracodeSource"].clone(),
                        s["ultracodeAt"].clone(),
                        s["ultracodeUnknown"].clone(),
                    )
                };
                let mut g = seen.lock().unwrap();
                if full {
                    g.clear();
                }
                let list = if full { &v["sessions"] } else { &v["upsert"] };
                for s in list.as_array().into_iter().flatten() {
                    let id = s["id"].as_str().unwrap_or_default().to_string();
                    if s["live"] == true && g.get(&id).map(key) != Some(key(s)) {
                        changes.lock().unwrap().push(format!(
                            "  {secs:>7.2}s seq={} full={full} {} ultracode={} source={} at={} unknown={}",
                            v["seq"],
                            id.get(..8).unwrap_or(&id),
                            s["ultracode"],
                            s["ultracodeSource"],
                            iso(s["ultracodeAt"].as_i64()),
                            s["ultracodeUnknown"]
                        ));
                    }
                    g.insert(id, s.clone());
                }
                for id in v["remove"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    g.remove(id);
                }
            }
        });
    }

    println!("── startup trace (real ~/.claude, read-only; {mode}) ──");
    let mut complete_at: HashMap<String, f64> = HashMap::new();
    let mut next = 0;
    while next < at.len() {
        let secs = t0.elapsed().as_secs_f64();
        for p in st.live.all() {
            let Some(id) = p.session_id else { continue };
            if !complete_at.contains_key(&id)
                && st.index.with_main(&id, |f| f.is_complete()) == Some(true)
            {
                complete_at.insert(id, secs);
            }
        }
        if secs >= at[next] {
            next += 1;
            let live = st.live.all();
            println!(
                "t={secs:.1}s: {} live, {} index jobs queued, {} sessions on the dashboard",
                live.len(),
                st.index.pending_jobs(),
                seen.lock().unwrap().len()
            );
            let frames = seen.lock().unwrap().clone();
            for p in live {
                let Some(id) = p.session_id.clone() else {
                    continue;
                };
                let s = frames.get(&id);
                let field = |k: &str| {
                    s.map(|s| s[k].to_string())
                        .unwrap_or_else(|| "(no frame)".into())
                };
                println!(
                    "  {} started={} dashboard: ultracode={} source={} at={} unknown={} before={}@{} effort={} ({}) thinking={}",
                    id.get(..8).unwrap_or(&id),
                    iso(p.started_at),
                    field("ultracode"),
                    field("ultracodeSource"),
                    iso(s.and_then(|s| s["ultracodeAt"].as_i64())),
                    field("ultracodeUnknown"),
                    field("ultracodeBefore"),
                    iso(s.and_then(|s| s["ultracodeBeforeAt"].as_i64())),
                    field("effort"),
                    field("effortSource"),
                    field("thinking"),
                );
                let index = st.index.with_main(&id, |f| {
                    (
                        f.is_complete(),
                        f.start,
                        f.offset,
                        f.size,
                        f.agg.light_clone(),
                    )
                });
                match index {
                    None => println!("      index: no main transcript"),
                    Some((complete, start, offset, size, agg)) => {
                        let unfiltered = derive_settings(&SettingsFacts {
                            live: true,
                            agg: Some(&agg),
                            ..Default::default()
                        });
                        println!(
                            "      index: {} start={start} offset={offset} size={size} ultracode {} \
                             → without the process-start rule: {:?}",
                            if complete { "complete" } else { "tail window" },
                            ultra_line(&agg.ultra),
                            unfiltered.ultracode,
                        );
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    println!("ultracode of live sessions in the frames the dashboard received:");
    for c in changes.lock().unwrap().iter() {
        println!("{c}");
    }
    let mut complete: Vec<_> = complete_at.iter().collect();
    complete.sort_by(|a, b| a.1.total_cmp(b.1));
    for (id, secs) in complete {
        println!(
            "  {} main transcript complete (full pass or checkpoint) at {secs:.2}s",
            id.get(..8).unwrap_or(id)
        );
    }
    // What a checkpoint keeps (only complete files are written).
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || st2.index.persist(true))
        .await
        .unwrap();
    let live: HashSet<String> = st
        .live
        .all()
        .into_iter()
        .filter_map(|p| p.session_id)
        .collect();
    if let Ok(bytes) = std::fs::read(data.path().join("index-cache.json")) {
        let cache: Value = serde_json::from_slice(&bytes).unwrap_or_default();
        println!("checkpoint (version {}):", cache["version"]);
        for f in cache["files"].as_array().into_iter().flatten() {
            let sid = f["sessionId"].as_str().unwrap_or_default();
            if live.contains(sid) && f["kind"] == "main" {
                println!(
                    "  {} offset={} start={} agg.ultra={}",
                    sid.get(..8).unwrap_or(sid),
                    f["offset"],
                    f["start"],
                    f["agg"]["ultra"]
                );
            }
        }
    }
}

/// What a live transcript's tail window says about ultracode, before and
/// after seeding it with the history before the window (the bootstrap uses
/// `TAIL_WINDOW`), against a full pass, on the REAL files (read-only).
///
/// ```text
/// cargo test --test measure measure_tail_window_markers -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn measure_tail_window_markers() {
    use mother_claude_lib::claude::index::{FileIndex, FileKind, TAIL_WINDOW};

    let home = ClaudeHome::resolve().unwrap();
    let idx = TranscriptIndex::new(None);
    idx.discover(&home);
    let (live, _) = live_ids(&home);
    for sid in live {
        let Some(path) = idx.transcript_path(&sid) else {
            println!("{}: no main transcript", sid.get(..8).unwrap_or(&sid));
            continue;
        };
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        println!("{} ({size} bytes):", sid.get(..8).unwrap_or(&sid));
        for w in [1 << 20, 2 << 20, 3 << 20, 4 << 20, TAIL_WINDOW] {
            let mut fi = FileIndex::new(path.clone(), FileKind::Main, &sid);
            fi.seek_window(w).unwrap();
            let start = fi.offset;
            fi.scan(None).unwrap();
            let window = ultra_line(&fi.agg.ultra);
            let t = Instant::now();
            fi.seed_ultra(start, None);
            println!(
                "  tail window {:>2} MiB (start {:>10}): {window}\n      seeded in {:?}: {}",
                w >> 20,
                fi.start,
                t.elapsed(),
                ultra_line(&fi.agg.ultra)
            );
        }
        let t = Instant::now();
        let mut fi = FileIndex::new(path.clone(), FileKind::Main, &sid);
        fi.scan(None).unwrap();
        println!(
            "  full pass ({:?}): {}, last compaction {}",
            t.elapsed(),
            ultra_line(&fi.agg.ultra),
            iso(fi.agg.last_compact_ts)
        );
    }
}
