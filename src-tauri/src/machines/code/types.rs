//! The move's requests, records and errors (docs/PLAN-MACHINES.md §5.10).
//!
//! `meta.json` is written by the source and read by the target, so it comes
//! from another machine: `SnapshotMeta::validate` checks every id, ref and
//! path in it before git or the file system sees one.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The snapshot's record in a move folder.
pub const META: &str = "meta.json";
/// The target's record of what it did with a move.
pub const APPLY: &str = "apply.json";
/// The bundle in a move folder (machine to machine).
pub const BUNDLE: &str = "move.bundle";
/// The records' format.
pub const FORMAT: u32 = 1;
/// At most this many commits are offered as haves.
pub const MAX_HAVES: usize = 500;

/// Why a step of a move failed (a stop by design is an [`ApplyStop`]).
#[derive(Debug, thiserror::Error)]
pub enum CodeError {
    #[error("Git is not installed, or could not be started: {0}")]
    NoGit(#[source] std::io::Error),
    #[error("git {command} failed (exit {code}): {stderr}")]
    Git {
        command: String,
        code: i32,
        stderr: String,
    },
    #[error("{} is not in a git working tree.", .0.display())]
    NotARepository(PathBuf),
    #[error("A git operation is in progress here ({0}): finish or abort it first.")]
    InProgress(String),
    #[error("The index has unmerged files: finish the merge first.")]
    Unmerged,
    #[error(
        "The snapshot does not match git status (missing: {}; extra: {}).",
        .missing.join(", "),
        .extra.join(", ")
    )]
    StatusMismatch {
        missing: Vec<String>,
        extra: Vec<String>,
    },
    #[error("Git could not read part of the working tree, so the snapshot would lose files: {0}")]
    Unreadable(String),
    #[error("Git's private hooks folder could not be emptied: {0}")]
    Hooks(String),
    #[error("The move's record is not valid: {0}.")]
    BadMove(String),
    #[error("This move belongs to another session.")]
    WrongSession,
    #[error("The fetched snapshot does not match the move's record.")]
    FetchMismatch,
    #[error("There is no remote named {0} in this repository.")]
    UnknownRemote(String),
    #[error("The move was not applied here.")]
    NotApplied,
    #[error("The move was undone already.")]
    AlreadyUndone,
    #[error("The files changed after the move, and undo would lose that: {0}")]
    ChangedSinceApply(String),
    #[error("Invalid {what}: {value:?}")]
    Invalid { what: &'static str, value: String },
    #[error("Could not {what}: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
}

/// `map_err` for an I/O step, named in plain words.
pub(crate) fn io_err(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> CodeError {
    let what = what.into();
    move |source| CodeError::Io { what, source }
}

// ─── haves and the snapshot ────────────────────────────────────────────────────

/// What the target has, sent to the source before a bundle move.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Haves {
    #[serde(default)]
    pub haves: Vec<String>,
    #[serde(default)]
    pub shallow: bool,
    #[serde(default)]
    pub machine: String,
}

/// How the snapshot travels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transfer {
    /// A bundle in the move folder, without what the target has.
    Bundle,
    /// Hidden refs pushed to `remote` (and the branch, when asked).
    Remote { remote: String, push_branch: bool },
}

#[derive(Clone, Debug)]
pub struct SnapshotRequest {
    pub session: String,
    pub epoch: u64,
    /// This machine's name (made safe for a ref).
    pub machine: String,
    pub transfer: Transfer,
    /// The target's [`Haves`] (bundle moves).
    pub haves: Vec<String>,
    /// The move folder: `meta.json`, the bundle, LFS objects.
    pub out_dir: PathBuf,
    /// Copy the Git LFS objects the snapshot names into the move folder.
    pub carry_lfs: bool,
    /// Chain this session's earlier snapshots the target has (F6).
    pub chain: bool,
    /// The branch the snapshot names (`refs/heads/…`) when it is not the
    /// checkout's own: a session that lived in a worktree goes home to its
    /// home branch, which the move recorded (plan §5.10 "Coming back").
    pub branch: Option<String>,
    /// A code sync's step (plan §5.15), `session` being the sync's id: the
    /// commits say so.
    pub sync: bool,
}

impl SnapshotRequest {
    pub fn new(session: &str, epoch: u64, machine: &str, out_dir: impl Into<PathBuf>) -> Self {
        Self {
            session: session.to_string(),
            epoch,
            machine: machine.to_string(),
            transfer: Transfer::Bundle,
            haves: Vec::new(),
            out_dir: out_dir.into(),
            carry_lfs: true,
            chain: true,
            branch: None,
            sync: false,
        }
    }
}

