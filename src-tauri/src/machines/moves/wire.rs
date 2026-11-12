//! What the source and the target of a move say to each other
//! (`/peer/moves/*`). Everything here comes from another machine: the
//! target checks every id, name and path before git or the file system
//! sees one (`target::*`).

use serde::{Deserialize, Serialize};

use super::{Carried, Check, MachineRef};

/// The account a move runs under, as the source names it: the target takes
/// its account of the same name (case-insensitive), else its Default for the
/// source's Default (decision 3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountAsk {
    pub name: String,
    pub is_default: bool,
}

/// The code a move carries, as the source asks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodeAsk {
    /// `bundle` | `remote`.
    pub mode: String,
    /// `checkout` | `worktree`.
    pub apply: String,
    /// The remote of a `remote` move (`origin`).
    pub remote: Option<String>,
    /// The session's folder under its repository's top, `/`-separated.
    pub rel: String,
    /// The snapshot's branch (`refs/heads/…`).
    pub branch: Option<String>,
    /// The source's `origin` URL.
    pub origin: Option<String>,
    /// The files git ignores that travel with it (the project's list),
    /// under the repository's top: carrying them needs the target's
    /// `dangerous` grant for the source.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored: Vec<String>,
}

/// `POST /peer/moves/preflight`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Ask {
    pub session: String,
    pub as_copy: bool,
    pub account: AccountAsk,
    /// The source's system.
    pub os: String,
    /// Where the session works on the source.
    pub cwd: String,
    /// The folder the user chose on the target.
    pub folder: Option<String>,
    /// Where the source knows it goes on the target (it came from there).
    pub hint: Option<String>,
    pub code: Option<CodeAsk>,
    pub trust: bool,
    pub permission_mode: Option<String>,
    /// About how much travels.
    pub bytes: u64,
    /// The source's Claude Code version.
    pub claude: Option<String>,
    /// The epoch of the session's lease on the source (0: none): a target
    /// that knows a newer home says so.
    pub epoch: u64,
}

/// The account the target found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountThere {
    pub id: String,
    pub name: String,
    /// `None`: never checked there.
    pub signed_in: Option<bool>,
}

/// The folder the target found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FolderThere {
    pub path: String,
    /// `chosen` | `root` (a root, or a folder pair) | `hint` | `same`.
    pub how: String,
    /// Claude Code trusts it there (else the trust must be carried).
    pub trusted: bool,
}

/// The target's checkout.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodeThere {
    /// The folder is in a git repository.
    pub repo: bool,
    pub clean: bool,
    pub parked: bool,
    pub changed: usize,
    pub untracked: usize,
    pub busy: Option<String>,
    pub branch: Option<String>,
    /// The tip of the snapshot's branch there.
    pub branch_tip: Option<String>,
    /// That branch is checked out in another worktree there.
    pub elsewhere: Option<String>,
    pub origin: Option<String>,
    pub remotes: Vec<String>,
    /// What a bundle may leave out.
    pub haves: Vec<String>,
    pub shallow: bool,
}

/// The target's session of that id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionThere {
    /// The target has its files.
    pub here: bool,
    /// A process runs it there.
    pub live: bool,
}

/// The target's answer to a preflight.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Answer {
    pub blockers: Vec<Check>,
    pub warnings: Vec<Check>,
    pub machine: MachineRef,
    pub account: Option<AccountThere>,
    pub folder: Option<FolderThere>,
    /// Folders there that may be the project, to choose from.
    pub candidates: Vec<String>,
    /// Where the session will work there.
    pub cwd: Option<String>,
    pub code: Option<CodeThere>,
    pub session: SessionThere,
    pub claude: Option<String>,
    pub free_bytes: Option<u64>,
    /// The copy of the session this computer keeps current for the source
    /// (docs/PLAN-MACHINES.md §5.9): a move lands from it. Absent from an
    /// older build, or when it keeps none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy: Option<crate::machines::copies::wire::CopyThere>,
}

/// One file of a package.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileSpec {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// The code of a package.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodeSpec {
    pub mode: String,
    pub apply: String,
    pub remote: Option<String>,
    pub rel: String,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub snapshot: Option<String>,
    pub index_tree: Option<String>,
    pub changed: usize,
    pub untracked: usize,
    pub warnings: Vec<String>,
    /// The files git ignores in `ignored.tar.gz`, under the repository's
    /// top (written into the landing once the code is there).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored: Vec<String>,
}

/// `POST /peer/moves/{id}`: the package and everything the target needs to
/// land it and go on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Manifest {
    pub session: String,
    /// The lease's epoch once committed (a copy: 0).
    pub epoch: u64,
    pub as_copy: bool,
    pub title: Option<String>,
    pub account: AccountAsk,
    pub os: String,
    pub cwd: String,
    pub folder: Option<String>,
    pub hint: Option<String>,
    pub files: Vec<FileSpec>,
    pub code: Option<CodeSpec>,
    pub settings: Carried,
    pub held: Vec<String>,
    pub recipe: Vec<String>,
    pub keep_driving: bool,
    pub trust: bool,
    pub when: String,
    pub by: String,
    /// The transcript as packed.
    pub transcript_bytes: u64,
    pub transcript_sha256: String,
    /// The conversation lands from the copy the target keeps current
    /// (§5.9): every file of the session here, which the target checks its
    /// copy against before it lands — `session.tar.gz` does not travel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_copy: Option<FromCopy>,
}

/// The session's files, as a move that lands from the target's copy lists
/// them: `name` is the path under the config directory (`/`-separated).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FromCopy {
    pub files: Vec<FileSpec>,
}

/// The target's answer to a landing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Landed {
    pub landed: bool,
    pub target_cwd: String,
    pub target_folder: Option<String>,
    pub target_session: String,
    pub account: Option<String>,
    pub landing: super::LandingPart,
    pub code: Option<super::super::code::Applied>,
    pub warnings: Vec<Check>,
    /// A copy: forked there already.
    pub resumed: bool,
    pub resume_error: Option<String>,
}

/// What a `/peer/leases` exchange carries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LeasesBody {
    pub leases: Vec<super::super::lease::WireLease>,
}

/// One move a lease let the receiver go on with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Resumed {
    #[serde(rename = "move")]
    pub move_id: String,
    pub session: String,
    pub ok: bool,
    pub error: Option<String>,
}
