//! Admission control with synthetic signals: each rule and its reason, the
//! queue the Capacity page reads, and the plan's Phase 3 exit criterion as a
//! simulation (12 tasks across 4 roles never exceed the user cap or admit
//! workers above 85 % of the 5-hour window; two failed gates escalate a task
//! with the reason recorded).

use super::*;
use crate::claude::model::{PlanUsageSource, PlanWindow};
use crate::claude::usage::UsageData;
use crate::orchestration::host::{SessionMemory, GIB, MIB};
use crate::orchestration::metrics::{TaskOutcome, TaskResult};
use crate::orchestration::outcomes::Learned;
use crate::orchestration::policy::Policy;
use crate::orchestration::router::{self, ModelChoice, RouteSignals, TaskClass, TaskFacts};
use crate::orchestration::scheduler::{note_rate_limit_event, parse_rate_limit_event};
use serde_json::json;

const T0: i64 = 1_790_000_000_000;
const MIN: i64 = 60_000;

fn req(project: &str, role: &str, priority: u8, lead: bool) -> StartRequest {
    StartRequest {
        project: project.into(),
        role: role.into(),
        model: "sonnet".into(),
        priority,
        account: None,
        lead,
    }
}

fn run(id: &str, project: &str, role: &str, lead: bool) -> RunningWorker {
    RunningWorker {
        session_id: id.into(),
        project: project.into(),
        role: role.into(),
        model: "sonnet".into(),
        lead,
    }
}

fn plenty(now: i64) -> Signals {
    Signals {
        now,
        memory: Some(MemoryFacts {
            available: 16 * GIB,
            reserve: 2 * GIB,
            estimate: 600 * MIB,
            measured: false,
        }),
        five_hour: Some((10.0, None)),
        rate: None,
        budget: None,
    }
}

fn reasons(v: &Verdict) -> Vec<String> {
    match &v.admission {
        Admission::Admit => Vec::new(),
        Admission::Wait { reasons } => reasons.clone(),
    }
}

fn workers(n: usize) -> Vec<RunningWorker> {
    (0..n)
        .map(|i| {
            run(
                &format!("w{i}"),
                "/Users/me/app",
                &format!("role{i}"),
                false,
            )
        })
        .collect()
}

#[test]
fn your_cap_counts_workers_and_never_the_lead() {
    let l = Limits::default();
    let s = plenty(T0);
    let v = decide(
        &l,
        &s,
        &req("/Users/me/app", "backend", 1, false),
        &workers(3),
        &[],
    );
    assert_eq!(
        reasons(&v),
        ["3 role sessions run already (your limit is 3)"]
    );
    assert!(v.slot_bound);
    assert!(decide(
        &l,
        &s,
        &req("/Users/me/app", "ceo", 0, true),
        &workers(3),
        &[]
    )
    .admission
    .is_admit());
    let mut running = workers(2);
    running.push(run("lead", "/Users/me/app", "ceo", true));
    assert!(
        decide(&l, &s, &req("/Users/me/app", "x", 1, false), &running, &[])
            .admission
            .is_admit()
    );
    let one = Limits {
        user_cap: 1,
        ..Limits::default()
    };
    let v = decide(
        &one,
        &s,
        &req("/Users/me/app", "x", 1, false),
        &workers(1),
        &[],
    );
    assert_eq!(
        reasons(&v),
        ["1 role session run already (your limit is 1)"]
    );
}

#[test]
fn the_five_hour_window_admits_only_the_lead_then_nothing() {
    let l = Limits::default();
    let mut s = plenty(T0);
    s.five_hour = Some((91.0, Some(T0 + 42 * MIN)));
    let v = decide(&l, &s, &req("/Users/me/app", "backend", 1, false), &[], &[]);
    assert_eq!(
        reasons(&v),
        ["the 5-hour usage window is at 91 % (at 85 % or more only the lead starts, it resets in 42 min)"]
    );
    assert!(!v.slot_bound, "a freed slot does not help");
    assert!(
        decide(&l, &s, &req("/Users/me/app", "ceo", 0, true), &[], &[])
            .admission
            .is_admit()
    );
    s.five_hour = Some((85.0, None));
    assert!(
        !decide(&l, &s, &req("/Users/me/app", "b", 1, false), &[], &[])
            .admission
            .is_admit()
    );
    s.five_hour = Some((95.5, None));
    let v = decide(&l, &s, &req("/Users/me/app", "ceo", 0, true), &[], &[]);
    assert_eq!(
        reasons(&v),
        ["the 5-hour usage window is at 95.5 % (nothing starts at 95 % or more)"]
    );
    s.five_hour = Some((84.9, None));
    assert!(
        decide(&l, &s, &req("/Users/me/app", "b", 1, false), &[], &[])
            .admission
            .is_admit()
    );
}

