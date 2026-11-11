//! Folder pairs (docs/BACKUP.md "Folder pairs", docs/ACCOUNTS.md §5.11): a
//! folder on one machine and the folder that holds the same projects on
//! another — `/Users/marcus/dev` on the MacBook and `D:\dev` on the Windows
//! desktop — so every project under one maps to the other, both ways.
//!
//! A pair lives only in a set's index (`folderPairs`), which is encrypted and
//! authenticated: nobody without the passphrase can add one, and `set.json`
//! (display only) never holds one. Pairs name machines by their id
//! ([`super::ThisMachine::id`]); each machine records its name and OS in the
//! index too (`machines`), which says how its folders are written. The index
//! merges them as it merges everything: a push or a pair edit brings only its
//! own additions and removals to the index as it is at that moment, a pair
//! joins by its id, and a removal is a tombstone that no later copy of the
//! pair undoes. This machine keeps what the sets last said in
//! `<app data dir>/backup-pairs.json`, so the session list maps a session's
//! folder without opening a set.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::SetIndex;
use crate::claude::paths::{self, LocalPair, PathForm};

/// At most this many pairs (removed ones not counted) per set.
pub const MAX_PAIRS: usize = 200;

/// One folder pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPair {
    /// `fp_` and 16 hex characters, made once: every copy of the pair keeps it.
    pub id: String,
    /// Machine id → its folder, as that machine writes paths. Two machines.
    pub folders: BTreeMap<String, String>,
    pub created_at: i64,
    /// The machine that made it.
    #[serde(default)]
    pub by: String,
    /// Removed: kept as a tombstone, so no copy of the pair brings it back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<Removed>,
    /// Fields a newer version wrote; kept as they were.
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

/// When, and by which machine, a pair was removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub at: i64,
    #[serde(default)]
    pub by: String,
}

/// A machine as a set's index knows it (each machine writes its own line on
/// every index write it makes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    /// Its host name (display only).
    pub name: String,
    /// `std::env::consts::OS`: `macos`, `windows`, `linux`, …
    pub os: String,
    /// When it last wrote the index.
    pub at: i64,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl MachineInfo {
    /// This machine now.
    pub fn here(name: &str, now: i64) -> Self {
        Self {
            name: name.to_string(),
            os: std::env::consts::OS.to_string(),
            at: now,
            extra: BTreeMap::new(),
        }
    }
}

/// `fp_` and 16 lowercase hex characters.
pub fn is_pair_id(s: &str) -> bool {
    s.strip_prefix("fp_")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

pub fn new_pair_id() -> String {
    let hex: String = crate::server::auth::random_bytes(8)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("fp_{hex}")
}

impl FolderPair {
    pub fn live(&self) -> bool {
        self.removed.is_none()
    }

    pub fn folder_of(&self, machine: &str) -> Option<&str> {
        self.folders.get(machine).map(String::as_str)
    }

    /// What an index may hold: a pair id, two or more machine ids, folders
    /// absolute on some machine. Anything else is dropped when read.
    pub fn valid(&self) -> bool {
        is_pair_id(&self.id)
            && self.folders.len() >= 2
            && self
                .folders
                .iter()
                .all(|(m, f)| super::is_machine_id(m) && paths::is_absolute_anywhere(f))
    }
}

/// Merge `incoming` — a run's additions and removals, or another copy's
/// whole list — into `pairs`: a pair joins by id, and a removal sticks (a
/// pair removed in either stays removed, its earliest removal kept).
pub fn merge(pairs: &mut Vec<FolderPair>, incoming: &[FolderPair]) {
    for p in incoming {
        match pairs.iter_mut().find(|q| q.id == p.id) {
            Some(q) => {
                if let Some(r) = &p.removed {
                    if q.removed.as_ref().is_none_or(|old| r.at < old.at) {
                        q.removed = Some(r.clone());
                    }
                }
            }
            None => pairs.push(p.clone()),
        }
    }
    pairs.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
}

/// `folder`, as machine `from` writes it, mapped to machine `to`'s folder by
/// the live pair between the two whose `from` folder is the longest one it
/// is under (`None`: no pair maps it, or `from` is `to`).
pub fn map_between(pairs: &[FolderPair], from: &str, to: &str, folder: &str) -> Option<String> {
    if from == to {
        return None;
    }
    let local: Vec<LocalPair> = pairs
        .iter()
        .filter(|p| p.live())
        .filter_map(|p| {
            Some(LocalPair {
                here: p.folder_of(to)?.to_string(),
                there: p.folder_of(from)?.to_string(),
            })
        })
        .collect();
    paths::map_here(folder, &local)
}

/// The pairs `here` (this machine's id) is a side of, as the session list
/// uses them: this machine's folder and each other machine's.
pub fn local_pairs(pairs: &[FolderPair], here: &str) -> Vec<LocalPair> {
    let mut out = Vec::new();
    for p in pairs.iter().filter(|p| p.live()) {
        let Some(mine) = p.folder_of(here) else {
            continue;
        };
        for (m, f) in &p.folders {
            if m != here {
                out.push(LocalPair {
                    here: mine.to_string(),
                    there: f.clone(),
                });
            }
        }
    }
    out
}

/// A machine a set knows, for display and for the form of its folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownMachine {
    pub name: String,
    /// Its OS, once it wrote it (an older build did not).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
}