/// A move's two refs (kept so gc keeps the commits).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveRefs {
    pub snap: String,
    pub index: String,
}

impl MoveRefs {
    /// `refs/mother-claude/moves/<session>/<epoch>/{snap,index}`.
    pub fn for_move(session: &str, epoch: &str) -> Self {
        let base = format!("refs/mother-claude/moves/{session}/{epoch}");
        Self {
            snap: format!("{base}/snap"),
            index: format!("{base}/index"),
        }
    }

    /// The hidden refs on a remote: `refs/mother-claude/<machine>/<session>/…`.
    pub fn hidden(machine: &str, session: &str) -> Self {
        let base = format!("refs/mother-claude/{machine}/{session}");
        Self {
            snap: format!("{base}/snap"),
            index: format!("{base}/index"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferKind {
    Bundle,
    Remote,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleInfo {
    pub file: String,
    pub size: u64,
    pub haves_offered: usize,
    pub haves_used: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    pub remote: String,
    pub hidden: MoveRefs,
    #[serde(default)]
    pub pushed_branch: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LfsInfo {
    pub pointers: usize,
    #[serde(default)]
    pub carried: Vec<String>,
    #[serde(default)]
    pub missing: Vec<String>,
    #[serde(default)]
    pub carried_disabled: bool,
}

/// The settings that decide how files look on disk.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RepoConfig {
    pub autocrlf: String,
    pub eol: Option<String>,
    pub file_mode: bool,
    pub symlinks: bool,
    pub ignore_case: bool,
    pub precompose_unicode: bool,
    pub longpaths: bool,
}

/// F1's cross-check: what `git status` says changed vs what the trees say.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WtCheck {
    pub ok: bool,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    /// The first try missed these and every file was hashed again.
    pub recomputed: bool,
    pub first_missing: Vec<String>,
}

/// A file as it is on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum Raw {
    Missing,
    Symlink {
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        case_name: Option<String>,
    },
    Dir {
        #[serde(default)]
        head: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        case_name: Option<String>,
    },
    File {
        size: u64,
        sha256: String,
        /// The SHA-256 with every CRLF read as LF.
        lf_sha256: String,
        crlf: bool,
        /// `None` where the file system has no exec bit (Windows).
        #[serde(default)]
        exec: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        case_name: Option<String>,
    },
    #[serde(other)]
    Other,
}

impl Raw {
    pub fn kind(&self) -> &'static str {
        match self {
            Raw::Missing => "missing",
            Raw::Symlink { .. } => "symlink",
            Raw::Dir { .. } => "dir",
            Raw::File { .. } => "file",
            Raw::Other => "other",
        }
    }

    /// The name on disk when it differs from git's only in case.
    pub fn case_name(&self) -> Option<&str> {
        match self {
            Raw::Symlink { case_name, .. }
            | Raw::Dir { case_name, .. }
            | Raw::File { case_name, .. } => case_name.as_deref(),
            _ => None,
        }
    }
}

/// One entry of the snapshot's tree and its bytes on the source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub mode: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub oid: String,
    pub raw: Raw,
}

/// What the move dialog warns about before a move (plan §5.10).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Warning {
    CaseCollision {
        pairs: Vec<(String, String)>,
    },
    WindowsNames {
        paths: Vec<String>,
    },
    LongPath {
        paths: Vec<String>,
    },
    Submodules {
        paths: Vec<String>,
        changed: Vec<String>,
    },
    Lfs,
    UnstagedModeChange {
        paths: Vec<String>,
    },
    UntrackedExec {
        paths: Vec<String>,
    },
    UntrackedSymlink {
        paths: Vec<String>,
    },
    ShallowSource,
    Unborn,
    #[serde(other)]
    Other,
}

impl Warning {
    pub fn kind(&self) -> &'static str {
        match self {
            Warning::CaseCollision { .. } => "case-collision",
            Warning::WindowsNames { .. } => "windows-names",
            Warning::LongPath { .. } => "long-path",
            Warning::Submodules { .. } => "submodules",
            Warning::Lfs => "lfs",
            Warning::UnstagedModeChange { .. } => "unstaged-mode-change",
            Warning::UntrackedExec { .. } => "untracked-exec",
            Warning::UntrackedSymlink { .. } => "untracked-symlink",
            Warning::ShallowSource => "shallow-source",
            Warning::Unborn => "unborn",
            Warning::Other => "other",
        }
    }
}