#[test]
fn memory_headroom_waits_but_a_lone_session_always_starts() {
    let l = Limits::default();
    let mut s = plenty(T0);
    s.memory = Some(MemoryFacts {
        available: GIB + GIB / 10,
        reserve: 2 * GIB,
        estimate: 600 * MIB,
        measured: false,
    });
    assert!(
        decide(&l, &s, &req("/Users/me/app", "ceo", 0, true), &[], &[])
            .admission
            .is_admit()
    );
    let v = decide(
        &l,
        &s,
        &req("/Users/me/app", "b", 1, false),
        &workers(1),
        &[],
    );
    assert_eq!(
        reasons(&v),
        ["only 1.1 GB of memory free (a role session needs about 600 MB, and 2.0 GB stay free)"]
    );
    assert!(v.slot_bound);
    // The lead waits for memory too once others run.
    let v = decide(
        &l,
        &s,
        &req("/Users/me/app", "ceo", 0, true),
        &workers(1),
        &[],
    );
    assert!(!v.admission.is_admit());
    // 3.5 GB free with 2 GB reserved: 2 more of 600 MB (measured).
    s.memory = Some(MemoryFacts {
        available: 3 * GIB + GIB / 2,
        reserve: 2 * GIB,
        estimate: 600 * MIB,
        measured: true,
    });
    assert!(decide(
        &l,
        &s,
        &req("/Users/me/app", "b", 1, false),
        &workers(2),
        &[]
    )
    .admission
    .is_admit());
    // Unsampled: the memory rule does not hold anything up.
    s.memory = None;
    assert!(decide(
        &l,
        &s,
        &req("/Users/me/app", "b", 1, false),
        &workers(2),
        &[]
    )
    .admission
    .is_admit());
    // The facts come from the sample and the settings.
    let sample = HostSample {
        sampled_at: T0,
        total_memory: 24 * GIB,
        available_memory: 5 * GIB,
        sessions: vec![SessionMemory {
            session_id: "s".into(),
            pid: 1,
            rss_bytes: 450 * MIB,
        }],
        ..Default::default()
    };
    let f = memory_facts(&sample, &l).unwrap();
    assert_eq!((f.estimate, f.measured), (450 * MIB, true));
    let fixed = Limits {
        session_mb: Some(1000),
        ..Limits::default()
    };
    assert_eq!(memory_facts(&sample, &fixed).unwrap().estimate, 1000 * MIB);
    assert!(memory_facts(&HostSample::default(), &l).is_none());
}

