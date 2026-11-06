//! The scheduler routes, called as the router calls them: who may read and
//! change what, validation with the field named, and what each answers.

use super::*;
use crate::orchestration::metrics::{TaskOutcome, TaskResult};
use crate::orchestration::router::{ModelChoice, TaskClass};
use crate::server::context::{Ingress, Principal};
use axum::http::StatusCode;
use serde_json::json;

fn state() -> AppState {
    crate::state::Inner::new(
        crate::claude::ClaudeHome::with_base(std::env::temp_dir().join("mc-capacity-api-no-home")),
        crate::state::ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
        },
        crate::server::auth::Auth::ephemeral(),
    )
}

fn desktop_ctx() -> ClientContext {
    let mut ctx = ClientContext::unknown();
    ctx.ingress = Ingress::Loopback;
    ctx.local = true;
    ctx.principal = Principal::Desktop;
    ctx
}

fn device_ctx() -> ClientContext {
    let mut ctx = ClientContext::unknown();
    ctx.principal = Principal::Device {
        id: "dev1".into(),
        allow_dangerous: false,
    };
    ctx
}

fn script_ctx() -> ClientContext {
    let mut ctx = ClientContext::unknown();
    ctx.principal = Principal::LocalScript;
    ctx
}

#[tokio::test]
async fn everyone_signed_in_reads_the_capacity_view() {
    let st = state();
    let Json(v) = get_capacity(Authed(script_ctx()), State(st.clone())).await;
    assert_eq!(v.caps.user_cap, 3);
    assert_eq!(v.caps.workers_running, 0);
    assert!(!v.can_manage, "a local script reads but does not change");
    assert!(!v.can_edit_policy);
    assert_eq!(v.accounts.len(), 1);
    assert_eq!(v.accounts[0].id, "default");
    assert!(!v.classifier.enabled);
    assert_eq!(
        v.classifier.why_not, None,
        "nothing to explain while it is off"
    );
    let json = serde_json::to_value(&v).unwrap();
    for key in [
        "host",
        "caps",
        "queue",
        "budgets",
        "routes",
        "settings",
        "rateLimits",
    ] {
        assert!(json.get(key).is_some(), "{key}");
    }
    assert_eq!(json["settings"]["userCap"], 3);
    assert_eq!(json["host"]["estimateBytes"], 600 * 1024 * 1024);
    let Json(v) = get_capacity(Authed(device_ctx()), State(st.clone())).await;
    assert!(v.can_manage && !v.can_edit_policy);
    let Json(v) = get_capacity(Authed(desktop_ctx()), State(st.clone())).await;
    assert!(v.can_manage && v.can_edit_policy);
    let Json(h) = get_host(Authed(device_ctx()), State(st)).await;
    assert!(!h.measured);
}

#[tokio::test]
async fn a_phone_changes_budgets_and_your_cap_but_not_the_desktop_settings() {
    let st = state();
    let Json(v) = put_settings(
        ManageData(device_ctx()),
        State(st.clone()),
        Body(json!({ "userCap": 5, "budgets": { "projects": { "/Users/me/app": 20 } } })),
    )
    .await
    .unwrap();
    assert_eq!(v.settings.user_cap, 5);
    assert_eq!(v.settings.budgets.projects["/Users/me/app"], 20.0);
    assert_eq!(st.scheduler.limits().user_cap, 5);
    // The project's budget shows up at once.
    assert!(v
        .budgets
        .iter()
        .any(|b| b.scope == "project" && b.key == "/Users/me/app" && b.limit_usd == 20.0));
    let err = put_settings(
        ManageData(device_ctx()),
        State(st.clone()),
        Body(json!({ "haikuClassifier": true })),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (err.status, err.code.as_str()),
        (StatusCode::FORBIDDEN, "desktop_only")
    );
    assert_eq!(err.extra["field"], "haikuClassifier");
    let err = put_settings(
        ManageData(device_ctx()),
        State(st.clone()),
        Body(json!({ "billing": { "default": "perToken" } })),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "desktop_only");
    // The desktop may.
    let Json(v) = put_settings(
        ManageData(desktop_ctx()),
        State(st.clone()),
        Body(json!({ "haikuClassifier": true, "billing": { "default": "perToken" } })),
    )
    .await
    .unwrap();
    assert!(v.settings.haiku_classifier);
    assert_eq!(v.accounts[0].billing.source, "your setting");
    // Out of range: 422 naming the field.
    let err = put_settings(
        ManageData(desktop_ctx()),
        State(st.clone()),
        Body(json!({ "userCap": 40 })),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (err.status, err.code.as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, "invalid_settings")
    );
    assert_eq!(err.extra["field"], "userCap");
    assert_eq!(st.scheduler.limits().user_cap, 5, "unchanged");
}

