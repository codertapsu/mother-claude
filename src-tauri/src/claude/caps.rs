//! Control capabilities: what the dashboard can honestly offer for a session
//! (design §5.5 and the v1.1 ControlCaps shape of §10). One pure function,
//! [`compute`], unit-tested as a matrix, so the UI and the route dispatchers
//! agree on what each session supports.
//!
//! Owner: **B4 (foreign control)**.
//!
//! * owned ⇒ everything, direct;
//! * tmux ⇒ typed relay, Escape interrupt, slash commands, keys, terminal;
//!   approvals through hooks when they are loaded, else with keys;
//! * hooks ⇒ relayed messages (next tool call / turn end / wake listener),
//!   approvals through the held `PermissionRequest`, interrupt at the next
//!   tool boundary;
//! * experimental PTY for background jobs;
//! * otherwise nothing, with [`CapLimit`]s that name the action that helps
//!   (install foreign control, reload plugins, continue here).
//!
//! Every non-owned session with a transcript can be forked / continued here.
//!
//! Callers build a [`CapsInput`] from whatever they know (the data layer does
//! it once per summary through [`input_for`]) and call [`compute`].

use super::model::{
    ApproveVia, AttachCap, CapLimit, ControlCaps, ControlChannel, InterruptCap, LifecycleCaps,
    LimitAction, MessageCap, RenameCap, SessionSummary, SetModeCap,
};
use crate::state::AppState;

/// Everything the capability computation looks at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapsInput {
    /// Mother Claude holds a live control handle (Path B / Path A child).
    pub owned: bool,
    /// Owned through the HTTP bridge's Agent SDK host.
    pub bridge_thread: bool,
    /// The session process is alive.
    pub live: bool,
    /// A hook event or wake poll arrived from the session's current process.
    pub hooks_active: bool,
    /// The foreign-control plugin is installed at user scope.
    pub hooks_installed: bool,
    /// A `/hooks/wait` wake listener is connected right now.
    pub wake: bool,
    /// tmux pane id (`%12`) when the session runs inside tmux.
    pub tmux_pane: Option<String>,
    /// Short background job id usable with `claude stop|respawn|rm|logs|attach`.
    pub background_job: Option<String>,
    /// Experimental PTY `claude attach` is compiled in and enabled.
    pub pty_enabled: bool,
    /// The session has no transcript on disk (never prompted), so it cannot
    /// be forked or resumed.
    pub transcript_missing: bool,
    /// The effective permission mode is `bypassPermissions`.
    pub bypass: bool,
}

/// What the data layer (B2) knows about a session when it builds a summary.
/// B2 fills these from its sources; [`input_for`] turns them plus the
/// foreign-control state into a [`CapsInput`]. Fields the live roster also
/// knows (pid, tmux, job id, kind) are looked up there when missing; the
/// effective permission mode comes from the hooks' latest report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionFacts {
    pub id: String,
    /// In `state.owned` or holding a live control handle.
    pub owned: bool,
    /// A conversation of the HTTP bridge.
    pub bridge_thread: bool,
    /// The process is alive.
    pub live: bool,
    /// `interactive` | `background` | …
    pub kind: Option<String>,
    /// Short background job id, when known.
    pub job_id: Option<String>,
    /// The pidfile `tmux` target (`session:@win.%pane`), when present.
    pub tmux: Option<String>,
}

/// Facts known besides [`SessionFacts`] (a summary carries them).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtraFacts {
    /// The claude process id.
    pub pid: Option<u32>,
    /// The effective permission mode.
    pub permission_mode: Option<String>,
    /// No transcript exists for the session.
    pub transcript_missing: bool,
}

/// The mode string that removes the human from the loop.
pub const BYPASS_MODE: &str = "bypassPermissions";

/// The tmux pane id (`%N`) of a pidfile `tmux` target
/// (`session:@window.%pane`), validated the way the CLI does.
pub fn pane_of_target(target: &str) -> Option<String> {
    super::tmux::parse_target(target).map(|t| t.pane)
}

