//! What the source and the holder of a copy say to each other
//! (`/peer/copies/*`). Everything here comes from another computer: the
//! holder checks every path, size, hash and offset before the file system
//! sees one (`target`).

use serde::{Deserialize, Serialize};

use super::check::Mark;
use crate::machines::lease::WireLease;
use crate::machines::moves::wire::AccountAsk;
use crate::machines::moves::Carried;

/// One of the session's files as the source has it now.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileNow {
    /// Under the config directory, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Its time there (ms).
    pub mtime: i64,
    /// Append-only (`.jsonl`): it may travel as the bytes appended.
    pub log: bool,
}

/// `POST /peer/copies/{session}/offer`: what the source has now; the holder
/// answers what it holds ([`Holdings`]) and writes nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Offer {
    pub files: Vec<FileNow>,
    /// Every file's bytes there.
    pub bytes: u64,
    /// The session's lease there (it lives there): the holder learns it.
    pub lease: Option<WireLease>,
}

/// One file the holder has of a copy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HeldFile {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    /// An append-only file's mark at its size.
    pub mark: Option<Mark>,
}

/// The holder's answer to an offer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Holdings {
    pub files: Vec<HeldFile>,
    /// What it keeps of this computer's copies at most.
    pub cap: u64,
    /// When the copy was last made current there.
    pub copied_at: Option<i64>,
}

/// What a round does with one file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpKind {
    /// Append the bytes to what the holder has (its mark is `base`).
    #[default]
    Append,
    /// Replace (or write) the whole file.
    Whole,
    /// The source has it no more.
    Remove,
}

/// One file of a round.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Op {
    pub path: String,
    pub kind: OpKind,
    /// Its bytes in the round's blob.
    pub offset: u64,
    pub len: u64,
    /// SHA-256 of those bytes.
    pub sha256: String,
    /// `append`: the mark the holder must have.
    pub base: Option<Mark>,
    /// `append`: the source's mark at the new size, which the result must
    /// have.
    pub after: Option<Mark>,
    /// Its time there (ms).
    pub mtime: i64,
    /// Append-only: the holder keeps its mark.
    pub log: bool,
}

/// The blob a round's bytes travel in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Blob {
    pub size: u64,
    pub sha256: String,
}

/// `POST /peer/copies/{session}/rounds/{round}/apply`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Apply {
    pub account: AccountAsk,
    /// Its account's id there (`default`, `acc_…`): the copy's folder.
    pub account_id: String,
    /// Where it works there.
    pub cwd: String,
    pub title: Option<String>,
    pub settings: Carried,
    /// The time of the turn it holds (the transcript's last write there).
    pub turn_at: i64,
    /// What the source has, all files.
    pub bytes: u64,
    pub files: usize,
    pub blob: Option<Blob>,
    pub ops: Vec<Op>,
}

/// The holder's answer to a round.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Applied {
    pub copied_at: i64,
    /// Files whose part did not fit what is held: the next round sends them
    /// whole.
    pub resend: Vec<String>,
    /// What the copy holds after it.
    pub bytes: u64,
    pub files: usize,
    pub complete: bool,
    /// Copies of this computer's sessions the cap took away to make room.
    pub evicted: Vec<String>,
}

/// `GET /peer/copies/{session}`: whether a copy is held, and as of when.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CopyThere {
    pub held: bool,
    pub copied_at: Option<i64>,
    pub turn_at: Option<i64>,
    pub bytes: u64,
    pub files: usize,
    pub complete: bool,
}
