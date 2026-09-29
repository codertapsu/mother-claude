//! Plan usage: the parsers (scales, whitelisting, codename keys ignored), the
//! CLI-cache fallback, the scheduler decision table with an injected clock,
//! the store, and the helper protocol against a fake `claude`.

use super::*;
use serde_json::json;

const NOW: i64 = 1_790_000_000_000;
const MIN: i64 = 60_000;

/// A `get_usage` answer shaped like CLI 2.1.284's (codename windows, spend,
/// breakdown and dollar fields included — none of it may come out).
fn full_answer() -> Value {
    json!({
        "session": { "id": "x" },
        "subscription_type": "max",
        "rate_limits_available": true,
        "behaviors": null,
        "rate_limits": {
            "five_hour": { "utilization": 42, "resets_at": "2026-09-29T15:00:00.123456+00:00",
                           "limit_dollars": null, "used_dollars": 12.5 },
            "seven_day": { "utilization": 18, "resets_at": "2026-10-03T08:00:00.000000+00:00" },
            "seven_day_opus": null,
            "amber_cistern": { "utilization": 99, "resets_at": null },
            "iguana_necktie": { "used_dollars": 3.0 },
            "spend": { "used": 1234, "limit": { "amount_minor": 50000, "currency": "USD" } },
            "seven_day_breakdown": { "rows": [{ "key": "chat", "percent": 3 }] },
            "member_dashboard_available": true,
            "model_scoped": [{ "display_name": "Fable", "utilization": 12,
                               "resets_at": "2026-10-03T08:00:00+00:00" }],
            "limits": [
                { "kind": "session", "group": "session", "percent": 42, "severity": "warning",
                  "resets_at": "2026-09-29T15:00:00.123456+00:00", "scope": null,
                  "is_active": true },
                { "kind": "weekly_all", "group": "weekly", "percent": 18, "severity": "normal",
                  "resets_at": "2026-10-03T08:00:00.000000+00:00", "scope": null,
                  "is_active": false },
                { "kind": "weekly_scoped", "group": "weekly", "percent": 12, "severity": "normal",
                  "resets_at": "2026-10-03T08:00:00+00:00",
                  "scope": { "model": { "id": null, "display_name": "Fable" }, "surface": null },
                  "is_active": false },
                { "kind": "weekly_scoped", "group": "weekly", "percent": 0, "severity": "normal",
                  "resets_at": "2026-10-03T08:00:00+00:00",
                  "scope": { "model": { "display_name": "Sonnet" } }, "is_active": false }
            ],
            "extra_usage": { "is_enabled": true, "monthly_limit": 5000, "used_credits": 1250,
                             "utilization": 25.0, "currency": "usd", "decimal_places": 2,
                             "disabled_reason": null, "user_disabled": false,
                             "spend_limit_reached": false, "credits_ever_enabled": true,
                             "daily": null, "weekly": null }
        }
    })
}

fn iso(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .timestamp_millis()
}

#[test]
fn get_usage_answers_are_whitelisted() {
    let UsageAnswer::Data {
        windows,
        extra_usage,
    } = parse_get_usage(&full_answer()).unwrap()
    else {
        panic!("data expected");
    };
    let keys: Vec<&str> = windows.iter().map(|w| w.key.as_str()).collect();
    // `limits[]` rows verbatim, in order; an inactive row at 0 % is skipped,
    // an inactive one with usage is kept.
    assert_eq!(keys, vec!["five_hour", "seven_day", "seven_day_fable"]);
    assert_eq!(windows[0].label, "Current session");
    assert_eq!(windows[0].pct, 42.0);
    assert_eq!(
        windows[0].resets_at,
        Some(iso("2026-09-29T15:00:00.123456+00:00"))
    );
    assert_eq!(windows[0].severity, Some(PlanSeverity::Warning));
    assert_eq!(windows[1].label, "Weekly · all models");
    assert_eq!(windows[2].label, "Weekly · Fable");
    assert_eq!(windows[2].severity, Some(PlanSeverity::Normal));
    let extra = extra_usage.unwrap();
    assert!(extra.enabled);
    assert_eq!(extra.used_minor, 1250);
    assert_eq!(extra.limit_minor, Some(5000));
    assert_eq!(extra.currency.as_deref(), Some("USD"));
    assert_eq!(extra.decimals, 2);
    assert_eq!(extra.utilization, Some(25.0));

    // Nothing else reaches the wire.
    let usage = PlanUsage {
        available: true,
        windows,
        extra_usage: Some(extra),
        ..Default::default()
    };
    let text = serde_json::to_string(&usage).unwrap();
    for secret in [
        "amber",
        "iguana",
        "spend",
        "breakdown",
        "dollars",
        "chat",
        "max",
        "\"id\"",
        "subscription",
        "member",
    ] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
}

#[test]
fn documented_windows_without_limit_rows() {
    let resp = json!({ "rate_limits_available": true, "rate_limits": {
        "seven_day": { "utilization": 7.25, "resets_at": "2026-10-03T08:00:00+00:00" },
        "five_hour": { "utilization": 101, "resets_at": "bogus" },
        "seven_day_opus": null,
        "seven_day_sonnet": { "utilization": 3, "resets_at": null },
        "limits": [],
        "nimbus_quill": { "utilization": 50 }
    }});
    let UsageAnswer::Data {
        windows,
        extra_usage,
    } = parse_get_usage(&resp).unwrap()
    else {
        panic!("data expected");
    };
    let keys: Vec<&str> = windows.iter().map(|w| w.key.as_str()).collect();
    assert_eq!(keys, vec!["five_hour", "seven_day", "seven_day_sonnet"]);
    assert_eq!(windows[0].pct, 101.0, "over 100 % is kept");
    assert_eq!(windows[0].resets_at, None, "an unparseable time is dropped");
    assert_eq!(windows[1].pct, 7.3);
    assert_eq!(windows[2].label, "Weekly · Sonnet");
    assert!(extra_usage.is_none());
}

