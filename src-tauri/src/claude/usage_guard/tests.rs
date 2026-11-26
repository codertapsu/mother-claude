use super::*;

const NOW: i64 = 1_800_000_000_000;
const HOUR: i64 = 3_600_000;

fn window(key: &str, label: &str, pct: f64, resets_in: Option<i64>) -> PlanWindow {
    PlanWindow {
        key: key.into(),
        label: label.into(),
        pct,
        resets_at: resets_in.map(|d| NOW + d),
        severity: None,
    }
}

fn model(id: &str, label: &str) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        label: label.into(),
        context_window: None,
    }
}

#[test]
fn the_session_and_weekly_windows_pause_the_account_a_model_window_its_model() {
    assert_eq!(scope_of("five_hour"), Some(Scope::Account));
    assert_eq!(scope_of("seven_day"), Some(Scope::Account));
    assert_eq!(
        scope_of("seven_day_opus"),
        Some(Scope::Model("opus".into()))
    );
    assert_eq!(
        scope_of("seven_day_fable"),
        Some(Scope::Model("fable".into()))
    );
    // Extra usage and a scope it cannot tie to sessions: left alone.
    assert_eq!(scope_of("seven_day_overage_included"), None);
    assert_eq!(scope_of("seven_day_scoped"), None);
    assert_eq!(scope_of("overage"), None);
    assert_eq!(scope_of("seven_day_"), None);

    let opus = model("claude-opus-4-7", "Opus 4.7");
    let fable = model("claude-fable-5-2", "Fable 5.2");
    assert!(model_matches(None, Some(&opus)));
    assert!(model_matches(Some("opus"), Some(&opus)));
    assert!(!model_matches(Some("opus"), Some(&fable)));
    assert!(model_matches(Some("fable"), Some(&fable)));
    // The window's slug may carry the label's version.
    assert!(model_matches(Some("fable_5"), Some(&model("x", "Fable 5"))));
    // No model known: a per-model window cannot claim it.
    assert!(!model_matches(Some("opus"), None));
    // A word, not a part of one.
    assert!(!model_matches(Some("pus"), Some(&opus)));
}

#[test]
fn a_window_pauses_once_at_the_threshold_until_it_resets() {
    let s = GuardSettings::default();
    let w = window("five_hour", "Current session", 91.0, Some(HOUR));
    assert!(should_pause(&s, &w, "default", &[], &[], NOW));
    // Below the threshold, no reset known, a reset passed: no.
    assert!(!should_pause(
        &s,
        &window("five_hour", "", 89.9, Some(HOUR)),
        "default",
        &[],
        &[],
        NOW
    ));
    assert!(!should_pause(
        &s,
        &window("five_hour", "", 95.0, None),
        "default",
        &[],
        &[],
        NOW
    ));
    assert!(!should_pause(
        &s,
        &window("five_hour", "", 95.0, Some(-1)),
        "default",
        &[],
        &[],
        NOW
    ));
    // Off, or a window it does not watch: no.
    let off = GuardSettings {
        enabled: false,
        ..GuardSettings::default()
    };
    assert!(!should_pause(&off, &w, "default", &[], &[], NOW));
    assert!(!should_pause(
        &s,
        &window("overage", "", 100.0, Some(HOUR)),
        "default",
        &[],
        &[],
        NOW
    ));
    // The threshold is the setting's.
    let low = GuardSettings {
        threshold_pct: 80,
        ..GuardSettings::default()
    };
    assert!(should_pause(
        &low,
        &window("five_hour", "", 80.0, Some(HOUR)),
        "default",
        &[],
        &[],
        NOW
    ));

    // Paused already, or paused and resumed in this very window: once.
    let paused = Pause {
        id: "up_1".into(),
        account: "default".into(),
        window: "five_hour".into(),
        resets_at: NOW + HOUR,
        ..Default::default()
    };
    assert!(!should_pause(
        &s,
        &w,
        "default",
        std::slice::from_ref(&paused),
        &[],
        NOW
    ));
    let ended = Pause {
        ended: Some(Ended::Resumed),
        ..paused.clone()
    };
    assert!(!should_pause(
        &s,
        &w,
        "default",
        std::slice::from_ref(&ended),
        &[],
        NOW
    ));
    // The next window (a later reset) pauses again.
    let next = window("five_hour", "", 92.0, Some(6 * HOUR));
    assert!(should_pause(&s, &next, "default", &[ended], &[], NOW));
    // Another account, another window: its own.
    assert!(should_pause(
        &s,
        &w,
        "acc_2",
        std::slice::from_ref(&paused),
        &[],
        NOW
    ));
    assert!(should_pause(
        &s,
        &window("seven_day", "", 95.0, Some(24 * HOUR)),
        "default",
        &[paused],
        &[],
        NOW
    ));
    // Resumed by hand: not again before that window resets.
    let held = vec![("default".to_string(), "five_hour".to_string(), NOW + HOUR)];
    assert!(!should_pause(&s, &w, "default", &[], &held, NOW));
    assert!(should_pause(&s, &next, "default", &[], &held, NOW));
}

