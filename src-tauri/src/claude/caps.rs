//! Control capabilities: what the dashboard can honestly offer for a session
//! (design §5.5). One pure function, unit-tested, so the UI and the route
//! dispatchers agree on what each session supports.
//!
//! Owner: **B4 (foreign control)** completes this. The foundation ships a sane
//! minimal version so the rest of the app has real values to work with:
//! owned ⇒ everything; tmux ⇒ relay + keys; hooks ⇒ relay + approvals;
//! background ⇒ lifecycle via the short id; otherwise nothing, with notes.
//!
//! Callers build a [`CapsInput`] from whatever they know (the data layer does
//! it once per summary) and call [`compute`]. Adding fields to `CapsInput` is
//! fine (give them a `Default`); keep `compute` pure.

use super::model::{
    ControlCaps, ControlChannel, InterruptCap, LifecycleCaps, MessageCap, SetModeCap,
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
    /// Hook events arrived from this session recently (plugin installed and
    /// the session loaded it).
    pub hooks_active: bool,
    /// The foreign-control plugin is installed at user scope.
    pub hooks_installed: bool,
    /// tmux pane id (`%12`) when the session runs inside tmux and the pane was
    /// found.
    pub tmux_pane: Option<String>,
    /// Short background job id usable with `claude stop|respawn|rm|logs|attach`.
    pub background_job: Option<String>,
    /// Experimental PTY `claude attach` is compiled in and enabled.
    pub pty_enabled: bool,
}

/// What the data layer (B2) knows about a session when it builds a summary.
/// B2 fills these from its sources; [`input_for`] (B4) turns them plus the
/// foreign-control state into a [`CapsInput`]. Extend with defaulted fields.
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