/// Build the capability input for a session from the data layer's facts and
/// the foreign-control state (hook traffic, plugin, tmux, PTY).
pub fn input_for(state: &AppState, facts: &SessionFacts) -> CapsInput {
    input_with(state, facts, &ExtraFacts::default())
}

/// [`input_for`] with the extra facts a caller has.
pub fn input_with(state: &AppState, facts: &SessionFacts, extra: &ExtraFacts) -> CapsInput {
    let live_rec = state.live.get(&facts.id);
    let pid = extra.pid.or(live_rec.as_ref().and_then(|l| l.pid));
    let tmux_target = facts
        .tmux
        .clone()
        .or_else(|| live_rec.as_ref().and_then(|l| l.tmux.clone()));
    let job_id = facts
        .job_id
        .clone()
        .or_else(|| live_rec.as_ref().and_then(|l| l.job_id.clone()));
    let is_background = matches!(facts.kind.as_deref(), Some("background") | Some("bg"))
        || live_rec
            .as_ref()
            .is_some_and(|l| matches!(l.kind.as_deref(), Some("background") | Some("bg")));
    // The short id is the first 8 hex chars of the UUID unless the CLI had to
    // rename it on a collision; prefer the recorded one.
    let background_job = job_id.or_else(|| {
        (is_background && facts.live && facts.id.len() >= 8 && facts.id.is_char_boundary(8))
            .then(|| facts.id[..8].to_string())
    });
    let mode = crate::server::hooks::permission_mode(state, &facts.id)
        .or_else(|| extra.permission_mode.clone());
    let hooks_active = crate::server::hooks::hooks_active(state, &facts.id, pid);
    // A pane this app could not reach (a custom `-S` socket, a TMUX_TMPDIR
    // only the user's shell has, tmux gone): route through the hooks when
    // they flow instead of failing every message with 409.
    let tmux_pane = tmux_target
        .as_deref()
        .and_then(pane_of_target)
        .filter(|_| !(hooks_active && state.tmux.unreachable(&facts.id, pid)));
    CapsInput {
        owned: facts.owned && !facts.bridge_thread,
        bridge_thread: facts.bridge_thread,
        live: facts.live,
        hooks_active,
        hooks_installed: state.plugin.installed_cached(),
        wake: crate::server::hooks::wake_armed(state, &facts.id),
        tmux_pane,
        background_job,
        pty_enabled: super::foreign_injection_enabled(),
        transcript_missing: extra.transcript_missing,
        bypass: mode.as_deref() == Some(BYPASS_MODE),
    }
}

/// The facts a summary itself carries (used to refresh `control` in place
/// when hook state changes between two data-layer refreshes).
pub fn facts_from_summary(s: &SessionSummary) -> (SessionFacts, ExtraFacts) {
    let bridge_thread = s.owned
        && s.control.channel == ControlChannel::Owned
        && s.control
            .limits
            .iter()
            .any(|l| l.code == codes::BRIDGE_THREAD);
    (
        SessionFacts {
            id: s.id.clone(),
            owned: s.owned,
            bridge_thread,
            live: s.live,
            kind: s.kind.clone(),
            job_id: s.job_id.clone(),
            tmux: s.tmux_pane.clone(),
        },
        ExtraFacts {
            pid: s.pid,
            permission_mode: s.permission_mode.clone(),
            // Keep whatever the data layer decided.
            transcript_missing: !s.control.continue_owned && !s.control.fork && !s.owned,
        },
    )
}

/// Recompute one session's `control` from its summary and publish it if it
/// changed (hooks became active, a wake listener connected, …). Cheap; the
/// data layer's next refresh recomputes from its own facts anyway.
pub async fn refresh(state: &AppState, id: &str) {
    let st = state.clone();
    crate::server::ws::patch_session(state, id, move |s| {
        // Owned sessions are the owned facade's business.
        if s.owned {
            return;
        }
        let (facts, extra) = facts_from_summary(s);
        s.control = compute(&input_with(&st, &facts, &extra));
    })
    .await;
}