#[test]
fn near_the_threshold_usage_is_read_every_minute() {
    let s = GuardSettings::default();
    assert!(near(&s, &[window("five_hour", "", 80.0, Some(HOUR))], NOW));
    assert!(!near(&s, &[window("five_hour", "", 79.9, Some(HOUR))], NOW));
    assert!(!near(&s, &[window("five_hour", "", 85.0, Some(-1))], NOW));
    assert!(!near(&s, &[window("overage", "", 99.0, Some(HOUR))], NOW));
    let off = GuardSettings {
        enabled: false,
        ..GuardSettings::default()
    };
    assert!(!near(
        &off,
        &[window("five_hour", "", 95.0, Some(HOUR))],
        NOW
    ));
}

#[test]
fn a_window_reads_empty_once_it_has_reset() {
    assert_eq!(
        pct_now(&window("five_hour", "", 97.0, Some(HOUR)), NOW),
        97.0
    );
    assert_eq!(pct_now(&window("five_hour", "", 97.0, Some(0)), NOW), 0.0);
    assert_eq!(pct_now(&window("five_hour", "", 97.0, None), NOW), 97.0);
}

#[test]
fn times_and_windows_read_as_people_say_them() {
    assert_eq!(span_text(30_000), "in under a minute");
    assert_eq!(span_text(60_000), "in 1 min");
    assert_eq!(span_text(4 * 60_000 + 10), "in 5 min");
    assert_eq!(span_text(76 * 60_000), "in 1 h 16 min");
    assert_eq!(span_text(2 * HOUR), "in 2 h");
    assert_eq!(span_text(72 * HOUR), "in 3 days");
    assert_eq!(
        window_words("five_hour", "Current session"),
        "the 5-hour limit"
    );
    assert_eq!(
        window_words("seven_day", "Weekly · all models"),
        "the weekly limit"
    );
    assert_eq!(
        window_words("seven_day_opus", "Weekly · Opus"),
        "the weekly Opus limit"
    );
    assert_eq!(window_words("odd", "Something"), "the something limit");
}

#[test]
fn a_resumed_session_is_told_what_stopped() {
    let p = Pause {
        window: "five_hour".into(),
        label: "Current session".into(),
        pct: 91.4,
        ..Default::default()
    };
    let s = PausedSession {
        doing: vec![
            "the turn it was working on".into(),
            "Agent “audio timing + STT checks”".into(),
        ],
        todos: Some((3, 7)),
        ..Default::default()
    };
    let text = resume_message(&p, &s);
    assert!(text.starts_with("Mother Claude paused this session at 91 % of the 5-hour limit"));
    assert!(text.contains("Continue exactly where you left off."));
    assert!(text.contains("- Agent “audio timing + STT checks”"));
    assert!(text.contains("Start again what did not finish"));
    assert!(text.contains("3 of 7 done"));
    // Nothing more to say: just the first lines.
    let plain = resume_message(&p, &PausedSession::default());
    assert!(!plain.contains("stopped:"));
    assert!(!plain.contains("todo"));
}

