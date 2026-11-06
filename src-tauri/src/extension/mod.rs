//! The Mother Claude browser extension's desktop side (docs/BROWSER.md, the
//! wire contract in docs/BROWSER-PROTOCOL.md, plan §5 and §10).
//!
//! * [`sites`] — site modes (`off`/`read`/`act`), the sensitive list,
//!   settings, standing grants.
//! * [`danger`] — which actions need a person's approval.
//! * [`page`] — what the desktop knows about each tab (refs, secret fields).
//! * [`redact`] — what the extension reads is redacted again before it
//!   reaches a model.
//!
//! Nothing here ever writes under a config directory.

pub mod danger;
pub mod page;
pub mod redact;
pub mod sites;