#[test]
fn a_rate_limit_rejection_stops_everyone_and_a_warning_the_workers() {
    let l = Limits::default();
    let mut s = plenty(T0);
    s.rate = parse_rate_limit_event(
        "default",
        &json!({ "type": "rate_limit_event", "rate_limit_info": {
            "status": "rejected", "rateLimitType": "five_hour",
            "resetsAt": (T0 + 75 * MIN) / 1000 } }),
        T0 - 2 * MIN,
    );
    let v = decide(&l, &s, &req("/Users/me/app", "ceo", 0, true), &[], &[]);
    assert_eq!(
        reasons(&v),
        ["Claude refused a request 2 min ago on the 5-hour limit (it resets in 1 h 15 min)"]
    );
    // After the reset it no longer holds.
    s.now = T0 + 76 * MIN;
    assert!(
        decide(&l, &s, &req("/Users/me/app", "ceo", 0, true), &[], &[])
            .admission
            .is_admit()
    );
    // Without a reset time: five minutes.
    s.rate = Some(RateLimitNote {
        at: T0,
        account: "default".into(),
        status: "rejected".into(),
        limit: None,
        resets_at: None,
        pct: None,
        source: "hook",
    });
    s.now = T0 + 4 * MIN;
    let v = decide(&l, &s, &req("/Users/me/app", "b", 1, false), &[], &[]);
    assert_eq!(
        reasons(&v),
        ["Claude refused a request 4 min ago on a usage limit"]
    );
    s.now = T0 + 6 * MIN;
    assert!(
        decide(&l, &s, &req("/Users/me/app", "b", 1, false), &[], &[])
            .admission
            .is_admit()
    );
    // A warning about the 5-hour limit holds workers, not the lead.
    s.rate = parse_rate_limit_event(
        "default",
        &json!({ "type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed_warning", "rateLimitType": "five_hour", "utilization": 0.9 } }),
        T0,
    );
    s.now = T0 + MIN;
    let v = decide(&l, &s, &req("/Users/me/app", "b", 1, false), &[], &[]);
    assert_eq!(
        reasons(&v),
        ["Claude warned 1 min ago that the 5-hour limit is near; only the lead starts for 5 minutes"]
    );
    assert!(
        decide(&l, &s, &req("/Users/me/app", "ceo", 0, true), &[], &[])
            .admission
            .is_admit()
    );
    // A weekly warning feeds the router, not admission.
    s.rate.as_mut().unwrap().limit = Some("seven_day".into());
    assert!(
        decide(&l, &s, &req("/Users/me/app", "b", 1, false), &[], &[])
            .admission
            .is_admit()
    );
}

#[test]
fn a_project_cap_and_a_spent_budget_hold_workers_of_that_project() {
    let l = Limits::default()
        .patched(&json!({ "projectCaps": { "/Users/me/app": 2 } }))
        .unwrap();
    let s = plenty(T0);
    let running = vec![
        run("a", "/Users/me/app", "frontend", false),
        run("b", "/Users/me/app/", "backend", false),
    ];
    let v = decide(
        &l,
        &s,
        &req("/Users/me/app", "scout", 1, false),
        &running,
        &[],
    );
    assert_eq!(
        reasons(&v),
        ["2 role sessions run in app already (its limit is 2)"]
    );
    assert!(decide(
        &l,
        &s,
        &req("/Users/me/site", "scout", 1, false),
        &running,
        &[]
    )
    .admission
    .is_admit());
    let mut s = plenty(T0);
    s.budget = Some((20.4, 20.0));
    let v = decide(&l, &s, &req("/Users/me/site", "scout", 1, false), &[], &[]);
    assert_eq!(
        reasons(&v),
        ["site's daily budget is spent ($20.40 of $20.00, estimated)"]
    );
    assert!(
        decide(&l, &s, &req("/Users/me/site", "ceo", 0, true), &[], &[])
            .admission
            .is_admit()
    );
    s.budget = Some((19.99, 20.0));
    assert!(
        decide(&l, &s, &req("/Users/me/site", "scout", 1, false), &[], &[])
            .admission
            .is_admit()
    );
}

#[test]
fn every_reason_that_holds_is_listed() {
    let l = Limits::default();
    let mut s = plenty(T0);
    s.five_hour = Some((90.0, None));
    s.budget = Some((30.0, 20.0));
    let v = decide(
        &l,
        &s,
        &req("/Users/me/app", "b", 1, false),
        &workers(3),
        &[],
    );
    let r = reasons(&v);
    assert_eq!(r.len(), 3, "{r:?}");
    assert!(r[0].starts_with("the 5-hour usage window"));
    assert!(r[1].starts_with("3 role sessions run already"));
    assert!(r[2].starts_with("app's daily budget is spent"));
    assert!(!v.slot_bound);
}

fn waiting(project: &str, role: &str, priority: u8, lead: bool, at: i64) -> QueueEntry {
    QueueEntry {
        project: project.into(),
        role: role.into(),
        model: "sonnet".into(),
        priority,
        lead,
        account: None,
        admitted: false,
        reasons: vec!["3 role sessions run already (your limit is 3)".into()],
        waiting_since: Some(at),
        at,
        asks: 1,
        slot_bound: true,
    }
}