fn limit(code: &str, text: &str, action: Option<LimitAction>) -> CapLimit {
    CapLimit {
        code: code.to_string(),
        text: text.to_string(),
        action,
    }
}

/// Stable limit codes (the UI keys copy and actions on them).
pub mod codes {
    pub const BRIDGE_THREAD: &str = "bridge_thread";
    pub const ENDED: &str = "ended";
    pub const INSTALL: &str = "foreign_control_not_installed";
    pub const HOOKS_NOT_LOADED: &str = "hooks_not_loaded";
    pub const CONTINUE_HERE: &str = "continue_here";
    pub const RELAY_DELIVERY: &str = "relay_delivery";
    pub const NO_WAKE: &str = "no_wake_listener";
    pub const NO_MODEL_CHANGE: &str = "no_model_change";
    pub const TMUX_BEST_EFFORT: &str = "tmux_best_effort";
    pub const TMUX_SETTINGS: &str = "tmux_slash_settings";
    pub const APPROVE_WITH_KEYS: &str = "approve_with_keys";
    pub const BYPASS: &str = "bypass_permissions";
    pub const PTY_EXPERIMENTAL: &str = "pty_experimental";
    pub const NO_TRANSCRIPT: &str = "no_transcript";
}

/// Compute the capabilities for one session.
pub fn compute(input: &CapsInput) -> ControlCaps {
    let owned = input.owned || input.bridge_thread;
    let lifecycle = LifecycleCaps {
        // Owned children are killed directly; background jobs via `claude stop`.
        stop: (owned && input.live) || input.background_job.is_some(),
        respawn: input.background_job.is_some() && !owned,
        rm: input.background_job.is_some() || owned,
    };

    if owned && input.live {
        let mut caps = ControlCaps {
            channel: ControlChannel::Owned,
            message: MessageCap::Direct,
            approve: true,
            answer: true,
            interrupt: InterruptCap::Immediate,
            set_model: true,
            set_mode: SetModeCap::Any,
            set_effort: true,
            slash_commands: true,
            attach_images: true,
            stop_task: true,
            fork: true,
            continue_owned: false,
            lifecycle,
            hooks_active: input.hooks_active,
            rename: RenameCap::Sync,
            attach: AttachCap::Inline,
            approve_via: ApproveVia::Control,
            ..Default::default()
        };
        if input.bridge_thread {
            caps.limits.push(limit(
                codes::BRIDGE_THREAD,
                "Driven through the HTTP bridge.",
                None,
            ));
        }
        return finish(caps);
    }

    // Anything with a transcript can be continued as an owned session: a live
    // foreign one is forked, an ended one resumes in place.
    let continuable = !input.transcript_missing;
    let mut caps = ControlCaps {
        lifecycle,
        hooks_active: input.hooks_active,
        wake: input.wake,
        fork: continuable,
        continue_owned: continuable,
        // `claude logs <short>` renders a background job's screen.
        terminal: input.background_job.is_some(),
        ..Default::default()
    };
    let continue_limit = |live: bool| {
        if !continuable {
            limit(
                codes::NO_TRANSCRIPT,
                "This session has no conversation yet, so it cannot be continued here.",
                None,
            )
        } else if live {
            limit(
                codes::CONTINUE_HERE,
                "Or continue it here as an owned session (a fork) for full control.",
                Some(LimitAction::Fork),
            )
        } else {
            limit(
                codes::CONTINUE_HERE,
                "Continue it here to pick the conversation up as an owned session.",
                Some(LimitAction::ContinueOwned),
            )
        }
    };

    if !input.live {
        caps.limits.push(limit(
            codes::ENDED,
            "This session has ended.",
            continuable.then_some(LimitAction::ContinueOwned),
        ));
        caps.limits.push(continue_limit(false));
        return finish(caps);
    }

    if input.bypass {
        caps.limits.push(limit(
            codes::BYPASS,
            "This session runs without permission prompts (bypassPermissions): messages, typed \
             text and keys from here need a device allowed dangerous actions.",
            None,
        ));
    }

    // Hooks not flowing although the plugin is installed: the session was
    // started before the install (plugins load at session start).
    let not_loaded = || {
        limit(
            codes::HOOKS_NOT_LOADED,
            "Started before foreign control was installed: run /reload-plugins in it, or \
             restart it.",
            Some(LimitAction::ReloadPlugins),
        )
    };
    let install = || {
        limit(
            codes::INSTALL,
            "Install foreign-session control on the Mac to approve and message this session \
             from here.",
            Some(LimitAction::InstallForeignControl),
        )
    };

    if input.tmux_pane.is_some() {
        caps.channel = ControlChannel::Tmux;
        caps.message = MessageCap::Relay;
        caps.interrupt = InterruptCap::Immediate;
        caps.slash_commands = true;
        caps.set_model = true;
        caps.set_effort = true;
        caps.keys = true;
        caps.terminal = true;
        caps.rename = RenameCap::Sync;
        // Files are uploaded and referenced as `@<path>` in the typed text.
        caps.attach = AttachCap::Path;
        caps.approve = input.hooks_active;
        caps.answer = input.hooks_active;
        // No channel can switch a foreign session's permission mode yet (a
        // hook reply cannot, and Shift+Tab only cycles): never offer it.
        caps.set_mode = SetModeCap::None;
        caps.approve_via = if input.hooks_active {
            ApproveVia::Hook
        } else {
            ApproveVia::Keys
        };
        caps.limits.push(limit(
            codes::TMUX_BEST_EFFORT,
            "Typed into the session's tmux pane (best effort): refused while a dialog is open; \
             a leading `!` and most slash commands need a device allowed dangerous actions.",
            None,
        ));
        caps.limits.push(limit(
            codes::TMUX_SETTINGS,
            "/model and /effort typed into the terminal also change your default for new \
             sessions.",
            None,
        ));
        if !input.hooks_active {
            caps.limits.push(limit(
                codes::APPROVE_WITH_KEYS,
                "Approvals are answered with keys in the terminal view.",
                Some(LimitAction::OpenTerminal),
            ));
            caps.limits.push(if input.hooks_installed {
                not_loaded()
            } else {
                install()
            });
        }
        caps.limits.push(continue_limit(true));
        return finish(caps);
    }

    if input.hooks_active {
        caps.channel = ControlChannel::Hooks;
        caps.message = MessageCap::Relay;
        caps.approve = true;
        caps.answer = true;
        caps.interrupt = InterruptCap::NextTool;
        caps.set_mode = SetModeCap::None;
        caps.approve_via = ApproveVia::Hook;
        // Files are uploaded and referenced as `@<path>` in the relayed text.
        caps.attach = AttachCap::Path;
        caps.limits.push(limit(
            codes::RELAY_DELIVERY,
            "Messages are delivered at Claude's next stopping point. They don't appear in the \
             session's own window; only Claude's reply does.",
            None,
        ));
        if !input.wake {
            caps.limits.push(limit(
                codes::NO_WAKE,
                "No wake listener is connected yet: a message waits for the next tool call or \
                 turn end.",
                None,
            ));
        }
        caps.limits.push(limit(
            codes::NO_MODEL_CHANGE,
            "The model, effort and slash commands can't be changed in this session from here.",
            continuable.then_some(LimitAction::Fork),
        ));
        return finish(caps);
    }

    if input.pty_enabled && input.background_job.is_some() {
        caps.channel = ControlChannel::Pty;
        caps.message = MessageCap::Relay;
        caps.limits.push(limit(
            codes::PTY_EXPERIMENTAL,
            "Experimental: typed into the background session through `claude attach`.",
            None,
        ));
        caps.limits.push(if input.hooks_installed {
            not_loaded()
        } else {
            install()
        });
        caps.limits.push(continue_limit(true));
        return finish(caps);
    }

    caps.limits.push(if input.hooks_installed {
        not_loaded()
    } else {
        install()
    });
    caps.limits.push(continue_limit(true));
    finish(caps)
}

