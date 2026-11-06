//! Teams of sessions (docs/PLAN-BROWSER-AND-TEAMS.md §6–§9): workflow
//! templates and roles, the task board, role sessions Mother Claude launches
//! and stops, the model router and the scheduler.
//!
//! Contract between the areas, fixed before they were built:
//!
//! * [`router`] chooses a model and effort for a task ([`router::choose`]).
//! * [`capacity`] decides whether a role session may start now
//!   ([`capacity::admit`]) and says why not.
//! * [`metrics`] records what each task cost and how it ended
//!   ([`metrics::record_outcome`]).
//!
//! The teams side (templates, board, launcher, ownership guard, worktrees)
//! calls these three and nothing else of the scheduler side; the scheduler
//! side never launches or stops a session itself.

pub mod capacity;
pub mod metrics;
pub mod router;

// The scheduler side behind those three (docs/ROUTING.md): host metrics,
// settings, the routing policy, outcomes, budgets, reports and its routes.
pub mod budget;
pub mod classify;
pub mod host;
pub mod limits;
pub mod outcomes;
pub mod policy;
pub mod reports;
pub mod scheduler;