#[test]
fn a_worker_leaves_a_free_slot_to_a_higher_priority_waiter() {
    let l = Limits::default();
    let s = plenty(T0);
    let running = workers(2); // one slot left
    let ahead = vec![waiting("/Users/me/app", "reviewer", 0, false, T0 - 10_000)];
    let v = decide(
        &l,
        &s,
        &req("/Users/me/app", "scout", 2, false),
        &running,
        &ahead,
    );
    assert_eq!(
        reasons(&v),
        ["reviewer in app (P0) waits for the same slot and goes first"]
    );
    // The waiter itself, asking again, is admitted.
    assert!(decide(
        &l,
        &s,
        &req("/Users/me/app", "reviewer", 0, false),
        &running,
        &ahead
    )
    .admission
    .is_admit());
    // Same priority does not yield; two slots for one waiter do not either.
    assert!(decide(
        &l,
        &s,
        &req("/Users/me/app", "scout", 0, false),
        &running,
        &ahead
    )
    .admission
    .is_admit());
    assert!(decide(
        &l,
        &s,
        &req("/Users/me/app", "scout", 2, false),
        &workers(1),
        &ahead
    )
    .admission
    .is_admit());
    // A waiter that stopped asking does not hold the slot.
    let stale = vec![waiting("/Users/me/app", "reviewer", 0, false, T0 - 2 * MIN)];
    assert!(decide(
        &l,
        &s,
        &req("/Users/me/app", "scout", 2, false),
        &running,
        &stale
    )
    .admission
    .is_admit());
    // A waiting lead goes ahead of any worker.
    let lead = vec![waiting("/Users/me/site", "ceo", 3, true, T0 - 1_000)];
    let v = decide(
        &l,
        &s,
        &req("/Users/me/app", "scout", 0, false),
        &running,
        &lead,
    );
    assert_eq!(
        reasons(&v),
        ["the lead of site waits for the same slot and goes first"]
    );
    // The lead never yields.
    assert!(decide(
        &l,
        &s,
        &req("/Users/me/app", "ceo", 0, true),
        &running,
        &ahead
    )
    .admission
    .is_admit());
}

#[test]
fn the_queue_keeps_the_last_decision_and_forgets_stale_ones() {
    let mut q = Queue::default();
    let wait = Verdict {
        admission: Admission::Wait {
            reasons: vec!["x".into()],
        },
        slot_bound: true,
    };
    let admit = Verdict {
        admission: Admission::Admit,
        slot_bound: false,
    };
    let r = req("/Users/me/app", "backend", 1, false);
    q.remember(&r, "default", &wait, T0);
    q.remember(&r, "default", &wait, T0 + MIN);
    let e = &q.entries()[0];
    assert_eq!((e.asks, e.waiting_since, e.at), (2, Some(T0), T0 + MIN));
    assert!(!e.admitted && e.slot_bound);
    // Another role waits too; waiting entries sort first, longest first.
    q.remember(
        &req("/Users/me/app", "scout", 2, false),
        "default",
        &wait,
        T0 + 2 * MIN,
    );
    q.remember(&r, "default", &admit, T0 + 3 * MIN);
    let entries = q.entries();
    assert_eq!(entries[0].role, "scout");
    assert!(entries[1].admitted && entries[1].reasons.is_empty());
    assert_eq!(entries[1].waiting_since, None);
    // A started entry shows two minutes, a waiting one ten.
    q.forget_stale(T0 + 3 * MIN + STARTED_TTL_MS + 1, |_| None);
    assert_eq!(q.entries().len(), 1);
    q.forget_stale(T0 + 2 * MIN + WAIT_TTL_MS + 1, |_| None);
    assert!(q.entries().is_empty());
    // Role sessions: registered with what they had spent, gone when ended.
    let sid = "11111111-2222-3333-4444-555555555555";
    q.note_running(
        &[
            run(sid, "/Users/me/app", "backend", false),
            run("nope", "/p", "r", false),
        ],
        T0,
        |_| Some(1.25),
    );
    let roles = q.roles();
    assert_eq!(roles.len(), 1, "an id that is not a session id is not kept");
    assert_eq!(roles[0].baseline_usd, Some(1.25));
    q.forget_stale(T0 + 30_000, |_| Some(false));
    assert_eq!(q.roles().len(), 1, "ended a moment ago: kept a minute");
    q.forget_stale(T0 + 2 * MIN, |_| Some(false));
    assert!(q.roles().is_empty());
}