/// Every machine a set's index names: those that wrote their line
/// (`machines`), and those its sessions, bundles and `seen` name.
pub fn known_machines(index: &SetIndex) -> BTreeMap<String, KnownMachine> {
    let mut out: BTreeMap<String, KnownMachine> = BTreeMap::new();
    let mut note = |id: &str, name: &str| {
        if super::is_machine_id(id) {
            out.entry(id.to_string()).or_insert_with(|| KnownMachine {
                name: name.to_string(),
                os: None,
            });
        }
    };
    for e in index.sessions.values() {
        note(&e.machine, &e.host);
    }
    for c in &index.config {
        note(&c.machine, &c.host);
    }
    for id in index.seen.keys() {
        note(id, "");
    }
    for (id, m) in &index.machines {
        if super::is_machine_id(id) {
            out.insert(
                id.clone(),
                KnownMachine {
                    name: m.name.clone(),
                    os: Some(m.os.clone()).filter(|o| !o.is_empty()),
                },
            );
        }
    }
    out
}

/// How a machine writes paths, by what the set knows of it, else by the
/// folder itself.
pub fn form_of(machines: &BTreeMap<String, KnownMachine>, id: &str, folder: &str) -> PathForm {
    machines
        .get(id)
        .and_then(|m| m.os.as_deref())
        .map(PathForm::of_os)
        .or_else(|| PathForm::of_path(folder))
        .unwrap_or(PathForm::Posix)
}

/// What a set's index says of pairs and machines, as the cache keeps it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SetPairs {
    pub pairs: Vec<FolderPair>,
    pub machines: BTreeMap<String, KnownMachine>,
    /// When it was read.
    pub at: i64,
}

impl SetPairs {
    pub fn of(index: &SetIndex, now: i64) -> Self {
        Self {
            pairs: index.folder_pairs.clone(),
            machines: known_machines(index),
            at: now,
        }
    }
}

// ─────────────────────────────── the cache ───────────────────────────────

/// `<app data dir>/backup-pairs.json` (0600): what each set (by its folder)
/// last said, as this app read or wrote its index.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cache {
    pub sets: BTreeMap<String, SetPairs>,
}

pub const CACHE_FILE: &str = "backup-pairs.json";

fn set_key(dir: &Path) -> String {
    dir.to_string_lossy().into_owned()
}

pub fn load(data_dir: &Path) -> Cache {
    crate::persist::read_json(&data_dir.join(CACHE_FILE)).unwrap_or_default()
}

/// Keep what `set_dir`'s index says (when it is not older than what the
/// cache has), and save the cache. Returns it.
pub fn note(data_dir: &Path, set_dir: &Path, seen: SetPairs) -> Cache {
    let mut cache = load(data_dir);
    let key = set_key(set_dir);
    if cache.sets.get(&key).is_none_or(|old| old.at <= seen.at) {
        cache.sets.insert(key, seen);
        if let Err(e) = crate::persist::write_json_private(&data_dir.join(CACHE_FILE), &cache) {
            tracing::warn!(error = %e, "could not keep the folder pairs");
        }
    }
    cache
}

impl Cache {
    /// The live pairs of these sets (each once, by id).
    pub fn pairs_of(&self, set_dirs: &[PathBuf]) -> Vec<FolderPair> {
        let mut out = self.all_pairs_of(set_dirs);
        out.retain(FolderPair::live);
        out
    }

    /// Every pair of these sets, removed ones too (each once, by id, a
    /// removal sticking): what roots are imported from
    /// (`machines::roots`).
    pub fn all_pairs_of(&self, set_dirs: &[PathBuf]) -> Vec<FolderPair> {
        let mut out: Vec<FolderPair> = Vec::new();
        for d in set_dirs {
            if let Some(s) = self.sets.get(&set_key(d)) {
                merge(&mut out, &s.pairs);
            }
        }
        out
    }

    /// The machines these sets know.
    pub fn machines_of(&self, set_dirs: &[PathBuf]) -> BTreeMap<String, KnownMachine> {
        let mut out: BTreeMap<String, KnownMachine> = BTreeMap::new();
        for d in set_dirs {
            for (id, m) in self
                .sets
                .get(&set_key(d))
                .map(|s| &s.machines)
                .into_iter()
                .flatten()
            {
                let slot = out.entry(id.clone()).or_insert_with(|| m.clone());
                if slot.os.is_none() {
                    slot.os = m.os.clone();
                }
                if slot.name.is_empty() {
                    slot.name = m.name.clone();
                }
            }
        }
        out
    }