#[test]
fn unavailable_and_failed_answers() {
    assert_eq!(
        parse_get_usage(&json!({ "rate_limits_available": false, "rate_limits": null })),
        Ok(UsageAnswer::Unavailable)
    );
    assert!(
        parse_get_usage(&json!({ "rate_limits_available": true, "rate_limits": null })).is_err()
    );
    assert!(parse_get_usage(&json!({})).is_err());
    assert!(parse_get_usage(&Value::Null).is_err());
    // Rows of unknown kinds render tolerantly; garbage rows are skipped.
    let resp = json!({ "rate_limits": { "limits": [
        { "kind": "monthly_scoped", "group": "monthly", "percent": 5,
          "scope": { "surface": { "display_name": "Cowork" } } },
        { "kind": "", "percent": 5 },
        { "kind": "session", "percent": "lots" },
        "not an object"
    ] } });
    let UsageAnswer::Data { windows, .. } = parse_get_usage(&resp).unwrap() else {
        panic!("data expected");
    };
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].key, "monthly_scoped_cowork");
    assert_eq!(windows[0].label, "Monthly scoped · Cowork");
}

#[test]
fn extra_usage_is_tolerant() {
    assert!(extra_usage_from(Some(&json!({ "used_credits": 1 }))).is_none());
    let e = extra_usage_from(Some(&json!({ "is_enabled": false, "currency": "€€",
        "decimal_places": 9, "used_credits": -5, "utilization": "x" })))
    .unwrap();
    assert!(!e.enabled);
    assert_eq!(e.currency, None, "never a guessed currency");
    assert_eq!(e.decimals, 2);
    assert_eq!(e.used_minor, 0);
    assert_eq!(e.limit_minor, None);
    assert_eq!(e.utilization, None);
    // Only a 3-letter code is a currency; none sent is none shown.
    let currency = |c: Value| {
        extra_usage_from(Some(&json!({ "is_enabled": true, "currency": c })))
            .unwrap()
            .currency
    };
    assert_eq!(currency(json!(" eur ")).as_deref(), Some("EUR"));
    assert_eq!(currency(json!("EURO")), None);
    assert_eq!(currency(json!("U$D")), None);
    assert_eq!(currency(json!(978)), None);
    assert_eq!(currency(Value::Null), None);
    let v = serde_json::to_value(extra_usage_from(Some(&json!({ "is_enabled": true }))).unwrap())
        .unwrap();
    assert!(v.get("currency").is_none(), "absent, not null: {v}");
}

#[test]
fn rate_limit_events_are_rescaled() {
    let u = parse_rate_limit_event(&json!({
        "type": "rate_limit_event",
        "rate_limit_info": {
            "status": "allowed_warning", "resetsAt": 1_790_010_000, "rateLimitType": "five_hour",
            "utilization": 0.9, "isUsingOverage": false,
            "unifiedWindows": {
                "five_hour": { "utilization": 0.4237, "resetsAt": 1_790_010_000 },
                "seven_day": { "utilization": 1.05, "resetsAt": 1_790_400_000_000_i64 },
                "seven_day_overage_included": { "utilization": 0.1, "resetsAt": 1 }
            }
        }
    }))
    .unwrap();
    assert!(!u.rejected);
    assert_eq!(
        u.windows,
        vec![
            ("five_hour".to_string(), 42.4, Some(1_790_010_000_000)),
            ("seven_day".to_string(), 105.0, Some(1_790_400_000_000)),
        ]
    );
    // A rejection without windows is still a signal.
    let r = parse_rate_limit_event(&json!({ "type": "rate_limit_event",
        "rate_limit_info": { "status": "rejected" } }))
    .unwrap();
    assert!(r.rejected && r.windows.is_empty());
    assert!(parse_rate_limit_event(&json!({ "type": "rate_limit_event",
        "rate_limit_info": { "status": "allowed" } }))
    .is_none());
    assert!(parse_rate_limit_event(&json!({ "type": "assistant" })).is_none());
    assert!(parse_rate_limit_event(&json!({ "type": "rate_limit_event",
        "rate_limit_info": { "unifiedWindows": { "five_hour": { "utilization": "x" } } } }))
    .is_none());
}

fn claude_json(fetched_at: i64, theirs: &str, ours: Option<&str>) -> String {
    let mut root = json!({
        "numStartups": 41,
        "projects": { "/p": { "hasTrustDialogAccepted": true } },
        "cachedUsageUtilization": {
            "fetchedAtMs": fetched_at,
            "accountUuid": theirs,
            "utilization": full_answer()["rate_limits"].clone()
        }
    });
    if let Some(ours) = ours {
        root["oauthAccount"] = json!({ "accountUuid": ours, "emailAddress": "a@b.test" });
    }
    root.to_string()
}