#[test]
fn labels_read_naturally() {
    assert_eq!(bytes_label(GIB + GIB / 10), "1.1 GB");
    assert_eq!(bytes_label(600 * MIB), "600 MB");
    assert_eq!(span_label(10_000), "less than a minute");
    assert_eq!(span_label(4 * MIN), "4 min");
    assert_eq!(span_label(60 * MIN), "1 h");
    assert_eq!(span_label(65 * MIN), "1 h 5 min");
    assert_eq!(project_name("/Users/me/mother-claude/"), "mother-claude");
    assert_eq!(project_name("C:\\dev\\app"), "app");
    assert_eq!(limit_label(Some("seven_day_opus")), "the Opus weekly limit");
}

// ─────────────────────────── the Phase 3 exit criterion ───────────────────────

struct SimTask {
    id: &'static str,
    role: &'static str,
    class: TaskClass,
    priority: u8,
    minutes: i64,
    /// Gates it fails before it passes.
    gate_failures: u8,
    escalation: u8,
    done: bool,
}

/// 12 tasks across 4 roles (a lead and three workers), one session per
/// role, your cap at 2 so it binds; the 5-hour window climbs with the work
/// and starts over after five hours; Claude rejects a request once and warns
/// once. At every decision: never more workers than your cap, no worker
/// admitted at or above 85 % of the window, nothing at or above 95 %, and
/// nothing while a rejection holds. Two failed gates escalate a task with
/// the reason recorded, and the router then routes it a step up.
#[test]
fn phase_3_exit_criterion_a_synthetic_load_of_12_tasks_across_4_roles() {
    const PROJECT: &str = "/Users/me/mother-claude";
    let limits = Limits {
        user_cap: 2,
        ..Limits::default()
    };
    let policy = Policy::default();
    let mut learned = Learned::default();
    let mut tasks: Vec<SimTask> = [
        ("p1", "ceo", TaskClass::Plan, 0, 30, 0),
        ("p2", "ceo", TaskClass::Review, 0, 25, 0),
        ("p3", "ceo", TaskClass::Review, 1, 20, 0),
        ("f1", "frontend", TaskClass::Implement, 1, 40, 0),
        ("f2", "frontend", TaskClass::Test, 2, 25, 0),
        ("f3", "frontend", TaskClass::Fix, 1, 20, 0),
        ("b1", "backend", TaskClass::Implement, 1, 35, 0),
        ("b2", "backend", TaskClass::Implement, 0, 30, 2),
        ("b3", "backend", TaskClass::Test, 2, 20, 0),
        ("s1", "scout", TaskClass::Search, 2, 15, 0),
        ("s2", "scout", TaskClass::Summarize, 3, 15, 0),
        ("s3", "scout", TaskClass::Search, 2, 10, 0),
    ]
    .into_iter()
    .map(
        |(id, role, class, priority, minutes, gate_failures)| SimTask {
            id,
            role,
            class,
            priority,
            minutes,
            gate_failures,
            escalation: 0,
            done: false,
        },
    )
    .collect();
    assert_eq!(tasks.len(), 12);
    let roles = ["ceo", "frontend", "backend", "scout"];

    // (task index, the session, its end)
    let mut running: Vec<(usize, RunningWorker, i64)> = Vec::new();
    let mut queue = Queue::default();
    let mut five_hour = 50.0_f64;
    let mut resets_at = T0 + 150 * MIN;
    let mut rate: Option<RateLimitNote> = None;
    let mut max_workers = 0;
    let mut waits_on = std::collections::BTreeSet::new();
    let mut escalation_reason = None;
    let mut escalated_route = None;
    let mut session_seq = 0;
    let (mut rejected_sent, mut warned) = (false, false);

    for step in 0..900 {
        let now = T0 + step * MIN;
        // The window climbs with the work, and starts over.
        let workers_now = running.iter().filter(|(_, w, _)| !w.lead).count();
        let leads_now = running.len() - workers_now;
        five_hour += 0.45 * workers_now as f64 + 0.1 * leads_now as f64;
        if now >= resets_at {
            five_hour = 5.0;
            resets_at += 300 * MIN;
        }
        // Tasks end: a failed gate sends the task back; two escalate it.
        let (ended, still): (Vec<_>, Vec<_>) =
            running.drain(..).partition(|(_, _, end)| *end <= now);
        running = still;
        for (i, w, _) in ended {
            let t = &mut tasks[i];
            let failed_gate = t.gate_failures > 0;
            let outcome = TaskOutcome {
                task_id: t.id.into(),
                project: PROJECT.into(),
                role: Some(t.role.into()),
                class: Some(t.class),
                choice: ModelChoice::new(&w.model, None),
                session_id: Some(w.session_id.clone()),
                started_at: now - t.minutes * MIN,
                ended_at: now,
                result: if failed_gate {
                    TaskResult::Failed
                } else {
                    TaskResult::Done
                },
                gate_passed: Some(!failed_gate),
                escalations: t.escalation,
            };
            if let Some(why) = learned.note(&outcome, &policy, now) {
                escalation_reason = Some((t.id, why));
            }
            if failed_gate {
                t.gate_failures -= 1;
                t.escalation = learned.task_steps(t.id, &policy);
            } else {
                t.done = true;
            }
        }

        // The teams side: each idle role asks for its most urgent task,
        // the most urgent role first.
        let mut asks: Vec<usize> = roles
            .iter()
            .filter(|r| !running.iter().any(|(_, w, _)| w.role == **r))
            .filter_map(|r| {
                tasks
                    .iter()
                    .enumerate()
                    .filter(|(i, t)| {
                        t.role == *r && !t.done && !running.iter().any(|(j, _, _)| j == i)
                    })
                    .min_by_key(|(_, t)| t.priority)
                    .map(|(i, _)| i)
            })
            .collect();
        asks.sort_by_key(|i| (tasks[*i].role != "ceo", tasks[*i].priority));
        // Claude rejects a request once (10 minutes) and later warns once,
        // each the first time a worker asks after a point in the run.
        let worker_asks = asks.iter().any(|i| tasks[*i].role != "ceo");
        if step >= 20 && !rejected_sent && worker_asks {
            rejected_sent = true;
            rate = parse_rate_limit_event(
                "default",
                &json!({ "type": "rate_limit_event", "rate_limit_info": {
                    "status": "rejected", "rateLimitType": "five_hour",
                    "resetsAt": (now + 10 * MIN) / 1000 } }),
                now,
            );
        }
        if step >= 200 && !warned && worker_asks {
            warned = true;
            rate = parse_rate_limit_event(
                "default",
                &json!({ "type": "rate_limit_event", "rate_limit_info": {
                    "status": "allowed_warning", "rateLimitType": "five_hour" } }),
                now,
            );
        }
        for i in asks {
            let t = &tasks[i];
            let lead = t.role == "ceo";
            let facts = TaskFacts {
                project: PROJECT.into(),
                role: Some(t.role.into()),
                class: Some(t.class),
                escalation: t.escalation,
                priority: t.priority,
                ..Default::default()
            };
            let route = router::decide_route(&policy, &limits, &RouteSignals::default(), &facts);
            let mut r = req(PROJECT, t.role, t.priority, lead);
            r.model = route.choice.model.clone();
            let signals = Signals {
                now,
                five_hour: Some((five_hour, Some(resets_at))),
                rate: rate.clone(),
                ..plenty(now)
            };
            let current: Vec<RunningWorker> = running.iter().map(|(_, w, _)| w.clone()).collect();
            let v = decide(&limits, &signals, &r, &current, &queue.entries());
            queue.remember(&r, "default", &v, now);
            match &v.admission {
                Admission::Admit => {
                    let workers = current.iter().filter(|w| !w.lead).count();
                    let held = rate.as_ref().is_some_and(|n| rejection_holds(n, now));
                    assert!(
                        !held,
                        "step {step}: admitted {} while a rejection holds",
                        t.id
                    );
                    assert!(
                        five_hour < limits.stop_pct,
                        "step {step}: {} at {five_hour} %",
                        t.id
                    );
                    if !lead {
                        assert!(
                            five_hour < limits.lead_only_pct,
                            "step {step}: worker {} admitted at {five_hour:.1} %",
                            t.id
                        );
                        assert!(workers < limits.user_cap, "step {step}: over your cap");
                    }
                    if t.escalation > 0 {
                        escalated_route = Some(route.clone());
                    }
                    session_seq += 1;
                    let w = RunningWorker {
                        session_id: format!("00000000-0000-4000-8000-{session_seq:012}"),
                        project: PROJECT.into(),
                        role: t.role.into(),
                        model: route.choice.model,
                        lead,
                    };
                    running.push((i, w, now + t.minutes * MIN));
                }
                Admission::Wait { reasons } => {
                    for reason in reasons {
                        let kind = if reason.starts_with("Claude refused") {
                            "rejected"
                        } else if reason.starts_with("Claude warned") {
                            "warning"
                        } else if reason.contains("your limit") {
                            "cap"
                        } else if reason.contains("nothing starts") {
                            "stop"
                        } else if reason.contains("only the lead starts") {
                            "lead-only"
                        } else {
                            "other"
                        };
                        waits_on.insert(kind);
                    }
                }
            }
        }
        let workers = running.iter().filter(|(_, w, _)| !w.lead).count();
        assert!(workers <= limits.user_cap, "step {step}: {workers} workers");
        max_workers = max_workers.max(workers);
        if tasks.iter().all(|t| t.done) && running.is_empty() {
            break;
        }
    }

    assert!(tasks.iter().all(|t| t.done), "every task completed");
    assert_eq!(
        max_workers, limits.user_cap,
        "the cap was reached, never passed"
    );
    for kind in ["cap", "lead-only", "stop", "rejected", "warning"] {
        assert!(
            waits_on.contains(kind),
            "a request waited on {kind}: {waits_on:?}"
        );
    }
    assert!(!waits_on.contains("other"), "{waits_on:?}");
    // Two failed gates escalated b2, with the reason recorded…
    assert_eq!(
        escalation_reason,
        Some(("b2", "2 failed gates: escalated to 1 step".to_string()))
    );
    let note = learned
        .notes()
        .into_iter()
        .find(|n| n.direction == "up")
        .expect("an escalation note");
    assert_eq!((note.task_id.as_str(), note.steps), ("b2", 1));
    // …and the router sent it a step up the ladder, saying so.
    let route = escalated_route.expect("the escalated task ran again");
    assert_eq!(route.choice, ModelChoice::new("sonnet", Some("high")));
    assert_eq!(route.reason, "escalated 1 step from Sonnet · medium");
}