/// `meta.json`: everything the target needs to land and prove a move.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMeta {
    pub version: u32,
    pub session: String,
    pub epoch: String,
    pub machine: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub created: String,
    /// The source's branch (`refs/heads/…`), none when detached.
    #[serde(default)]
    pub branch: Option<String>,
    /// The source's HEAD, none on an unborn branch.
    #[serde(default)]
    pub base: Option<String>,
    pub idx_tree: String,
    pub wt_tree: String,
    pub snap: String,
    pub index_commit: String,
    pub refs: MoveRefs,
    pub transfer: TransferKind,
    #[serde(default)]
    pub bundle: Option<BundleInfo>,
    #[serde(default)]
    pub remote: Option<RemoteInfo>,
    #[serde(default)]
    pub lfs: Option<LfsInfo>,
    #[serde(default)]
    pub chained: Vec<String>,
    #[serde(default)]
    pub config: RepoConfig,
    /// `git status --porcelain=v1 -z -uall` on the source.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub manifest: Vec<ManifestEntry>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default)]
    pub wt_check: WtCheck,
    /// Hooks something wrote into the private hooks folder (F9), removed
    /// before the next git call.
    #[serde(default)]
    pub hook_intrusions: Vec<String>,
    #[serde(default)]
    pub timings: BTreeMap<String, u64>,
}

impl SnapshotMeta {
    pub fn warning_kinds(&self) -> Vec<&'static str> {
        self.warnings.iter().map(Warning::kind).collect()
    }
}

// ─── applying ──────────────────────────────────────────────────────────────────

/// Where the code lands on the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Landing {
    /// The checkout itself, when clean or parked for this session.
    Checkout,
    /// A new worktree in `<root>/<session>` (the checkout is never touched).
    Worktree { root: PathBuf },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LandingKind {
    Checkout,
    Worktree,
}

/// What a checkout holds: HEAD, the branch it points at, what is staged and
/// the working tree (untracked files included, ignored ones not) — the
/// state code sync compares and keeps (plan §5.15).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeState {
    /// `refs/heads/…`, none when detached.
    #[serde(default)]
    pub branch: Option<String>,
    /// HEAD's commit, none on an unborn branch.
    #[serde(default)]
    pub head: Option<String>,
    pub idx_tree: String,
    pub wt_tree: String,
}

/// When a checkout landing may move a branch that has commits the
/// snapshot's base lacks (its old tip is kept under the landing's undo
/// ref either way).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Reset {
    /// Never: the landing stops ([`ApplyStop::Diverged`]).
    #[default]
    Never,
    /// When the branch's tip is this commit: a code sync's last synced
    /// head, which the other side rewrote (an amend, a rebase).
    IfTip(String),
    /// Always: the side that lost a sync conflict (its tip is kept).
    Always,
}

#[derive(Clone, Debug)]
pub struct ApplyRequest {
    /// The session the move must belong to.
    pub session: String,
    pub landing: Landing,
    /// This machine's name.
    pub machine: String,
    /// Keep the hidden refs on the remote after fetching them.
    pub keep_remote_refs: bool,
    /// The remote to fetch from when it is named differently here.
    pub remote: Option<String>,
    /// A code sync's last synced state: the checkout counts as parked (F7)
    /// exactly when it holds this, and stops ([`ApplyStop::NotParked`])
    /// otherwise — even when clean. `None`: a move's rule (clean, or parked
    /// for a snapshot of this session).
    pub parked: Option<TreeState>,
    /// Whether a branch with commits the base lacks may be moved.
    pub reset: Reset,
}

impl ApplyRequest {
    pub fn new(session: &str, landing: Landing) -> Self {
        Self {
            session: session.to_string(),
            landing,
            machine: crate::claude::backup::host_name(),
            keep_remote_refs: false,
            remote: None,
            parked: None,
            reset: Reset::Never,
        }
    }
}

