//! What another computer may do on this one (docs/PLAN-MACHINES.md §5.4):
//! five grants, each with a column for a direct path on the local network
//! and one for everything else (relayed, or a public address).

use serde::{Deserialize, Serialize};

/// One grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Grant {
    /// List sessions, read transcripts, see pending items.
    View,
    /// Send messages, answer ordinary approvals, interrupt, stop, set model,
    /// effort and mode (never bypass).
    Control,
    /// Move sessions to and from this machine, keep copies here.
    Move,
    /// Run commands and tasks here (Phase 4).
    Run,
    /// What a device's dangerous grant allows: bypass permissions, dangerous
    /// approvals, raw keys, deleting.
    Dangerous,
}

impl Grant {
    pub const ALL: [Grant; 5] = [
        Grant::View,
        Grant::Control,
        Grant::Move,
        Grant::Run,
        Grant::Dangerous,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Grant::View => "view",
            Grant::Control => "control",
            Grant::Move => "move",
            Grant::Run => "run",
            Grant::Dangerous => "dangerous",
        }
    }
}

/// A grant on the local network and over the internet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantPair {
    #[serde(default)]
    pub lan: bool,
    #[serde(default)]
    pub internet: bool,
}

impl GrantPair {
    pub const fn both(on: bool) -> Self {
        Self {
            lan: on,
            internet: on,
        }
    }

    /// Whether it holds for a request on the local network (`lan`) or not.
    pub fn holds(self, lan: bool) -> bool {
        if lan {
            self.lan
        } else {
            self.internet
        }
    }
}

/// Every grant one computer gives another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grants {
    #[serde(default)]
    pub view: GrantPair,
    #[serde(default)]
    pub control: GrantPair,
    #[serde(default, rename = "move")]
    pub moves: GrantPair,
    #[serde(default)]
    pub run: GrantPair,
    #[serde(default)]
    pub dangerous: GrantPair,
}

impl Default for Grants {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Grants {
    /// A newly paired machine (both are yours): see, drive and move, on the
    /// local network and over the internet; run and dangerous off.
    pub const fn defaults() -> Self {
        Self {
            view: GrantPair::both(true),
            control: GrantPair::both(true),
            moves: GrantPair::both(true),
            run: GrantPair::both(false),
            dangerous: GrantPair::both(false),
        }
    }

    /// Nothing at all.
    pub const fn none() -> Self {
        Self {
            view: GrantPair::both(false),
            control: GrantPair::both(false),
            moves: GrantPair::both(false),
            run: GrantPair::both(false),
            dangerous: GrantPair::both(false),
        }
    }

    pub fn get(&self, grant: Grant) -> GrantPair {
        match grant {
            Grant::View => self.view,
            Grant::Control => self.control,
            Grant::Move => self.moves,
            Grant::Run => self.run,
            Grant::Dangerous => self.dangerous,
        }
    }

    /// Whether `grant` holds for a request that came over the local network
    /// (`lan`) or not.
    pub fn allows(&self, grant: Grant, lan: bool) -> bool {
        self.get(grant).holds(lan)
    }

    /// The grants a machine actually gets: driving needs seeing, and moving
    /// and running need driving, so a dependent grant without its base is
    /// dropped rather than half-honoured.
    pub fn normalized(mut self) -> Self {
        for lan in [true, false] {
            let base_view = self.view.holds(lan);
            let base_control = self.control.holds(lan) && base_view;
            set(&mut self.control, lan, base_control);
            for pair in [&mut self.moves, &mut self.run, &mut self.dangerous] {
                let v = pair.holds(lan) && base_control;
                set(pair, lan, v);
            }
        }
        self
    }
}

fn set(pair: &mut GrantPair, lan: bool, value: bool) {
    if lan {
        pair.lan = value;
    } else {
        pair.internet = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_let_a_paired_machine_see_drive_and_move_but_not_run_or_endanger() {
        let g = Grants::defaults();
        for lan in [true, false] {
            assert!(g.allows(Grant::View, lan));
            assert!(g.allows(Grant::Control, lan));
            assert!(g.allows(Grant::Move, lan));
            assert!(!g.allows(Grant::Run, lan));
            assert!(!g.allows(Grant::Dangerous, lan));
        }
    }

    #[test]
    fn the_columns_are_independent() {
        let mut g = Grants::none();
        g.view = GrantPair {
            lan: true,
            internet: false,
        };
        assert!(g.allows(Grant::View, true));
        assert!(!g.allows(Grant::View, false));
    }

    #[test]
    fn a_grant_without_its_base_is_dropped() {
        let mut g = Grants::none();
        g.dangerous = GrantPair::both(true);
        g.control = GrantPair::both(true);
        let n = g.normalized();
        assert_eq!(n.control, GrantPair::both(false), "driving needs seeing");
        assert_eq!(n.dangerous, GrantPair::both(false));
        let mut ok = Grants::none();
        ok.view = GrantPair::both(true);
        ok.control = GrantPair::both(true);
        ok.run = GrantPair {
            lan: true,
            internet: false,
        };
        assert_eq!(ok.normalized(), ok);
    }

    #[test]
    fn the_wire_names_the_move_grant_move() {
        let v = serde_json::to_value(Grants::defaults()).unwrap();
        assert_eq!(v["move"]["lan"], true);
        let back: Grants = serde_json::from_value(serde_json::json!({
            "view": {"lan": true}, "move": {"internet": true}
        }))
        .unwrap();
        assert!(back.view.lan && !back.view.internet);
        assert!(back.moves.internet);
        assert_eq!(back.run, GrantPair::default());
    }
}