#[test]
fn the_cli_cache_needs_the_same_account_and_an_hour() {
    let data = parse_cache(&claude_json(NOW - 10 * MIN, "acc-1", Some("acc-1")), NOW).unwrap();
    assert_eq!(data.source, PlanUsageSource::Cache);
    assert_eq!(data.fetched_at, NOW - 10 * MIN);
    assert_eq!(data.windows[0].key, "five_hour");
    assert!(data.extra_usage.is_some());
    // Another account's snapshot, no signed-in account, too old, garbage.
    assert!(parse_cache(&claude_json(NOW - MIN, "acc-2", Some("acc-1")), NOW).is_none());
    assert!(parse_cache(&claude_json(NOW - MIN, "acc-1", None), NOW).is_none());
    assert!(parse_cache(&claude_json(NOW - MIN, "", Some("")), NOW).is_none());
    assert!(parse_cache(&claude_json(NOW - 61 * MIN, "acc-1", Some("acc-1")), NOW).is_none());
    assert!(parse_cache("{not json", NOW).is_none());
    assert!(parse_cache("{}", NOW).is_none());
    // The account id is compared, never kept.
    assert!(!format!("{data:?}").contains("acc-1"));
}

fn base() -> Inputs {
    Inputs {
        now: NOW,
        interval_ms: 5 * MIN,
        clients: 1,
        last_read: Some(NOW - 6 * MIN),
        full_at: Some(NOW - 6 * MIN),
        ..Default::default()
    }
}

#[test]
fn the_scheduler_decision_table() {
    // Nobody connected: nothing, whatever else holds — but a manual refresh.
    let idle_everything = Inputs {
        clients: 0,
        activity: true,
        first_connect: true,
        limit_hit: true,
        full_at: None,
        ..base()
    };
    assert_eq!(decide(&idle_everything), None);
    assert_eq!(
        decide(&Inputs {
            manual: true,
            ..idle_everything
        }),
        Some(Reason::Manual)
    );
    // Never two reads within a minute, not even manual ones.
    let just_read = Inputs {
        last_read: Some(NOW - 59_000),
        manual: true,
        limit_hit: true,
        first_connect: true,
        ..base()
    };
    assert_eq!(decide(&just_read), None);
    // A limit hit reads at once (≥ 60 s), even while backing off.
    assert_eq!(
        decide(&Inputs {
            limit_hit: true,
            backoff_until: Some(NOW + 10 * MIN),
            ..base()
        }),
        Some(Reason::LimitHit)
    );
    // Backing off blocks everything else.
    assert_eq!(
        decide(&Inputs {
            activity: true,
            first_connect: true,
            backoff_until: Some(NOW + MIN),
            ..base()
        }),
        None
    );
    // Periodic: interval elapsed and some activity since the last full read.
    assert_eq!(decide(&base()), None, "idle: no read yet");
    assert_eq!(
        decide(&Inputs {
            activity: true,
            ..base()
        }),
        Some(Reason::Periodic)
    );
    let recent = Inputs {
        activity: true,
        full_at: Some(NOW - 4 * MIN),
        last_read: Some(NOW - 4 * MIN),
        ..base()
    };
    assert_eq!(decide(&recent), None, "interval not elapsed");
    // A fresher stream update covers the 5-hour/weekly windows…
    let streamed = Inputs {
        activity: true,
        stream_at: Some(NOW - MIN),
        ..base()
    };
    assert_eq!(decide(&streamed), None);
    // …but the full set is still read every 30 min.
    assert_eq!(
        decide(&Inputs {
            full_at: Some(NOW - 31 * MIN),
            last_read: Some(NOW - 31 * MIN),
            ..streamed
        }),
        Some(Reason::Idle)
    );
    // Idle: every 30 min.
    assert_eq!(
        decide(&Inputs {
            full_at: Some(NOW - 30 * MIN),
            last_read: Some(NOW - 30 * MIN),
            ..base()
        }),
        Some(Reason::Idle)
    );
    // No data at all with someone connected: read.
    assert_eq!(
        decide(&Inputs {
            full_at: None,
            last_read: None,
            ..base()
        }),
        Some(Reason::Idle)
    );
    // First connect: a stale full set is read, a fresh one is not…
    assert_eq!(
        decide(&Inputs {
            first_connect: true,
            ..base()
        }),
        Some(Reason::FirstConnect)
    );
    assert_eq!(
        decide(&Inputs {
            first_connect: true,
            full_at: Some(NOW - MIN),
            ..base()
        }),
        None
    );
    // …and a fresh stream update (two windows) does not make it fresh.
    assert_eq!(
        decide(&Inputs {
            first_connect: true,
            stream_at: Some(NOW - MIN),
            ..base()
        }),
        Some(Reason::FirstConnect)
    );
    // Manual only (interval 0): first connect (stale after 5 min), manual,
    // limit hits — never periodic or idle reads.
    let manual_only = Inputs {
        interval_ms: 0,
        activity: true,
        full_at: Some(NOW - 120 * MIN),
        last_read: Some(NOW - 120 * MIN),
        ..base()
    };
    assert_eq!(decide(&manual_only), None);
    assert_eq!(
        decide(&Inputs {
            first_connect: true,
            ..manual_only
        }),
        Some(Reason::FirstConnect)
    );
    assert_eq!(
        decide(&Inputs {
            first_connect: true,
            full_at: Some(NOW - 4 * MIN),
            ..manual_only
        }),
        None
    );
    assert_eq!(
        decide(&Inputs {
            limit_hit: true,
            ..manual_only
        }),
        Some(Reason::LimitHit)
    );
    // No plan usage for this account: hourly re-check, or on a new sign-in.
    let unavailable = Inputs {
        unavailable: true,
        unavailable_since: Some(NOW - 59 * MIN),
        activity: true,
        first_connect: true,
        ..base()
    };
    assert_eq!(decide(&unavailable), None);
    assert_eq!(
        decide(&Inputs {
            unavailable_since: Some(NOW - 60 * MIN),
            ..unavailable
        }),
        Some(Reason::Recheck)
    );
    assert_eq!(
        decide(&Inputs {
            auth_changed: true,
            ..unavailable
        }),
        Some(Reason::Recheck)
    );
    // Limit hits (throttled API-key or third-party requests raise them too)
    // never read for such an account: it waits for its re-check.
    let throttled = Inputs {
        limit_hit: true,
        unavailable_since: Some(NOW - 10 * MIN),
        ..unavailable
    };
    assert_eq!(decide(&throttled), None);
    assert_eq!(
        decide(&Inputs {
            auth_changed: true,
            ..throttled
        }),
        Some(Reason::Recheck)
    );
    assert_eq!(
        decide(&Inputs {
            unavailable_since: Some(NOW - 61 * MIN),
            ..throttled
        }),
        Some(Reason::Recheck)
    );
    // A Refresh the user asked for still reads.
    assert_eq!(
        decide(&Inputs {
            manual: true,
            ..throttled
        }),
        Some(Reason::Manual)
    );
}