// ─────────────────────────── the contract, end to end ─────────────────────────

fn state() -> AppState {
    crate::state::Inner::new(
        crate::claude::ClaudeHome::with_base(std::env::temp_dir().join("mc-capacity-no-home")),
        crate::state::ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        crate::server::auth::Auth::ephemeral(),
    )
}

fn five_hour_at(st: &AppState, pct: f64) {
    let now = now_ms();
    st.usage.set_data_for_test(UsageData {
        windows: vec![
            PlanWindow {
                key: "five_hour".into(),
                label: "Current session".into(),
                pct,
                resets_at: Some(now + 60 * MIN),
                severity: None,
            },
            PlanWindow {
                key: "seven_day_opus".into(),
                label: "Opus weekly".into(),
                pct: 90.0,
                resets_at: Some(now + 3 * 24 * 60 * MIN),
                severity: None,
            },
        ],
        extra_usage: None,
        fetched_at: now,
        source: PlanUsageSource::Live,
    });
}

#[test]
fn the_contract_reads_the_real_signals_and_remembers_why() {
    let st = state();
    let now = now_ms();
    st.scheduler.host.set(HostSample {
        sampled_at: now,
        total_memory: 24 * GIB,
        available_memory: 16 * GIB,
        cpus: 12,
        ..Default::default()
    });
    five_hour_at(&st, 91.0);
    let worker = req("/Users/me/app", "backend", 1, false);
    match admit(&st, &worker, &[]) {
        Admission::Wait { reasons } => {
            assert!(
                reasons[0].starts_with("the 5-hour usage window is at 91 % (at 85 % or more"),
                "{reasons:?}"
            )
        }
        Admission::Admit => panic!("a worker above 85 %"),
    }
    assert!(admit(&st, &req("/Users/me/app", "ceo", 0, true), &[]).is_admit());
    let q = lock(&st.scheduler.queue).entries();
    assert_eq!(q.len(), 2);
    assert_eq!(q[0].role, "backend");
    assert_eq!(q[0].account.as_deref(), Some("default"));
    assert!(q[1].admitted);

    // An owned session's rejection stops even the lead.
    five_hour_at(&st, 10.0);
    note_rate_limit_event(
        &st,
        "default",
        &json!({ "type": "rate_limit_event", "rate_limit_info": {
            "status": "rejected", "rateLimitType": "seven_day",
            "resetsAt": (now + 30 * MIN) / 1000 } }),
    );
    match admit(&st, &req("/Users/me/app", "ceo", 0, true), &[]) {
        Admission::Wait { reasons } => {
            assert!(reasons[0].contains("on the weekly limit"), "{reasons:?}")
        }
        Admission::Admit => panic!("admitted during a rejection"),
    }

    // The router reads the Opus weekly window (90 %): plan work on Sonnet.
    let d = router::choose(
        &st,
        &TaskFacts {
            project: "/Users/me/app".into(),
            class: Some(TaskClass::Plan),
            ..Default::default()
        },
    );
    assert_eq!(d.choice, ModelChoice::new("sonnet", Some("high")));
    assert!(
        d.reason.starts_with("the Opus weekly window is at 90 %"),
        "{}",
        d.reason
    );
    assert_eq!(lock(&st.scheduler.routes).len(), 1, "kept for the page");
    assert_eq!(
        router::fallback_for(&st, &ModelChoice::new("opus", None)),
        Some("sonnet".into())
    );
}