    /// When a set of these was last read.
    pub fn read_at(&self, set_dirs: &[PathBuf]) -> Option<i64> {
        set_dirs
            .iter()
            .filter_map(|d| self.sets.get(&set_key(d)).map(|s| s.at))
            .max()
    }
}

/// Make these sets' pairs this machine's (`here`: its id) for the session
/// list ([`paths::set_local_pairs`]; `data_dir`: whose they are).
pub fn apply(data_dir: &Path, cache: &Cache, here: Option<&str>, set_dirs: &[PathBuf]) {
    let pairs = here
        .map(|h| local_pairs(&cache.pairs_of(set_dirs), h))
        .unwrap_or_default();
    paths::set_local_pairs(&data_dir.to_string_lossy(), pairs);
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccc";

    fn pair(id: &str, a: &str, b: &str, at: i64) -> FolderPair {
        FolderPair {
            id: id.to_string(),
            folders: [
                (A.to_string(), a.to_string()),
                (B.to_string(), b.to_string()),
            ]
            .into_iter()
            .collect(),
            created_at: at,
            by: A.to_string(),
            removed: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn pair_ids_are_made_and_checked() {
        let id = new_pair_id();
        assert!(is_pair_id(&id), "{id}");
        assert_ne!(new_pair_id(), id);
        for bad in [
            "fp_",
            "fp_0123456789abcdeF",
            "xx_0123456789abcdef",
            "fp_0123456789abcde",
        ] {
            assert!(!is_pair_id(bad), "{bad}");
        }
        assert!(pair("fp_0123456789abcdef", "/Users/m/dev", r"D:\dev", 1).valid());
        assert!(!pair("fp_0123456789abcdef", "dev", r"D:\dev", 1).valid());
    }

    /// Two machines edit at once, each writing only its own changes to the
    /// index as it is then: neither loses the other's pair, and a removal
    /// sticks even when a copy of the removed pair comes in later.
    #[test]
    fn merging_keeps_both_machines_pairs_and_removals_stick() {
        let p0 = pair("fp_0000000000000000", "/Users/m/old", r"D:\old", 1);
        let mut index = vec![p0.clone()];
        // A adds one; B removes p0 and adds another, from the same start.
        let p1 = pair("fp_1111111111111111", "/Users/m/dev", r"D:\dev", 2);
        let mut gone = p0.clone();
        gone.removed = Some(Removed {
            at: 3,
            by: B.into(),
        });
        let p2 = pair("fp_2222222222222222", "/Users/m/work", r"E:\work", 4);
        merge(&mut index, std::slice::from_ref(&p1));
        merge(&mut index, &[gone, p2.clone()]);
        let live: Vec<&str> = index
            .iter()
            .filter(|p| p.live())
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(live, ["fp_1111111111111111", "fp_2222222222222222"]);
        // A copy of p0 from before the removal (a stale list) changes nothing.
        merge(&mut index, &[p0]);
        assert!(!index[0].live(), "removed for good: {:?}", index[0]);
        assert_eq!(index.len(), 3);
    }

    #[test]
    fn pairs_map_between_their_two_machines_only() {
        let pairs = [
            pair("fp_1111111111111111", "/Users/m/dev", r"D:\dev", 1),
            pair("fp_2222222222222222", "/Users/m/dev/work", r"E:\work", 2),
        ];
        assert_eq!(
            map_between(&pairs, B, A, r"d:\dev\app").as_deref(),
            Some("/Users/m/dev/app")
        );
        assert_eq!(
            map_between(&pairs, A, B, "/Users/m/dev/work/api").as_deref(),
            Some(r"E:\work\api")
        );
        assert_eq!(map_between(&pairs, A, C, "/Users/m/dev/app"), None);
        assert_eq!(map_between(&pairs, A, A, "/Users/m/dev/app"), None);
        let mut removed = pairs[0].clone();
        removed.removed = Some(Removed {
            at: 9,
            by: A.into(),
        });
        assert_eq!(map_between(&[removed], B, A, r"D:\dev\app"), None);
        let local = local_pairs(&pairs, B);
        assert_eq!(
            local[0],
            LocalPair {
                here: r"D:\dev".into(),
                there: "/Users/m/dev".into()
            }
        );
    }

    #[test]
    fn the_cache_keeps_the_newest_word_of_each_set() {
        let tmp = tempfile::tempdir().unwrap();
        let set = tmp.path().join("NAS/Default");
        let newer = SetPairs {
            pairs: vec![pair("fp_1111111111111111", "/Users/m/dev", r"D:\dev", 1)],
            machines: BTreeMap::new(),
            at: 20,
        };
        note(tmp.path(), &set, newer.clone());
        let older = SetPairs {
            at: 10,
            ..SetPairs::default()
        };
        let cache = note(tmp.path(), &set, older);
        assert_eq!(cache.sets[&set_key(&set)], newer);
        assert_eq!(
            load(tmp.path()).pairs_of(std::slice::from_ref(&set)).len(),
            1
        );
        assert!(load(tmp.path())
            .pairs_of(&[tmp.path().join("other")])
            .is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(tmp.path().join(CACHE_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