#[test]
fn next_refresh_estimates() {
    // Manual only, nobody connected, unavailable: none.
    for i in [
        Inputs {
            interval_ms: 0,
            activity: true,
            ..base()
        },
        Inputs {
            clients: 0,
            activity: true,
            ..base()
        },
        Inputs {
            unavailable: true,
            activity: true,
            ..base()
        },
    ] {
        assert_eq!(next_due(&i), None);
    }
    // Idle: the 30-minute read (never inside a back-off).
    assert_eq!(next_due(&base()), Some(NOW + 24 * MIN));
    assert_eq!(
        next_due(&Inputs {
            backoff_until: Some(NOW + 40 * MIN),
            ..base()
        }),
        Some(NOW + 40 * MIN)
    );
    assert_eq!(
        next_due(&Inputs {
            full_at: Some(NOW - 45 * MIN),
            ..base()
        }),
        Some(NOW),
        "overdue"
    );
    let active = Inputs {
        activity: true,
        full_at: Some(NOW - 2 * MIN),
        last_read: Some(NOW - 2 * MIN),
        ..base()
    };
    assert_eq!(next_due(&active), Some(NOW + 3 * MIN));
    // A fresh stream update pushes it out (never past the 30-min read).
    assert_eq!(
        next_due(&Inputs {
            stream_at: Some(NOW - 30_000),
            ..active
        }),
        Some(NOW + 4 * MIN + 30_000)
    );
    assert_eq!(
        next_due(&Inputs {
            stream_at: Some(NOW + 40 * MIN),
            ..active
        }),
        Some(NOW + 28 * MIN)
    );
    // Overdue: now, but never inside the floor or a back-off.
    assert_eq!(
        next_due(&Inputs {
            full_at: Some(NOW - 20 * MIN),
            last_read: Some(NOW - 30_000),
            ..active
        }),
        Some(NOW + 30_000)
    );
    assert_eq!(
        next_due(&Inputs {
            full_at: Some(NOW - 20 * MIN),
            backoff_until: Some(NOW + 7 * MIN),
            ..active
        }),
        Some(NOW + 7 * MIN)
    );
    assert_eq!(
        next_due(&Inputs {
            full_at: None,
            ..active
        }),
        Some(NOW)
    );
}

fn state() -> AppState {
    crate::state::Inner::new(
        crate::claude::ClaudeHome::with_base(std::env::temp_dir().join("mc-usage-test-no-home")),
        crate::state::ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        crate::server::auth::Auth::ephemeral(),
    )
}

fn window(key: &str, pct: f64, resets_at: Option<i64>) -> PlanWindow {
    PlanWindow {
        key: key.into(),
        label: label_for(key),
        pct,
        resets_at,
        severity: Some(PlanSeverity::Normal),
    }
}

#[test]
fn a_window_past_its_reset_shows_zero() {
    let store = UsageStore::new(None);
    store.set_data_for_test(UsageData {
        windows: vec![
            window("five_hour", 97.0, Some(NOW - 1)),
            window("seven_day", 40.0, Some(NOW + 1)),
        ],
        extra_usage: None,
        fetched_at: NOW - 10 * MIN,
        source: PlanUsageSource::Live,
    });
    let u = store.snapshot(NOW);
    assert!(u.available);
    assert_eq!(u.windows[0].pct, 0.0);
    assert_eq!(u.windows[0].severity, None);
    assert_eq!(u.windows[0].resets_at, Some(NOW - 1));
    assert_eq!(u.windows[1].pct, 40.0);
    assert_eq!(u.fetched_at, Some(NOW - 10 * MIN));
    assert_eq!(u.source, Some(PlanUsageSource::Live));
    assert_eq!(u.interval_secs, DEFAULT_INTERVAL);
}