#[test]
fn recorded_outcomes_escalate_through_the_contract() {
    // No runtime: the usage join runs inline (the task names no session).
    let st = state();
    let outcome = |result, gate| TaskOutcome {
        task_id: "task-7".into(),
        project: "/Users/me/app".into(),
        role: Some("backend".into()),
        class: Some(TaskClass::Fix),
        choice: ModelChoice::new("sonnet", Some("medium")),
        session_id: None,
        started_at: 1_000,
        ended_at: 2_000,
        result,
        gate_passed: gate,
        escalations: 0,
    };
    crate::orchestration::metrics::record_outcome(&st, &outcome(TaskResult::Failed, Some(false)));
    assert_eq!(router::escalation_for(&st, "task-7"), 0);
    crate::orchestration::metrics::record_outcome(&st, &outcome(TaskResult::Failed, Some(false)));
    assert_eq!(router::escalation_for(&st, "task-7"), 1);
    let records = st.scheduler.outcomes.all();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].escalation.as_deref(),
        Some("2 failed gates: escalated to 1 step")
    );
    assert!(records[1].joined);
    assert_eq!(
        records[1].note.as_deref(),
        Some("no session named, so no usage")
    );
    assert_eq!(records[1].account.as_deref(), Some("default"));
    // New fix tasks in the project start a step up, and say why.
    let d = router::choose(
        &st,
        &TaskFacts {
            project: "/Users/me/app".into(),
            class: Some(TaskClass::Fix),
            ..Default::default()
        },
    );
    assert_eq!(d.choice, ModelChoice::new("sonnet", Some("high")));
    assert!(
        d.reason
            .starts_with("fix tasks in this project escalated recently"),
        "{}",
        d.reason
    );
}
