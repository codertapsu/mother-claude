//! The prefix check of an append-only file (docs/PLAN-MACHINES.md §5.9):
//! how two computers agree that the copy one holds is the start of the file
//! the other has, without reading a transcript of gigabytes whole every turn.
//!
//! A file's **mark** at a size `n` is that size and the SHA-256 of a fixed
//! tag, `n`, the first [`WINDOW`] bytes and the [`WINDOW`] bytes that end at
//! `n` — at most two reads of 64 KiB on each side, whatever the file's size
//! (a file of up to two windows is checked whole). The copy's holder keeps the
//! mark of what it holds; before it appends what grew, the sender computes the
//! mark of its own file at that size and both must agree:
//!
//! - **appended** (the usual turn): the sender's file is longer and its mark
//!   at the holder's size is the holder's — only the new bytes travel, with
//!   their SHA-256, and the holder checks the mark of the result against the
//!   sender's mark at the new size before it keeps it;
//! - **truncated**: the sender's file is shorter than the copy;
//! - **rewritten**: its first bytes, or the ones just before the copy's end,
//!   differ.
//!
//! The last two send the whole file again (and so does any doubt: a copy
//! that is missing, a mark that can't be read). A change inside the middle of
//! a file that keeps its size and both windows is not seen — Claude Code
//! only appends to these files, and every byte that travelled was checked
//! when it arrived; a move checks every file's SHA-256 whole.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The bytes read at each end of a file.
pub const WINDOW: u64 = 64 * 1024;
/// What every check starts with (its version).
const TAG: &[u8] = b"mother-claude/copy-check/1";

/// A file's mark at a size.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub size: u64,
    /// SHA-256 (hex) of the tag, the size, the first window and the window
    /// that ends at `size`.
    pub check: String,
}