/// Mirror the limits into the legacy `notes`.
fn finish(mut caps: ControlCaps) -> ControlCaps {
    caps.notes = caps.limits.iter().map(|l| l.text.clone()).collect();
    caps
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn has(c: &ControlCaps, code: &str) -> bool {
        c.limits.iter().any(|l| l.code == code)
    }

    fn action(c: &ControlCaps, code: &str) -> Option<LimitAction> {
        c.limits
            .iter()
            .find(|l| l.code == code)
            .and_then(|l| l.action)
    }

    #[test]
    fn owned_gets_everything() {
        let c = compute(&CapsInput {
            owned: true,
            live: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Owned);
        assert_eq!(c.message, MessageCap::Direct);
        assert!(c.approve && c.answer && c.set_model && c.set_effort && c.attach_images);
        assert!(c.slash_commands && c.stop_task && c.fork && !c.continue_owned);
        assert_eq!(c.interrupt, InterruptCap::Immediate);
        assert_eq!(c.set_mode, SetModeCap::Any);
        assert_eq!(c.rename, RenameCap::Sync);
        assert_eq!(c.attach, AttachCap::Inline);
        assert_eq!(c.approve_via, ApproveVia::Control);
        assert!(!c.keys && !c.terminal);
        assert!(c.lifecycle.stop && c.lifecycle.rm && !c.lifecycle.respawn);
        assert!(c.limits.is_empty() && c.notes.is_empty());

        let b = compute(&CapsInput {
            bridge_thread: true,
            live: true,
            ..Default::default()
        });
        assert_eq!(b.channel, ControlChannel::Owned);
        assert!(has(&b, codes::BRIDGE_THREAD));
        assert_eq!(b.notes.len(), b.limits.len());
    }

    #[test]
    fn tmux_with_hooks_approves_through_the_hook() {
        let c = compute(&CapsInput {
            live: true,
            tmux_pane: Some("%3".into()),
            hooks_active: true,
            hooks_installed: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Tmux);
        assert_eq!(c.message, MessageCap::Relay);
        assert_eq!(c.interrupt, InterruptCap::Immediate);
        assert!(c.approve && c.answer && c.slash_commands && c.set_model && c.set_effort);
        assert!(c.keys && c.terminal && c.fork && c.continue_owned);
        assert_eq!(c.approve_via, ApproveVia::Hook);
        assert_eq!(c.set_mode, SetModeCap::None, "POST /mode refuses tmux");
        assert_eq!(c.rename, RenameCap::Sync);
        assert!(has(&c, codes::TMUX_BEST_EFFORT) && has(&c, codes::TMUX_SETTINGS));
        assert!(!has(&c, codes::APPROVE_WITH_KEYS));
    }

    #[test]
    fn tmux_without_hooks_approves_with_keys() {
        let c = compute(&CapsInput {
            live: true,
            tmux_pane: Some("%3".into()),
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Tmux);
        assert!(!c.approve && !c.answer && c.slash_commands && c.keys);
        assert_eq!(c.approve_via, ApproveVia::Keys);
        assert_eq!(c.set_mode, SetModeCap::None);
        assert_eq!(
            action(&c, codes::APPROVE_WITH_KEYS),
            Some(LimitAction::OpenTerminal)
        );
        assert_eq!(
            action(&c, codes::INSTALL),
            Some(LimitAction::InstallForeignControl)
        );
        // Installed but this session never loaded it.
        let c = compute(&CapsInput {
            live: true,
            tmux_pane: Some("%3".into()),
            hooks_installed: true,
            ..Default::default()
        });
        assert_eq!(
            action(&c, codes::HOOKS_NOT_LOADED),
            Some(LimitAction::ReloadPlugins)
        );
        assert!(!has(&c, codes::INSTALL));
    }

    #[test]
    fn hooks_idle_with_a_waiter() {
        let c = compute(&CapsInput {
            live: true,
            hooks_active: true,
            hooks_installed: true,
            wake: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Hooks);
        assert_eq!(c.message, MessageCap::Relay);
        assert!(c.approve && c.answer && c.fork && c.continue_owned && c.wake);
        assert_eq!(c.interrupt, InterruptCap::NextTool);
        assert_eq!(c.set_mode, SetModeCap::None, "POST /mode refuses hooks");
        assert_eq!(c.approve_via, ApproveVia::Hook);
        assert!(!c.set_model && !c.slash_commands && !c.keys && !c.terminal);
        assert_eq!(c.rename, RenameCap::None);
        assert_eq!(c.attach, AttachCap::Path);
        assert!(has(&c, codes::RELAY_DELIVERY));
        assert!(!has(&c, codes::NO_WAKE));
        assert_eq!(action(&c, codes::NO_MODEL_CHANGE), Some(LimitAction::Fork));
    }

    #[test]
    fn hooks_idle_without_a_waiter() {
        let c = compute(&CapsInput {
            live: true,
            hooks_active: true,
            hooks_installed: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Hooks);
        assert!(!c.wake);
        assert!(has(&c, codes::NO_WAKE));
        // The limit is informational: messages still relay.
        assert_eq!(c.message, MessageCap::Relay);
    }

    #[test]
    fn installed_but_not_loaded() {
        let c = compute(&CapsInput {
            live: true,
            hooks_installed: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::None);
        assert_eq!(c.message, MessageCap::None);
        assert!(!c.approve);
        assert_eq!(
            action(&c, codes::HOOKS_NOT_LOADED),
            Some(LimitAction::ReloadPlugins)
        );
        assert!(!has(&c, codes::INSTALL), "never 'not installed' here");
        assert_eq!(action(&c, codes::CONTINUE_HERE), Some(LimitAction::Fork));
        assert!(c.fork && c.continue_owned);
    }

    #[test]
    fn not_installed() {
        let c = compute(&CapsInput {
            live: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::None);
        assert_eq!(c.message, MessageCap::None);
        assert!(!c.approve);
        assert_eq!(
            action(&c, codes::INSTALL),
            Some(LimitAction::InstallForeignControl)
        );
        assert!(c
            .notes
            .iter()
            .any(|n| n.contains("Install foreign-session control")));
        assert!(c.continue_owned && c.fork);
    }

    #[test]
    fn ended_sessions_can_only_be_continued() {
        let c = compute(&CapsInput::default());
        assert_eq!(c.channel, ControlChannel::None);
        assert!(c.continue_owned && c.fork);
        assert!(!c.lifecycle.stop);
        assert_eq!(action(&c, codes::ENDED), Some(LimitAction::ContinueOwned));
        assert_eq!(
            action(&c, codes::CONTINUE_HERE),
            Some(LimitAction::ContinueOwned)
        );
        // Without a transcript nothing can be continued.
        let c = compute(&CapsInput {
            transcript_missing: true,
            ..Default::default()
        });
        assert!(!c.continue_owned && !c.fork);
        assert!(has(&c, codes::NO_TRANSCRIPT));
    }

    #[test]
    fn background_lifecycle_uses_the_short_id() {
        let c = compute(&CapsInput {
            live: true,
            background_job: Some("a1b2c3d4".into()),
            ..Default::default()
        });
        assert!(c.lifecycle.stop && c.lifecycle.respawn && c.lifecycle.rm);
        assert!(c.terminal, "claude logs renders a background job's screen");
        assert_eq!(c.channel, ControlChannel::None);
        let c = compute(&CapsInput {
            live: true,
            background_job: Some("a1b2c3d4".into()),
            pty_enabled: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Pty);
        assert!(has(&c, codes::PTY_EXPERIMENTAL));
        // Hooks win over PTY for a background job that loaded them.
        let c = compute(&CapsInput {
            live: true,
            background_job: Some("a1b2c3d4".into()),
            pty_enabled: true,
            hooks_active: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Hooks);
    }

    #[test]
    fn bypass_sessions_carry_a_limit() {
        let c = compute(&CapsInput {
            live: true,
            hooks_active: true,
            bypass: true,
            ..Default::default()
        });
        assert!(has(&c, codes::BYPASS));
        assert_eq!(c.channel, ControlChannel::Hooks);
    }

    #[test]
    fn serialized_shape_matches_the_contract() {
        let c = compute(&CapsInput {
            live: true,
            tmux_pane: Some("%1".into()),
            ..Default::default()
        });
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["channel"], "tmux");
        assert_eq!(v["approveVia"], "keys");
        assert_eq!(v["rename"], "sync");
        assert_eq!(v["attach"], "path");
        assert_eq!(v["keys"], true);
        assert_eq!(v["terminal"], true);
        assert_eq!(v["wake"], false);
        assert!(v["limits"].is_array());
        let first = &v["limits"][0];
        assert!(first["code"].is_string() && first["text"].is_string());
        assert_eq!(
            json!(LimitAction::InstallForeignControl),
            json!("install-foreign-control")
        );
        assert_eq!(json!(LimitAction::OpenOnDesktop), json!("open-on-desktop"));
        assert_eq!(json!(ApproveVia::Control), json!("control"));
        // Tolerant round trip.
        let back: ControlCaps = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn input_from_facts() {
        let state = crate::state::Inner::new(
            crate::claude::ClaudeHome::with_base(std::env::temp_dir()),
            crate::state::ServerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            },
            crate::server::auth::Auth::ephemeral(),
        );
        let input = input_for(
            &state,
            &SessionFacts {
                id: "0a1b2c3d-0000-4000-8000-000000000000".into(),
                live: true,
                kind: Some("background".into()),
                tmux: Some("main:@1.%7".into()),
                ..Default::default()
            },
        );
        assert_eq!(input.tmux_pane.as_deref(), Some("%7"));
        assert_eq!(input.background_job.as_deref(), Some("0a1b2c3d"));
        assert!(!input.hooks_active && !input.wake && !input.bypass);
        let bridge = input_with(
            &state,
            &SessionFacts {
                id: "b".into(),
                owned: true,
                bridge_thread: true,
                live: true,
                ..Default::default()
            },
            &ExtraFacts {
                permission_mode: Some(BYPASS_MODE.into()),
                ..Default::default()
            },
        );
        assert!(!bridge.owned && bridge.bridge_thread && bridge.bypass);
        assert_eq!(compute(&bridge).channel, ControlChannel::Owned);
        // An invalid tmux target is ignored.
        let bad = input_for(
            &state,
            &SessionFacts {
                id: "c".into(),
                live: true,
                tmux: Some("x; rm -rf /:@1.%2".into()),
                ..Default::default()
            },
        );
        assert!(bad.tmux_pane.is_none());

        // A pane this app cannot reach falls back to the hooks, when they flow.
        let id = "1a1b2c3d-0000-4000-8000-000000000000";
        let facts = SessionFacts {
            id: id.into(),
            live: true,
            tmux: Some("main:@1.%7".into()),
            ..Default::default()
        };
        state.tmux.mark_unreachable(id, None);
        assert_eq!(
            input_for(&state, &facts).tmux_pane.as_deref(),
            Some("%7"),
            "no hooks: tmux is still the best bet"
        );
        state.hooks.touch(id, 1);
        let hooked = input_for(&state, &facts);
        assert!(hooked.hooks_active && hooked.tmux_pane.is_none());
        assert_eq!(compute(&hooked).channel, ControlChannel::Hooks);
    }
}