/// What happened to the branch in a checkout landing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BranchAction {
    Same,
    FastForward,
    Create,
    Unborn,
    Detach,
    /// Moved to the base although it had commits the base lacks
    /// ([`Reset`]); its old tip is kept under the undo ref.
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubmoduleState {
    NotInitialized,
    Same,
    /// The commit the snapshot names is not on this machine.
    Missing,
    Dirty,
    Updated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmoduleLanding {
    pub path: String,
    pub state: SubmoduleState,
    #[serde(default)]
    pub want: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
}

/// A landed move.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub landing: LandingKind,
    /// Where the session continues: the checkout or the new worktree.
    pub path: PathBuf,
    #[serde(default)]
    pub action: Option<BranchAction>,
    /// The epoch of the parked state this landed on (F7).
    #[serde(default)]
    pub parked: Option<String>,
    /// The worktree's branch (`refs/heads/mother-claude/<session>`).
    #[serde(default)]
    pub wt_branch: Option<String>,
    #[serde(default)]
    pub undo_ref: Option<String>,
    #[serde(default)]
    pub submodules: Vec<SubmoduleLanding>,
    #[serde(default)]
    pub lfs_installed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParkedTrees {
    pub epoch: String,
    pub wt_tree: String,
    pub idx_tree: String,
}

/// What undo puts back after a checkout landing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoInfo {
    pub stamp: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    #[serde(default)]
    pub prev_sym: Option<String>,
    #[serde(default)]
    pub prev_head: Option<String>,
    #[serde(default)]
    pub branch_old: Option<String>,
    pub action: BranchAction,
    #[serde(default)]
    pub parked: Option<ParkedTrees>,
}

/// What a stop offers instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Offer {
    Worktree,
    /// A worktree, or resetting the branch to the source's (its tip kept).
    WorktreeOrReset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyOutcome {
    Applied,
    Stopped,
}

/// `apply.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRecord {
    pub version: u32,
    pub session: String,
    pub epoch: String,
    pub machine: String,
    pub created: String,
    pub outcome: ApplyOutcome,
    pub landing: LandingKind,
    #[serde(default)]
    pub applied: Option<Applied>,
    #[serde(default)]
    pub undo: Option<UndoInfo>,
    #[serde(default)]
    pub stop: Option<String>,
    #[serde(default)]
    pub offer: Option<Offer>,
    #[serde(default)]
    pub fetched: bool,
    #[serde(default)]
    pub hook_intrusions: Vec<String>,
    #[serde(default)]
    pub undone: bool,
}

/// A landing that stopped by design (nothing in the checkout changed), or
/// failed.
#[derive(Debug, thiserror::Error)]
pub enum ApplyStop {
    #[error("The target checkout has changes of its own ({changed} paths).")]
    Dirty { changed: usize },
    #[error("A git operation is in progress in the target checkout ({what}).")]
    Busy { what: String },
    #[error("The target's {branch} has commits the snapshot's base lacks.")]
    Diverged { branch: String, target_tip: String },
    #[error("{branch} is checked out in another worktree ({}).", .worktree.display())]
    CheckedOutElsewhere { branch: String, worktree: PathBuf },
    /// Files the target ignores sit where the move would write: git would
    /// overwrite them, and nothing could bring them back.
    #[error(
        "The target has files of its own where the move would write ({}); it ignores them, so git would overwrite them.",
        .paths.join(", ")
    )]
    WouldOverwrite { paths: Vec<String> },
    #[error("{} exists and is not empty.", .path.display())]
    WorktreeExists { path: PathBuf },
    /// A code sync's checkout no longer holds its last synced state.
    #[error("The checkout changed since it was last synced ({what}).")]
    NotParked { what: String },
    #[error(transparent)]
    Failed(#[from] CodeError),
}

impl ApplyStop {
    /// What the move dialog offers instead (none for a failure).
    pub fn offer(&self) -> Option<Offer> {
        match self {
            ApplyStop::Dirty { .. }
            | ApplyStop::Busy { .. }
            | ApplyStop::CheckedOutElsewhere { .. }
            | ApplyStop::WouldOverwrite { .. } => Some(Offer::Worktree),
            ApplyStop::Diverged { .. } => Some(Offer::WorktreeOrReset),
            ApplyStop::WorktreeExists { .. }
            | ApplyStop::NotParked { .. }
            | ApplyStop::Failed(_) => None,
        }
    }

    /// A stop by design rather than a failure.
    pub fn is_stop(&self) -> bool {
        !matches!(self, ApplyStop::Failed(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Undone {
    pub landing: LandingKind,
    /// Files that came with the move but changed since: left in place.
    pub kept: Vec<String>,
}

// ─── verifying ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// Everything as on the source (line endings and filters explained).
    Pass,
    /// As on the source but for a known limit (submodule, mode, case, …).
    Limit,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyCheck {
    pub check: String,
    pub ok: bool,
    #[serde(default)]
    pub details: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub outcome: Outcome,
    pub limits: Vec<String>,
    pub fails: Vec<String>,
    /// Every kind of raw difference seen (`eol`, `filter`, …).
    pub raw_kinds: Vec<String>,
    pub checks: Vec<VerifyCheck>,
    pub config: RepoConfig,
    pub ms: u64,
}