impl Mark {
    /// A mark well formed: a 64-hex check.
    pub fn well_formed(&self) -> bool {
        super::is_sha256(&self.check)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn digest(size: u64, head: &[u8], tail: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(TAG);
    h.update(size.to_le_bytes());
    h.update((head.len() as u64).to_le_bytes());
    h.update(head);
    h.update((tail.len() as u64).to_le_bytes());
    h.update(tail);
    hex(&h.finalize())
}

/// The mark of the first `size` bytes of `bytes` (tests, and what a sender
/// holds in memory).
pub fn mark_of_bytes(bytes: &[u8], size: u64) -> Option<Mark> {
    if size > bytes.len() as u64 {
        return None;
    }
    let n = size as usize;
    let w = WINDOW as usize;
    let head = &bytes[..n.min(w)];
    let tail = &bytes[n.saturating_sub(w)..n];
    Some(Mark {
        size,
        check: digest(size, head, tail),
    })
}

/// The mark of the first `size` bytes of the file at `path`: `None` when the
/// file is shorter, or can't be read. Reads at most two windows.
pub fn mark_of(path: &Path, size: u64) -> Option<Mark> {
    let mut f = std::fs::File::open(path).ok()?;
    if f.metadata().ok()?.len() < size {
        return None;
    }
    let head_len = size.min(WINDOW) as usize;
    let mut head = vec![0u8; head_len];
    f.read_exact(&mut head).ok()?;
    let tail_from = size.saturating_sub(WINDOW);
    let tail = if tail_from == 0 {
        head.clone()
    } else {
        let mut tail = vec![0u8; (size - tail_from) as usize];
        f.seek(SeekFrom::Start(tail_from)).ok()?;
        f.read_exact(&mut tail).ok()?;
        tail
    };
    Some(Mark {
        size,
        check: digest(size, &head, &tail),
    })
}

/// What the sender does with an append-only file, from its own size and the
/// mark the holder has (`None`: the holder has none of it). `mine_at` gives
/// the sender's mark at a size (read lazily: only when it can be used).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// The holder has it as it is.
    Same,
    /// Only what grew travels: the bytes from `from` on.
    Append { from: u64 },
    /// The whole file travels (the holder has none, or another file).
    Whole,
}

/// See [`Plan`].
pub fn plan(size: u64, held: Option<&Mark>, mine_at: impl Fn(u64) -> Option<Mark>) -> Plan {
    let Some(held) = held else {
        return Plan::Whole;
    };
    if held.size > size {
        // Truncated here: what the holder has is no start of this file.
        return Plan::Whole;
    }
    match mine_at(held.size) {
        Some(m) if m == *held => {
            if held.size == size {
                Plan::Same
            } else {
                Plan::Append { from: held.size }
            }
        }
        // Rewritten (or unreadable here): the whole of it.
        _ => Plan::Whole,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<u8> {
        (0..n)
            .flat_map(|i| format!("{{\"type\":\"user\",\"n\":{i}}}\n").into_bytes())
            .collect()
    }

    #[test]
    fn a_mark_reads_two_windows_and_checks_small_files_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let data = lines(20_000); // ~ 400 KiB: more than two windows
        assert!(data.len() as u64 > 3 * WINDOW);
        std::fs::write(&path, &data).unwrap();
        for size in [0, 1, 100, WINDOW, WINDOW + 7, 2 * WINDOW, data.len() as u64] {
            assert_eq!(
                mark_of(&path, size),
                mark_of_bytes(&data, size),
                "size {size}"
            );
        }
        assert_eq!(mark_of(&path, data.len() as u64 + 1), None, "past its end");
        assert_eq!(mark_of(&dir.path().join("none"), 0), None);
        // Different sizes, different marks (even of the same bytes).
        assert_ne!(mark_of_bytes(&data, 10), mark_of_bytes(&data, 11));
        assert!(mark_of_bytes(&data, 10).unwrap().well_formed());
        // A small file is checked whole: any byte counts.
        let mut small = data[..1000].to_vec();
        let before = mark_of_bytes(&small, 1000).unwrap();
        small[500] ^= 1;
        assert_ne!(mark_of_bytes(&small, 1000).unwrap(), before);
    }

    #[test]
    fn appended_bytes_travel_alone_and_truncation_or_a_rewrite_sends_it_whole() {
        let data = lines(20_000);
        let held_at = 250_000u64;
        let held = mark_of_bytes(&data, held_at).unwrap();
        let mine = |d: &[u8]| {
            let d = d.to_vec();
            move |n: u64| mark_of_bytes(&d, n)
        };

        // The usual turn: it grew; only the new bytes go.
        let grown = data.len() as u64;
        assert_eq!(
            plan(grown, Some(&held), mine(&data)),
            Plan::Append { from: held_at }
        );
        // Nothing new.
        assert_eq!(
            plan(held_at, Some(&held), mine(&data[..held_at as usize])),
            Plan::Same
        );
        // The holder has none of it.
        assert_eq!(plan(grown, None, mine(&data)), Plan::Whole);

        // Truncated here: shorter than what the holder has.
        assert_eq!(
            plan(
                held_at - 1,
                Some(&held),
                mine(&data[..held_at as usize - 1])
            ),
            Plan::Whole
        );
        // Truncated, then grew past the old size again: the bytes before the
        // copy's end differ.
        let mut regrown = data[..200_000].to_vec();
        regrown.extend(lines(5_000).iter().map(|b| b.to_ascii_uppercase()));
        assert!(regrown.len() as u64 > held_at);
        assert_eq!(
            plan(regrown.len() as u64, Some(&held), mine(&regrown)),
            Plan::Whole
        );
        // Rewritten at its start, same size.
        let mut head = data.clone();
        head[3] = b'X';
        assert_eq!(plan(grown, Some(&held), mine(&head)), Plan::Whole);
        // Rewritten just before the copy's end, same size.
        let mut tail = data.clone();
        tail[held_at as usize - 10] = b'X';
        assert_eq!(plan(grown, Some(&held), mine(&tail)), Plan::Whole);
        // Unreadable here: whole.
        assert_eq!(plan(grown, Some(&held), |_| None), Plan::Whole);
    }
}