/// Build the capability input for a session from the data layer's facts and
/// the foreign-control state (hooks traffic, plugin, tmux, PTY).
///
/// Owner: **B4** — the foundation version: hooks activity from
/// `server::hooks::is_active`, the pane id taken verbatim from the pidfile
/// target's `%pane` part (B4 verifies it against tmux), PTY per the
/// experimental feature switch.
pub fn input_for(state: &AppState, facts: &SessionFacts) -> CapsInput {
    let background_job = facts.job_id.clone().or_else(|| {
        (facts.kind.as_deref() == Some("background") && facts.live).then(|| facts.id.clone())
    });
    CapsInput {
        owned: facts.owned && !facts.bridge_thread,
        bridge_thread: facts.bridge_thread,
        live: facts.live,
        hooks_active: crate::server::hooks::is_active(state, &facts.id),
        hooks_installed: state.plugin.installed_cached(),
        tmux_pane: facts
            .tmux
            .as_deref()
            .and_then(|t| t.rsplit_once('.').map(|(_, pane)| pane.to_string()))
            .filter(|p| p.starts_with('%')),
        background_job,
        pty_enabled: super::foreign_injection_enabled(),
    }
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
        return ControlCaps {
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
            notes: if input.bridge_thread {
                vec!["Driven through the HTTP bridge.".into()]
            } else {
                Vec::new()
            },
        };
    }

    let mut caps = ControlCaps {
        lifecycle,
        hooks_active: input.hooks_active,
        // Any session with a transcript can be continued as an owned session:
        // a live foreign one is forked, an ended one resumes in place.
        fork: true,
        continue_owned: true,
        ..Default::default()
    };

    if !input.live {
        caps.notes.push(
            "This session has ended. Continue it here to pick up the conversation as an \
             owned session."
                .into(),
        );
        return caps;
    }

    if let Some(_pane) = &input.tmux_pane {
        caps.channel = ControlChannel::Tmux;
        caps.message = MessageCap::Relay;
        caps.interrupt = InterruptCap::Immediate;
        caps.slash_commands = true;
        caps.set_model = true;
        caps.set_effort = true;
        caps.approve = input.hooks_active;
        caps.answer = input.hooks_active;
        caps.set_mode = if input.hooks_active {
            SetModeCap::OnPrompt
        } else {
            SetModeCap::None
        };
        caps.notes.push(
            "Typed into the session's tmux pane (best effort). /model and /effort change your \
             user settings."
                .into(),
        );
        if !input.hooks_active {
            caps.notes.push(
                "Approvals need foreign-session control (the hooks plugin); without it use the \
                 key buttons."
                    .into(),
            );
        }
        return caps;
    }

    if input.hooks_active {
        caps.channel = ControlChannel::Hooks;
        caps.message = MessageCap::Relay;
        caps.approve = true;
        caps.answer = true;
        caps.interrupt = InterruptCap::NextTool;
        caps.set_mode = SetModeCap::OnPrompt;
        caps.notes.push(
            "Messages are relayed through hooks: delivered after the current tool call, or \
             when the session goes idle."
                .into(),
        );
        return caps;
    }

    if input.pty_enabled && input.background_job.is_some() {
        caps.channel = ControlChannel::Pty;
        caps.message = MessageCap::Relay;
        caps.notes.push(
            "Experimental: typed into the background session through `claude attach`.".into(),
        );
        return caps;
    }

    if !input.hooks_installed {
        caps.notes.push(
            "Install foreign-session control on the desktop to approve and message this \
             session from here."
                .into(),
        );
    } else {
        caps.notes.push(
            "No hook traffic from this session yet — it may have started before foreign \
             control was installed. Restart it to enable control."
                .into(),
        );
    }
    caps.notes
        .push("Or continue here as an owned session (fork) for full control.".into());
    caps
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(c.interrupt, InterruptCap::Immediate);
        assert_eq!(c.set_mode, SetModeCap::Any);
        assert!(c.lifecycle.stop);
    }

    #[test]
    fn hooks_relay_and_approve() {
        let c = compute(&CapsInput {
            live: true,
            hooks_active: true,
            hooks_installed: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Hooks);
        assert_eq!(c.message, MessageCap::Relay);
        assert!(c.approve && c.answer && c.fork && c.continue_owned);
        assert_eq!(c.interrupt, InterruptCap::NextTool);
        assert_eq!(c.set_mode, SetModeCap::OnPrompt);
    }

    #[test]
    fn tmux_without_hooks_cannot_approve() {
        let c = compute(&CapsInput {
            live: true,
            tmux_pane: Some("%3".into()),
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Tmux);
        assert!(!c.approve && c.slash_commands);
        let c = compute(&CapsInput {
            live: true,
            tmux_pane: Some("%3".into()),
            hooks_active: true,
            ..Default::default()
        });
        assert!(c.approve && c.answer);
    }

    #[test]
    fn nothing_available_explains_why() {
        let c = compute(&CapsInput {
            live: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::None);
        assert_eq!(c.message, MessageCap::None);
        assert!(!c.approve);
        assert!(c
            .notes
            .iter()
            .any(|n| n.contains("Install foreign-session control")));
        assert!(c.continue_owned);
    }

    #[test]
    fn background_lifecycle_uses_the_short_id() {
        let c = compute(&CapsInput {
            live: true,
            background_job: Some("a1b2c3".into()),
            ..Default::default()
        });
        assert!(c.lifecycle.stop && c.lifecycle.respawn && c.lifecycle.rm);
        let c = compute(&CapsInput {
            live: true,
            background_job: Some("a1b2c3".into()),
            pty_enabled: true,
            ..Default::default()
        });
        assert_eq!(c.channel, ControlChannel::Pty);
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
                id: "s".into(),
                live: true,
                kind: Some("background".into()),
                tmux: Some("main:@1.%7".into()),
                ..Default::default()
            },
        );
        assert_eq!(input.tmux_pane.as_deref(), Some("%7"));
        assert_eq!(input.background_job.as_deref(), Some("s"));
        assert!(!input.hooks_active);
        let bridge = input_for(
            &state,
            &SessionFacts {
                id: "b".into(),
                owned: true,
                bridge_thread: true,
                live: true,
                ..Default::default()
            },
        );
        assert!(!bridge.owned && bridge.bridge_thread);
        assert_eq!(compute(&bridge).channel, ControlChannel::Owned);
    }

    #[test]
    fn ended_sessions_can_only_be_continued() {
        let c = compute(&CapsInput::default());
        assert_eq!(c.channel, ControlChannel::None);
        assert!(c.continue_owned);
        assert!(!c.lifecycle.stop);
    }
}