#[tokio::test]
async fn the_policy_is_read_by_all_and_edited_on_the_desktop() {
    let st = state();
    let Json(v) = get_policy(Authed(device_ctx()), State(st.clone())).await;
    assert!(v.is_default && !v.can_edit);
    assert_eq!(v.classes.len(), 11);
    assert_eq!(v.policy, Policy::default());
    let mut edited = serde_json::to_value(Policy::default()).unwrap();
    edited["classes"]["review"] = json!({ "model": "opus", "effort": "xhigh" });
    let Json(v) = put_policy(DesktopOnly(desktop_ctx()), State(st.clone()), Body(edited))
        .await
        .unwrap();
    assert!(!v.is_default);
    assert_eq!(
        v.policy.classes["review"],
        ModelChoice::new("opus", Some("xhigh"))
    );
    assert_eq!(
        st.scheduler.policy().classes["review"],
        ModelChoice::new("opus", Some("xhigh"))
    );
    let mut bad = serde_json::to_value(Policy::default()).unwrap();
    bad["ladder"] = json!([{ "model": "opus", "effort": "high" }, { "model": "haiku" }]);
    let err = put_policy(DesktopOnly(desktop_ctx()), State(st.clone()), Body(bad))
        .await
        .unwrap_err();
    assert_eq!(err.code, "invalid_policy");
    assert_eq!(err.extra["field"], "ladder");
    let Json(v) = reset_policy(DesktopOnly(desktop_ctx()), State(st.clone())).await;
    assert!(v.is_default);
    assert_eq!(st.scheduler.policy(), Policy::default());
}

fn outcome(task: &str, result: TaskResult) -> TaskOutcome {
    TaskOutcome {
        task_id: task.into(),
        project: "/Users/me/app".into(),
        role: Some("backend".into()),
        class: Some(TaskClass::Implement),
        choice: ModelChoice::new("sonnet", Some("medium")),
        session_id: None,
        started_at: now_ms() - 600_000,
        ended_at: now_ms(),
        result,
        gate_passed: Some(result == TaskResult::Done),
        escalations: 0,
    }
}

#[test]
fn the_report_and_its_csv_cover_recorded_outcomes() {
    let st = state();
    crate::orchestration::metrics::record_outcome(&st, &outcome("t1", TaskResult::Done));
    crate::orchestration::metrics::record_outcome(&st, &outcome("t2", TaskResult::Failed));
    let (days, since, project) = period(&ReportQuery::default()).unwrap();
    assert_eq!(days, 30);
    let r = report(&st, days, since, project);
    assert_eq!(r.totals.tasks, 2);
    assert_eq!(r.totals.completed, 1);
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0].class, "implement");
    assert_eq!(r.totals.gate_pass_rate, Some(0.5));
    assert_eq!(r.pending, 0, "joined inline without a runtime");
    assert!(r.estimate_note.starts_with("Estimates"));
    // Period and project are checked.
    let bad = ReportQuery {
        days: Some(0),
        ..Default::default()
    };
    assert_eq!(period(&bad).unwrap_err().code, "invalid_days");
    let bad = ReportQuery {
        project: Some("relative/dir".into()),
        ..Default::default()
    };
    assert_eq!(period(&bad).unwrap_err().code, "invalid_project");
    let elsewhere = ReportQuery {
        project: Some("/Users/me/other".into()),
        ..Default::default()
    };
    let (d, s, p) = period(&elsewhere).unwrap();
    assert_eq!(report(&st, d, s, p).totals.tasks, 0);
}

#[tokio::test]
async fn the_csv_downloads_with_its_name_and_type() {
    let st = state();
    let resp = get_report_csv(
        Authed(device_ctx()),
        State(st.clone()),
        Query(ReportQuery {
            days: Some(7),
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()[header::CONTENT_TYPE],
        "text/csv; charset=utf-8"
    );
    assert_eq!(
        resp.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"mother-claude-routing-7d.csv\""
    );
    let resp = get_report_csv(
        Authed(device_ctx()),
        State(st.clone()),
        Query(ReportQuery {
            rows: Some("outcomes".into()),
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        resp.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"mother-claude-task-outcomes-30d.csv\""
    );
    let err = get_report_csv(
        Authed(device_ctx()),
        State(st),
        Query(ReportQuery {
            rows: Some("everything".into()),
            ..Default::default()
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "invalid_rows");
}

#[tokio::test]
async fn classify_runs_the_rules_and_haiku_only_when_asked_and_allowed() {
    let st = state();
    let Json(c) = post_classify(
        Authed(script_ctx()),
        State(st.clone()),
        Body(ClassifyBody {
            title: "Fix the flaky login test".into(),
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    assert_eq!(c.class, TaskClass::Fix);
    let err = post_classify(
        Authed(script_ctx()),
        State(st.clone()),
        Body(ClassifyBody::default()),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "invalid_task");
    // Haiku spends tokens: not for a local script, and not while it is off.
    let haiku = || ClassifyBody {
        title: "Plan the release".into(),
        haiku: true,
        ..Default::default()
    };
    let err = post_classify(Authed(script_ctx()), State(st.clone()), Body(haiku()))
        .await
        .unwrap_err();
    assert_eq!(err.code, "desktop_only");
    let err = post_classify(Authed(desktop_ctx()), State(st.clone()), Body(haiku()))
        .await
        .unwrap_err();
    assert_eq!(
        (err.status, err.code.as_str()),
        (StatusCode::CONFLICT, "classifier_unavailable")
    );
    assert!(err.message.contains("off"));
}