#[tokio::test]
async fn stream_updates_merge_into_the_store_and_publish() {
    let st = state();
    let now = now_ms();
    let mut rx = st.bus.subscribe();
    st.usage.set_data_for_test(UsageData {
        windows: vec![
            window("five_hour", 10.0, Some(now + 60 * MIN)),
            window("seven_day", 5.0, Some(now + 3 * 24 * 60 * MIN)),
            window("seven_day_fable", 2.0, None),
        ],
        extra_usage: None,
        fetched_at: now - 10 * MIN,
        source: PlanUsageSource::Live,
    });
    merge_stream(
        &st,
        StreamUpdate {
            windows: vec![("five_hour".into(), 12.5, Some(now + 61 * MIN))],
            rejected: false,
        },
    );
    let u = current(&st);
    assert_eq!(u.windows[0].pct, 12.5);
    assert_eq!(u.windows[0].resets_at, Some(now + 61 * MIN));
    assert_eq!(u.windows[0].severity, None, "the old severity went stale");
    assert_eq!(u.windows[1].severity, Some(PlanSeverity::Normal));
    assert_eq!(u.windows.len(), 3, "other windows are kept");
    // The full read keeps its time and source: the Fable window and extra
    // usage are not this fresh. `streamAt` says when the two were.
    assert_eq!(u.source, Some(PlanUsageSource::Live));
    assert_eq!(u.fetched_at, Some(now - 10 * MIN));
    let streamed = st.usage.lock().stream_at.unwrap();
    assert_eq!(u.stream_at, Some(streamed));
    let ev = rx.try_recv().unwrap();
    assert!(matches!(&*ev, crate::state::ServerEvent::PlanUsage { usage } if usage == &u));
    // A new window the store did not know yet gets its label.
    merge_stream(
        &st,
        StreamUpdate {
            windows: vec![("seven_day".into(), 6.0, None)],
            rejected: false,
        },
    );
    let u2 = current(&st);
    assert_eq!(u2.windows[1].pct, 6.0);
    assert_eq!(u2.windows[1].resets_at, Some(now + 3 * 24 * 60 * MIN));
    assert_eq!(u2.windows[1].label, "Weekly · all models");
    // A stream update proves plan usage exists.
    st.usage.lock().unavailable = true;
    merge_stream(
        &st,
        StreamUpdate {
            windows: vec![("five_hour".into(), 13.0, None)],
            rejected: false,
        },
    );
    assert!(current(&st).available);
    // A rejection flags a limit hit.
    merge_stream(
        &st,
        StreamUpdate {
            windows: vec![],
            rejected: true,
        },
    );
    assert!(st.usage.lock().limit_hit.is_some());
}

#[tokio::test]
async fn manual_refresh_is_throttled_to_one_a_minute() {
    let st = state();
    let first = request_refresh(&st).unwrap();
    assert!(first.refreshing);
    let wait = request_refresh(&st).unwrap_err();
    assert!((1..=60).contains(&wait), "{wait}");
    // After a real read, the floor counts from it.
    {
        let mut g = st.usage.lock();
        g.manual_at = None;
        g.refreshing = false;
        g.last_read = Some(now_ms() - 30_000);
    }
    let wait = request_refresh(&st).unwrap_err();
    assert!((29..=31).contains(&wait), "{wait}");
    st.usage.lock().last_read = Some(now_ms() - 61_000);
    assert!(request_refresh(&st).is_ok());
}

#[test]
fn the_interval_is_validated_and_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let store = UsageStore::new(Some(dir.path()));
    assert_eq!(store.interval_secs(), DEFAULT_INTERVAL);
    assert!(store.set_interval(120).is_err());
    store.set_interval(180).unwrap();
    assert_eq!(store.interval_secs(), 180);
    let text = std::fs::read_to_string(dir.path().join("usage.json")).unwrap();
    assert!(text.contains("\"intervalSecs\": 180"), "{text}");
    assert_eq!(UsageStore::new(Some(dir.path())).interval_secs(), 180);
    store.set_interval(0).unwrap();
    assert_eq!(UsageStore::new(Some(dir.path())).interval_secs(), 0);
    // A hand-edited bad value falls back to the default.
    std::fs::write(dir.path().join("usage.json"), r#"{"intervalSecs":7}"#).unwrap();
    assert_eq!(
        UsageStore::new(Some(dir.path())).interval_secs(),
        DEFAULT_INTERVAL
    );
    assert_eq!(
        helper_dir(dir.path()),
        dir.path().join("usage-helper"),
        "the helper runs in the app data dir"
    );
}

/// A fake `claude` for the helper: records argv and the environment it got,
/// answers `initialize` and `get_usage` like CLI 2.1.284, then lingers (the
/// helper must kill it, not wait for it).
#[cfg(unix)]
fn fake_cli(dir: &Path, usage_reply: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$@" > argv.txt
printf 'owned=%s claudecode=%s entry=%s\n' "${{MOTHER_CLAUDE_OWNED:-}}" "${{CLAUDECODE:-unset}}" "${{CLAUDE_CODE_ENTRYPOINT:-unset}}" > env.txt
while IFS= read -r line; do
  printf '%s\n' "$line" >> stdin.txt
  rid=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"subtype":"initialize"'*)
      printf '{{"type":"system","subtype":"noise"}}\n'
      printf '{{"type":"control_request","request_id":"cli-1","request":{{"subtype":"frobnicate"}}}}\n'
      printf '{{"type":"control_response","response":{{"subtype":"success","request_id":"%s","response":{{"commands":[]}}}}}}\n' "$rid" ;;
    *'"subtype":"get_usage"'*)
      printf '{usage_reply}\n' "$rid"
      exec sleep 30 ;;
  esac
done
"#
    );
    let path = dir.join("fake-claude.sh");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[tokio::test]