#[test]
fn settings_take_only_what_makes_sense() {
    let s = GuardSettings::default();
    assert!(s.enabled && s.auto_resume && s.include_elsewhere);
    assert_eq!((s.threshold_pct, s.buffer_secs), (90, 60));
    let next = s
        .with(&SettingsPatch {
            threshold_pct: Some(85),
            auto_resume: Some(false),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(next.threshold_pct, 85);
    assert!(!next.auto_resume);
    assert!(next.enabled);
    for bad in [49, 100, 0] {
        let e = s
            .with(&SettingsPatch {
                threshold_pct: Some(bad),
                ..Default::default()
            })
            .unwrap_err();
        assert!(e.contains("50 to 99"), "{e}");
    }
    assert!(s
        .with(&SettingsPatch {
            buffer_secs: Some(MAX_BUFFER_SECS + 1),
            ..Default::default()
        })
        .is_err());
}

#[test]
fn the_state_survives_a_restart_and_keeps_a_short_history() {
    let dir = tempfile::tempdir().unwrap();
    let guard = UsageGuard::load(Some(dir.path()));
    assert!(guard.active().is_empty());
    {
        let mut s = guard.lock();
        s.settings.threshold_pct = 80;
        for i in 0..14 {
            s.pauses.push(Pause {
                id: format!("up_{i}"),
                account: "default".into(),
                window: "five_hour".into(),
                ended: (i < 13).then_some(Ended::Resumed),
                ..Default::default()
            });
        }
        s.held_off.push(HeldOff {
            account: "default".into(),
            window: "five_hour".into(),
            until: NOW - 1,
        });
        prune(&mut s, NOW);
        guard.save(&s);
    }
    let again = UsageGuard::load(Some(dir.path()));
    let view = again.view();
    assert_eq!(view.settings.threshold_pct, 80);
    // The active one first, then the last ten that ended.
    assert_eq!(view.pauses.len(), 1 + HISTORY);
    assert!(view.pauses[0].active());
    assert_eq!(view.pauses[0].id, "up_13");
    assert_eq!(again.active().len(), 1);
    // A hold-off whose window reset is gone.
    assert!(again.lock().held_off.is_empty());
    // A file with a threshold out of range is brought back into it.
    std::fs::write(
        dir.path().join(FILE),
        r#"{"settings":{"enabled":true,"thresholdPct":5,"bufferSecs":999999}}"#,
    )
    .unwrap();
    let odd = UsageGuard::load(Some(dir.path()));
    assert_eq!(odd.settings().threshold_pct, MIN_THRESHOLD);
    assert_eq!(odd.settings().buffer_secs, MAX_BUFFER_SECS);
}

#[test]
fn a_pause_covers_its_account_and_model() {
    use crate::claude::model::AccountRef;
    let p = Pause {
        account: "acc_2".into(),
        ..Default::default()
    };
    let mut s = SessionSummary::default();
    assert!(!p.covers(&s), "no account: the Default's");
    s.account = Some(AccountRef {
        id: "acc_2".into(),
        name: "Work".into(),
        color: String::new(),
        kind: crate::claude::accounts::AccountKind::Directory,
    });
    assert!(p.covers(&s));
    let opus_only = Pause {
        model: Some("opus".into()),
        ..p.clone()
    };
    assert!(!opus_only.covers(&s));
    s.model = Some(model("claude-opus-4-7", "Opus 4.7"));
    assert!(opus_only.covers(&s));
    let default = Pause {
        account: DEFAULT_ID.into(),
        ..Default::default()
    };
    assert!(default.covers(&SessionSummary::default()));
}

#[test]
fn a_message_leaves_a_session_alone_even_one_idle_when_it_paused() {
    let stopped = PausedSession {
        id: "s1".into(),
        stopped: true,
        ..Default::default()
    };
    let mut pauses = vec![
        Pause {
            id: "up_1".into(),
            account: DEFAULT_ID.into(),
            sessions: vec![stopped],
            ..Default::default()
        },
        Pause {
            id: "up_2".into(),
            account: "acc_2".into(),
            ..Default::default()
        },
        Pause {
            id: "up_3".into(),
            account: DEFAULT_ID.into(),
            ended: Some(Ended::Resumed),
            ..Default::default()
        },
    ];
    // On the list: released once.
    assert!(release(&mut pauses, "s1", None));
    assert!(pauses[0].sessions[0].released);
    assert!(!release(&mut pauses, "s1", None), "already");
    // Idle when it paused (not on the list): added, left alone, never
    // resumed (it is not one the pause stopped). Only the pauses that cover
    // it, and never an ended one.
    let idle = SessionSummary {
        id: "s2".into(),
        name: "docs".into(),
        ..Default::default()
    };
    assert!(release(&mut pauses, "s2", Some(&idle)));
    let added = pauses[0].sessions.iter().find(|s| s.id == "s2").unwrap();
    assert!(added.released && !added.stopped);
    assert_eq!(added.name, "docs");
    assert!(pauses[1].sessions.is_empty(), "another account's pause");
    assert!(pauses[2].sessions.is_empty(), "an ended pause");
    // Unknown to the session list: nothing to add.
    assert!(!release(&mut pauses, "s3", None));
}

#[test]
fn the_timer_wakes_at_the_resume_never_spinning_on_a_due_pause() {
    let on = GuardSettings::default();
    let pause = |resume_in: i64, hold: bool| Pause {
        id: "up_1".into(),
        resume_at: NOW + resume_in,
        hold,
        ..Default::default()
    };
    // Nothing paused, or far off: the tick.
    assert_eq!(wait_until_next(&on, &[], NOW), TICK);
    assert_eq!(wait_until_next(&on, &[pause(HOUR, false)], NOW), TICK);
    // Sooner than the tick: at the resume.
    assert_eq!(
        wait_until_next(&on, &[pause(5_000, false)], NOW),
        Duration::from_secs(5)
    );
    // Due (or overdue) but still active: the shortest wait, never zero.
    assert_eq!(
        wait_until_next(&on, &[pause(-60_000, false)], NOW),
        MIN_WAIT
    );
    // Held, or resuming by itself off: nothing to time.
    assert_eq!(wait_until_next(&on, &[pause(-60_000, true)], NOW), TICK);
    let off = GuardSettings {
        auto_resume: false,
        ..GuardSettings::default()
    };
    assert_eq!(wait_until_next(&off, &[pause(-60_000, false)], NOW), TICK);
}

#[test]
fn teams_wait_for_a_whole_account_pause_or_their_models() {
    let g = UsageGuard::load(None);
    g.lock().pauses.push(Pause {
        id: "up_1".into(),
        account: DEFAULT_ID.into(),
        window: "seven_day_opus".into(),
        label: "Weekly · Opus".into(),
        pct: 92.4,
        model: Some("opus".into()),
        resume_at: NOW + 72 * HOUR,
        ..Default::default()
    });
    // A model's window holds only that model, alias or full id.
    assert!(g.account_pause(DEFAULT_ID).is_none());
    let p = g.model_pause(DEFAULT_ID, "opus").unwrap();
    assert!(g.model_pause(DEFAULT_ID, "claude-opus-5-5").is_some());
    assert!(g.model_pause(DEFAULT_ID, "sonnet").is_none());
    assert!(g.model_pause(DEFAULT_ID, "").is_none());
    assert!(g.model_pause("acc_2", "opus").is_none());
    assert_eq!(
        pause_reason(&p, NOW),
        "Mother Claude paused this account's sessions on that model at 92 % of the weekly \
         Opus limit (it resumes in 3 days)"
    );
    let held = Pause {
        hold: true,
        model: None,
        window: "five_hour".into(),
        ..p
    };
    assert_eq!(
        pause_reason(&held, NOW),
        "Mother Claude paused this account's sessions at 92 % of the 5-hour limit (until \
         someone resumes it)"
    );
}

#[test]
fn a_session_two_limits_paused_resumes_with_the_last_one() {
    let from_five_hour = PausedSession {
        id: "s1".into(),
        name: "auth".into(),
        owned: true,
        cwd: "/w/auth".into(),
        stopped: true,
        doing: vec!["the turn it was working on".into(), "Agent “tests”".into()],
        todos: Some((2, 5)),
        model: Some("claude-opus-5-5".into()),
        resumed_at: Some(NOW),
        ..Default::default()
    };
    let mut pauses = vec![
        Pause {
            id: "up_week".into(),
            account: DEFAULT_ID.into(),
            window: "seven_day".into(),
            sessions: vec![PausedSession {
                id: "s1".into(),
                stopped: false,
                doing: vec!["the turn it was working on".into()],
                ..Default::default()
            }],
            ..Default::default()
        },
        Pause {
            id: "up_done".into(),
            ended: Some(Ended::Resumed),
            ..Default::default()
        },
    ];
    // Merged into the pause that still holds it: stopped, everything that
    // stopped named once, what it ran with kept.
    hand_over(&mut pauses, "up_week", &from_five_hour);
    let s = &pauses[0].sessions[0];
    assert!(s.stopped);
    assert_eq!(
        s.doing,
        vec![
            "the turn it was working on".to_string(),
            "Agent “tests”".to_string()
        ]
    );
    assert_eq!(s.todos, Some((2, 5)));
    assert_eq!(s.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(s.cwd, "/w/auth");
    // Not on its list yet: added, never as resumed already.
    let other = PausedSession {
        id: "s2".into(),
        ..from_five_hour.clone()
    };
    hand_over(&mut pauses, "up_week", &other);
    let added = pauses[0].sessions.iter().find(|s| s.id == "s2").unwrap();
    assert!(added.stopped && added.resumed_at.is_none());
    // An ended pause takes nothing.
    hand_over(&mut pauses, "up_done", &other);
    assert!(pauses[1].sessions.is_empty());
}

const MIN: i64 = 60_000;

/// A live session of the Default account, `status` with activity `kind`.
fn live(
    id: &str,
    owned: bool,
    status: SessionStatus,
    kind: ActivityKind,
    resumes_at: Option<i64>,
) -> SessionSummary {
    let mut s = SessionSummary {
        id: id.into(),
        owned,
        live: true,
        status,
        ..Default::default()
    };
    // Started elsewhere: reached through the plugin's hooks.
    if !owned {
        s.control.channel = ControlChannel::Hooks;
    }
    s.activity.kind = kind;
    s.activity.resumes_at = resumes_at;
    if kind == ActivityKind::Scheduled {
        s.activity.detail = Some("Check the deploy".into());
    }
    s
}

#[test]
fn a_scheduled_run_is_held_and_stopped_when_it_starts_a_turn() {
    use ActivityKind::{Idle, Scheduled, Thinking, Tool};
    use SessionStatus::{Background, Working};
    let mut p = Pause {
        id: "up_1".into(),
        account: DEFAULT_ID.into(),
        paused_at: NOW - HOUR,
        resume_at: NOW + HOUR,
        ..Default::default()
    };
    // Waiting on a wakeup due before the resume: held, with what runs.
    let waiting = live("s1", false, Background, Scheduled, Some(NOW + 10 * MIN));
    let hold = Watch::Hold {
        at: Some(NOW + 10 * MIN),
        run: Some("Check the deploy".into()),
    };
    assert_eq!(watch_one(&p, &waiting, NOW, None), hold);
    // Due after the resume: it runs once work goes on anyway — unless the
    // pause waits for someone.
    let later = live("s9", false, Background, Scheduled, Some(NOW + 2 * HOUR));
    assert_eq!(watch_one(&p, &later, NOW, None), Watch::Leave);
    let waits = Pause {
        hold: true,
        ..p.clone()
    };
    assert!(matches!(
        watch_one(&waits, &later, NOW, None),
        Watch::Hold { .. }
    ));
    // A recurring job whose next run is unknown: held.
    let cron = live("s8", false, Background, Scheduled, None);
    assert!(matches!(
        watch_one(&p, &cron, NOW, None),
        Watch::Hold { at: None, .. }
    ));
    // Nothing reaches it (no hooks, no tmux pane): never shown as held.
    let mut unreachable = live("s7", false, Background, Scheduled, Some(NOW + 10 * MIN));
    unreachable.control.channel = ControlChannel::None;
    assert_eq!(watch_one(&p, &unreachable, NOW, None), Watch::Leave);
    // Noted once.
    p.sessions.push(held_entry(
        &waiting,
        Some(NOW + 10 * MIN),
        Some("Check the deploy".into()),
    ));
    assert_eq!(watch_one(&p, &waiting, NOW, None), Watch::Leave);

    // The run starts a turn (a session started elsewhere too): stopped, also
    // when it fires a little early.
    let fired = live("s1", false, Working, Thinking, None);
    assert_eq!(
        watch_one(&p, &fired, NOW + 10 * MIN, None),
        Watch::Stop { fired: true }
    );
    assert_eq!(
        watch_one(&p, &fired, NOW + 8 * MIN, None),
        Watch::Stop { fired: true }
    );
    // Typed there well before the run: someone's turn, which releases it.
    assert_eq!(watch_one(&p, &fired, NOW, None), Watch::Release);
    // Released: left alone, whatever starts its turns.
    p.sessions[0].released = true;
    assert_eq!(watch_one(&p, &fired, NOW + 10 * MIN, None), Watch::Leave);

    // Mother Claude's own: any turn nobody sent it is stopped again...
    let own = live("s2", true, Working, Tool, None);
    assert_eq!(watch_one(&p, &own, NOW, None), Watch::Stop { fired: false });
    // ...but not one someone started or continued since the pause began.
    assert_eq!(watch_one(&p, &own, NOW, Some(NOW - MIN)), Watch::Leave);
    assert_eq!(
        watch_one(&p, &own, NOW, Some(NOW - 2 * HOUR)),
        Watch::Stop { fired: false }
    );
    // Started elsewhere and not held: a turn typed there goes on.
    let typed = live("s3", false, Working, Tool, None);
    assert_eq!(watch_one(&p, &typed, NOW, None), Watch::Leave);
    // Idle: nothing to do.
    let idle = live("s4", true, SessionStatus::Idle, Idle, None);
    assert_eq!(watch_one(&p, &idle, NOW, None), Watch::Leave);
}

#[test]
fn a_stopped_run_is_recorded_once_and_no_longer_held() {
    let waiting = live(
        "s1",
        false,
        SessionStatus::Background,
        ActivityKind::Scheduled,
        Some(NOW),
    );
    let mut p = Pause {
        id: "up_1".into(),
        sessions: vec![held_entry(
            &waiting,
            Some(NOW),
            Some("Check the deploy".into()),
        )],
        ..Default::default()
    };
    let turn = run_words(Some("Check the deploy"));
    assert_eq!(
        turn,
        "the turn its scheduled run “Check the deploy” started while it was paused"
    );
    let stopped = PausedSession {
        id: "s1".into(),
        stopped: true,
        doing: vec![turn.clone()],
        model: Some("claude-opus-5-5".into()),
        ..Default::default()
    };
    merge_stopped(&mut p, stopped.clone());
    merge_stopped(&mut p, stopped);
    let s = &p.sessions[0];
    assert!(s.stopped && !s.held && !s.released);
    assert_eq!(s.doing, vec![turn]);
    assert_eq!(s.run.as_deref(), Some("Check the deploy"));
    assert_eq!(s.model.as_deref(), Some("claude-opus-5-5"));
    // It is told what stopped when the pause resumes.
    let p = Pause {
        pct: 91.0,
        window: "five_hour".into(),
        ..p
    };
    let text = resume_message(&p, &p.sessions[0]);
    assert!(
        text.contains("- the turn its scheduled run “Check the deploy” started"),
        "{text}"
    );
    assert_eq!(
        run_words(None),
        "the turn a scheduled run (a wakeup or a cron job) started while it was paused"
    );
}