async fn the_helper_speaks_initialize_then_get_usage_and_is_terminated() {
    let tmp = tempfile::tempdir().unwrap();
    let helper = helper_dir(tmp.path());
    let ok = r#"{"type":"control_response","response":{"subtype":"success","request_id":"%s","response":{"rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":42,"resets_at":"2026-09-29T15:00:00.000000+00:00"},"amber_cistern":{"utilization":9}}}}}"#;
    let fake = fake_cli(tmp.path(), ok);
    let started = std::time::Instant::now();
    let resp = helper_read(fake.to_str().unwrap(), &helper).await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "terminated after the answer, not waited for"
    );
    let UsageAnswer::Data { windows, .. } = parse_get_usage(&resp).unwrap() else {
        panic!("data expected");
    };
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].pct, 42.0);

    // The helper directory is private.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&helper).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
    // Its argv: print mode, stream-json both ways, no tools, no MCP, project
    // settings only, nothing persisted.
    let argv: Vec<String> = std::fs::read_to_string(helper.join("argv.txt"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let expected: Vec<String> = HELPER_ARGS.iter().map(|s| s.to_string()).collect();
    assert_eq!(argv, expected);
    // Scrubbed and marked owned.
    let env = std::fs::read_to_string(helper.join("env.txt")).unwrap();
    assert!(env.contains("owned=1"), "{env}");
    assert!(env.contains("claudecode=unset"), "{env}");
    assert!(env.contains("entry=unset"), "{env}");
    // initialize first, then get_usage with skip_behaviors, and the CLI's
    // own request was answered `unsupported`.
    let sent: Vec<Value> = std::fs::read_to_string(helper.join("stdin.txt"))
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    assert_eq!(sent[0]["request"]["subtype"], "initialize");
    let reply = sent
        .iter()
        .find(|v| v["type"] == "control_response")
        .unwrap();
    assert_eq!(reply["response"]["request_id"], "cli-1");
    assert_eq!(reply["response"]["subtype"], "error");
    let usage = sent
        .iter()
        .find(|v| v["request"]["subtype"] == "get_usage")
        .unwrap();
    assert_eq!(usage["request"]["skip_behaviors"], true);
    assert_eq!(sent.len(), 3, "nothing else, no user message: {sent:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn helper_failures_are_short_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let err = r#"{"type":"control_response","response":{"subtype":"error","request_id":"%s","error":"get_usage: not signed in"}}"#;
    let fake = fake_cli(tmp.path(), err);
    let e = helper_read(fake.to_str().unwrap(), &helper_dir(tmp.path()))
        .await
        .unwrap_err();
    assert_eq!(e, "get_usage: not signed in");
    let e = helper_read("/definitely/not/a/claude", &helper_dir(tmp.path()))
        .await
        .unwrap_err();
    assert!(e.contains("Could not start"), "{e}");
    // A CLI that exits at once.
    let e = helper_read("true", &helper_dir(tmp.path()))
        .await
        .unwrap_err();
    assert!(e.contains("exited") || e.contains("Could not write"), "{e}");
}

#[test]
fn errors_are_one_short_line() {
    assert_eq!(short_error("boom\nat x.js:1"), "boom");
    assert_eq!(short_error(""), "Plan usage could not be read.");
    assert_eq!(short_error(&"x".repeat(500)).chars().count(), 200);
}

#[tokio::test]
async fn stream_only_data_and_later_snapshots_keep_honest_times() {
    let st = state();
    let now = now_ms();
    // No full read yet: the stream's windows are all there is.
    merge_stream(
        &st,
        StreamUpdate {
            windows: vec![
                ("five_hour".into(), 12.0, Some(now + 60 * MIN)),
                ("seven_day".into(), 3.0, Some(now + 600 * MIN)),
            ],
            rejected: false,
        },
    );
    let u = current(&st);
    assert_eq!(u.source, Some(PlanUsageSource::Stream));
    let first = u.fetched_at.unwrap();
    assert_eq!(u.stream_at, None, "nothing older to tell apart");
    // An update that leaves a window out does not make that one fresh.
    st.usage.lock().data.as_mut().unwrap().fetched_at = first - MIN;
    merge_stream(
        &st,
        StreamUpdate {
            windows: vec![("five_hour".into(), 13.0, None)],
            rejected: false,
        },
    );
    let u = current(&st);
    assert_eq!(u.fetched_at, Some(first - MIN));
    assert!(u.stream_at.is_some());
    // Both again: all of it is this fresh.
    merge_stream(
        &st,
        StreamUpdate {
            windows: vec![
                ("five_hour".into(), 14.0, None),
                ("seven_day".into(), 4.0, None),
            ],
            rejected: false,
        },
    );
    let u = current(&st);
    assert_eq!(u.fetched_at, Some(st.usage.lock().stream_at.unwrap()));
    assert_eq!(u.stream_at, None);

    // The CLI's snapshot, taken before the last stream update: adopted (it
    // is the only full set), with the streamed windows kept as streamed.
    let stream_at = st.usage.lock().stream_at.unwrap();
    let snapshot = |at: i64| UsageData {
        windows: vec![
            window("five_hour", 9.0, Some(now + 60 * MIN)),
            window("seven_day", 1.0, Some(now + 600 * MIN)),
            window("seven_day_fable", 7.0, None),
        ],
        extra_usage: None,
        fetched_at: at,
        source: PlanUsageSource::Cache,
    };
    adopt_snapshot(&mut st.usage.lock(), snapshot(stream_at - MIN));
    let u = current(&st);
    assert_eq!(u.source, Some(PlanUsageSource::Cache));
    assert_eq!(u.fetched_at, Some(stream_at - MIN));
    assert_eq!(u.stream_at, Some(stream_at));
    let pct: Vec<f64> = u.windows.iter().map(|w| w.pct).collect();
    assert_eq!(pct, vec![14.0, 4.0, 7.0]);
    assert_eq!(u.windows[0].resets_at, Some(now + 60 * MIN));
    assert_eq!(st.usage.lock().full_at, Some(stream_at - MIN));
    // An older snapshot than the last full set is not taken…
    adopt_snapshot(&mut st.usage.lock(), snapshot(stream_at - 2 * MIN));
    assert_eq!(current(&st).fetched_at, Some(stream_at - MIN));
    // …a newer one than the stream is taken as it is.
    adopt_snapshot(&mut st.usage.lock(), snapshot(stream_at + MIN));
    let u = current(&st);
    assert_eq!(u.windows[0].pct, 9.0);
    assert_eq!(u.stream_at, None);
    // After a live read the stream counts from there again.
    apply_result(
        &mut st.usage.lock(),
        Ok(Some(UsageAnswer::Data {
            windows: vec![window("five_hour", 20.0, None)],
            extra_usage: None,
        })),
        stream_at + 2 * MIN,
    );
    let u = current(&st);
    assert_eq!(u.source, Some(PlanUsageSource::Live));
    assert_eq!(u.stream_at, None);
}

#[test]
fn read_results_are_booked() {
    let mut g = Inner {
        refreshing: true,
        ..Default::default()
    };
    // Failures back off 5 → 10 → 20 → 40 → 60 → 60 min.
    for (n, minutes) in [5, 10, 20, 40, 60, 60].into_iter().enumerate() {
        g.refreshing = true;
        assert!(apply_result(&mut g, Err("boom".into()), NOW));
        assert!(!g.refreshing);
        assert_eq!(g.failures, n as u32 + 1);
        assert_eq!(g.backoff_until, Some(NOW + minutes * MIN));
        assert_eq!(g.error.as_deref(), Some("boom"));
    }
    // Data resets all of it.
    let data = || {
        Ok(Some(UsageAnswer::Data {
            windows: vec![window("five_hour", 5.0, None)],
            extra_usage: None,
        }))
    };
    g.unavailable = true;
    assert!(!apply_result(&mut g, data(), NOW));
    assert_eq!(
        (g.failures, g.backoff_until, g.error.as_deref()),
        (0, None, None)
    );
    assert!(!g.unavailable);
    assert_eq!(g.full_at, Some(NOW));
    let d = g.data.as_ref().unwrap();
    assert_eq!((d.fetched_at, d.source), (NOW, PlanUsageSource::Live));
    // Nothing conclusive changes nothing.
    assert!(!apply_result(&mut g, Ok(None), NOW + MIN));
    assert_eq!(g.full_at, Some(NOW));
    assert!(g.data.is_some());
    // No plan usage: the data goes, and so do the error and the back-off.
    apply_result(&mut g, Err("again".into()), NOW + MIN);
    assert!(!apply_result(
        &mut g,
        Ok(Some(UsageAnswer::Unavailable)),
        NOW + 2 * MIN
    ));
    assert!(g.unavailable);
    assert_eq!(g.unavailable_since, Some(NOW + 2 * MIN));
    assert!(g.data.is_none());
    assert_eq!(
        (g.failures, g.backoff_until, g.error.as_deref()),
        (0, None, None)
    );
    assert!(!snapshot_of(&g, NOW).available);
}

#[test]
fn quota_hits_and_sign_ins_are_noted() {
    let mut g = Inner {
        quota_seen: NOW - 30 * MIN,
        ..Default::default()
    };
    // A new transcript rejection within ten minutes asks for a read, once.
    note_quota(&mut g, Some(NOW - 2 * MIN), NOW);
    assert_eq!(g.limit_hit, Some(NOW));
    assert_eq!(g.quota_seen, NOW - 2 * MIN);
    g.limit_hit = None;
    note_quota(&mut g, Some(NOW - 2 * MIN), NOW + MIN);
    assert_eq!(g.limit_hit, None, "already acted on");
    // An old one is only remembered.
    note_quota(&mut g, Some(NOW - 1), NOW + 20 * MIN);
    assert_eq!(g.limit_hit, None);
    assert_eq!(g.quota_seen, NOW - 1);
    // A limit hit nobody acted on expires.
    g.limit_hit = Some(NOW);
    note_quota(&mut g, None, NOW + 10 * MIN);
    assert!(g.limit_hit.is_some());
    note_quota(&mut g, None, NOW + 11 * MIN);
    assert_eq!(g.limit_hit, None);

    let auth = |provider: &str, email: &str| crate::claude::AuthStatus {
        logged_in: true,
        api_provider: Some(provider.into()),
        email: Some(email.into()),
        ..Default::default()
    };
    let mut g = Inner::default();
    // The first sign-in seen is no change; its email is fingerprinted.
    assert!(!note_auth(
        &mut g,
        &auth("firstParty", "Me@Example.test"),
        NOW
    ));
    assert!(!g.unavailable);
    assert_eq!(g.account_key, email_key("me@example.test"));
    assert!(!note_auth(
        &mut g,
        &auth("firstParty", "Me@Example.test"),
        NOW
    ));
    // A third-party provider has no plan usage (no read to find out).
    assert!(!note_auth(
        &mut g,
        &auth("bedrock", "Me@Example.test"),
        NOW + MIN
    ));
    assert!(g.unavailable);
    assert_eq!(g.unavailable_since, Some(NOW + MIN));
    // Signing in again re-opens the question.
    assert!(note_auth(
        &mut g,
        &auth("firstParty", "other@example.test"),
        NOW + 2 * MIN
    ));
    assert!(!g.unavailable);
    assert_eq!(g.account_key, email_key("other@example.test"));
    // Nothing readable is kept.
    assert!(!format!("{g:?}").contains("example"));
}

#[test]
fn the_helper_runs_on_its_own_only_while_reads_are_automatic() {
    assert!(helper_allowed(Some(Reason::Manual), 0));
    for reason in [
        Reason::FirstConnect,
        Reason::LimitHit,
        Reason::Recheck,
        Reason::Idle,
        Reason::Periodic,
    ] {
        assert!(!helper_allowed(Some(reason), 0), "{reason:?}");
        assert!(helper_allowed(Some(reason), 300_000), "{reason:?}");
    }
}

#[test]
fn owned_sessions_count_only_on_the_machines_plan() {
    let ours = email_key("me@example.test");
    let facts = |not_plan: bool, email: Option<&str>| {
        Some(AccountFacts {
            not_plan,
            email_key: email.and_then(email_key),
        })
    };
    assert!(account_ok(None, ours), "an older CLI reports no account");
    assert!(account_ok(facts(false, Some("ME@example.test ")), ours));
    assert!(account_ok(facts(false, None), ours));
    assert!(account_ok(facts(false, Some("x@example.test")), None));
    // Another account's windows, or no plan at all (an API key, Bedrock).
    assert!(!account_ok(facts(false, Some("x@example.test")), ours));
    assert!(!account_ok(facts(true, Some("me@example.test")), ours));
    assert_eq!(email_key("  "), None);
}

#[test]
fn frames_leave_in_the_order_they_are_recorded() {
    let st = state();
    let mut rx = st.bus.subscribe();
    std::thread::scope(|s| {
        for t in 0..8 {
            let st = &st;
            s.spawn(move || {
                for i in 0..50 {
                    st.usage.lock().error = Some(format!("{t}-{i}"));
                    publish(st);
                }
            });
        }
    });
    let mut last = None;
    loop {
        match rx.try_recv() {
            Ok(ev) => last = Some(ev),
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    let recorded = st.usage.lock().last_published.clone().unwrap();
    assert!(
        matches!(&*last.unwrap(), crate::state::ServerEvent::PlanUsage { usage } if *usage == recorded),
        "the last frame out is the one recorded"
    );
}

/// A fake `claude` that writes a pidfile like the CLI, answers `initialize`,
/// never answers `get_usage`, and removes the pidfile when terminated.
#[cfg(unix)]
fn hanging_cli(dir: &Path, sessions: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        r#"#!/bin/sh
mkdir -p '{sessions}'
printf '{{}}' > '{sessions}'/$$.json
printf '%s' $$ > pid.txt
trap 'kill $! 2>/dev/null; rm -f '"'"'{sessions}'"'"'/$$.json; exit 0' TERM
while IFS= read -r line; do
  rid=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"subtype":"initialize"'*)
      printf '{{"type":"control_response","response":{{"subtype":"success","request_id":"%s","response":{{}}}}}}\n' "$rid" ;;
    *'"subtype":"get_usage"'*)
      sleep 30 &
      wait $! ;;
  esac
done
"#,
        sessions = sessions.display()
    );
    let path = dir.join("hanging-claude.sh");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[tokio::test]
async fn a_helper_mid_read_is_stopped_cleanly_when_the_app_quits() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join("claude-home/sessions");
    let helper = helper_dir(tmp.path());
    let fake = hanging_cli(tmp.path(), &sessions);
    let bin = fake.to_str().unwrap().to_string();
    let dir = helper.clone();
    let read = tokio::spawn(async move { helper_read(&bin, &dir).await });
    // Mid-read: its pidfile exists and it is registered.
    let mut pid = None;
    for _ in 0..200 {
        pid = std::fs::read_to_string(helper.join("pid.txt"))
            .ok()
            .and_then(|p| p.trim().parse::<u32>().ok())
            .filter(|p| sessions.join(format!("{p}.json")).exists());
        if pid.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let pid = pid.expect("the fake started");
    assert!(helpers().contains(&pid));
    // Quitting: SIGTERM, and the wait ends once its pidfile is gone.
    let started = std::time::Instant::now();
    tokio::task::spawn_blocking({
        let sessions = sessions.clone();
        move || stop_helpers(&[pid], &sessions)
    })
    .await
    .unwrap();
    assert!(
        started.elapsed() < SHUTDOWN_GRACE,
        "{:?}",
        started.elapsed()
    );
    assert!(!sessions.join(format!("{pid}.json")).exists());
    // The read ends with the helper, which is then unregistered.
    let e = read.await.unwrap().unwrap_err();
    assert!(e.contains("exited"), "{e}");
    for _ in 0..200 {
        if !helpers().contains(&pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(!helpers().contains(&pid));
}

#[tokio::test]
async fn steps_consume_first_connect_and_a_refresh_reads_without_clients() {
    let st = state();
    on_client_connected(&st, 1);
    assert!(st.usage.lock().first_connect);
    // Nobody is connected any more (the hub counts none): no read, and the
    // first-connect signal is spent.
    step(&st).await;
    {
        let g = st.usage.lock();
        assert!(!g.first_connect);
        assert_eq!(g.last_read, None);
        assert_eq!(g.next_refresh_at, None);
    }
    // A Refresh reads anyway. Here it fails (no app data directory for the
    // helper): the error shows and the back-off starts.
    assert!(request_refresh(&st).unwrap().refreshing);
    step(&st).await;
    let g = st.usage.lock();
    assert!(g.last_read.is_some());
    assert!(!g.refreshing && !g.manual);
    assert_eq!(g.failures, 1);
    assert!(g.backoff_until.is_some());
    assert!(
        g.error
            .as_deref()
            .is_some_and(|e| e.contains("app data directory")),
        "{:?}",
        g.error
    );
}
