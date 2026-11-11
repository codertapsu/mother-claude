//! A session's files for a move to another computer (docs/PLAN-MACHINES.md
//! §5.8, decision 2): packed in the backup's layout — the very file list of
//! a set's archive (`session_now`), with `.mc/meta.json` — as a plain
//! tar.gz (the peer link is encrypted end to end), and landed with the
//! pull's machinery: its staging, relocation into the folder the session
//! works in there, placement, fast-forward replace, conflict copy and undo
//! folder. The transcripts are never rewritten.
//!
//! A move's landing differs from a pull in one rule only: the incoming copy
//! is the session's home's (the lease holder's), so when this machine's
//! copy is not a prefix of it — it went on here meanwhile, after a take
//! over — the incoming copy becomes the session and this machine's own
//! turns stay beside it as `<id>.conflict-<host>-<time>.jsonl`, never
//! merged. Both are in the undo folder's manifest.

use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use super::{
    conflict_name, file_is_prefix_of_file, io, main_transcript, now_ms, pack_into, place_staged,
    relocate_staged, replace_local, session_folder, session_now, staged, unpack_from, BackupError,
    HashingWriter, Staged, SET_VERSION,
};
use crate::claude::home::ClaudeHome;
use crate::claude::paths::LocalPair;
use crate::claude::transfer::{self, Created, Manifest, Mode, Progress, What};

/// The data job's kind a move's landing records (`GET /api/data/jobs`).
pub const KIND: &str = "machine-move";
/// The package's file name.
pub const FILE: &str = "session.tar.gz";

/// What a session is here, read without packing it (the move sheet's "What
/// goes").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    /// The cwd its transcript records first.
    pub cwd: Option<String>,
    /// The project folder it lives in as this machine sees it (folder pairs
    /// included): where it works here.
    pub folder: Option<String>,
    pub transcript_bytes: u64,
    /// Every file's bytes.
    pub bytes: u64,
    pub files: usize,
    /// Its subagents' transcripts.
    pub subagents: usize,
    pub newest_mtime: i64,
}

/// [`Facts`] of session `id` in `home` (`local`: this machine's folder
/// pairs). Blocking.
pub fn facts(home: &ClaudeHome, id: &str, local: &[LocalPair]) -> Result<Facts, BackupError> {
    let now = session_now(home, id)?;
    let subagents = now
        .files
        .iter()
        .filter(|f| {
            f.extension().is_some_and(|e| e == "jsonl")
                && f.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|d| d == "subagents")
        })
        .count();
    Ok(Facts {
        folder: session_folder(home, id, now.cwd.as_deref(), local),
        cwd: now.cwd,
        transcript_bytes: now.transcript_bytes,
        bytes: now.bytes,
        files: now.files.len(),
        subagents,
        newest_mtime: now.newest_mtime,
    })
}

/// A packed session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    /// The session's own transcript as packed.
    pub transcript_bytes: u64,
    pub transcript_sha256: String,
    pub cwd: Option<String>,
    pub folder: Option<String>,
    pub files: usize,
}

/// Pack session `id` of `home` into `out` (a temp file beside it, synced,
/// renamed over an earlier attempt's). The session must not change while it
/// is read: a turn that wrote meanwhile fails it (`session_changed`), so a
/// half-written copy never travels. Blocking.
pub fn pack(
    home: &ClaudeHome,
    id: &str,
    out: &Path,
    local: &[LocalPair],
) -> Result<Packed, BackupError> {
    use sha2::Digest;
    let now = session_now(home, id)?;
    let base = home.base();
    let files: Vec<(PathBuf, PathBuf)> = now
        .files
        .iter()
        .map(|f| (f.clone(), f.strip_prefix(base).unwrap_or(f).to_path_buf()))
        .collect();
    let folder = session_folder(home, id, now.cwd.as_deref(), local);
    // Under either spelling (Windows keys `D:/dev/app`), as an archive's.
    let entry_of = |c: &str| transfer::project_entry(&home.claude_json(), c);
    let meta = serde_json::json!({
        "version": SET_VERSION,
        "id": id,
        "cwd": now.cwd,
        "folder": folder,
        "projectEntry": folder.as_deref().and_then(entry_of)
            .or_else(|| now.cwd.as_deref().and_then(entry_of)),
    });
    let meta = serde_json::to_vec_pretty(&meta).unwrap_or_default();
    let dir = out
        .parent()
        .ok_or_else(|| BackupError::Invalid(format!("{} has no folder", out.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| io("creating", dir, e))?;
    let tmp = dir.join(format!(".session.mc-{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = (|| -> Result<(String, u64), BackupError> {
        let file = std::fs::File::create(&tmp).map_err(|e| io("writing", &tmp, e))?;
        let hashing = HashingWriter {
            inner: BufWriter::new(file),
            hasher: sha2::Sha256::new(),
            bytes: 0,
        };
        let hashing = pack_into(
            hashing,
            &tmp,
            base,
            &files,
            &now.dirs,
            &[(".mc/meta.json", meta)],
            None,
        )?;
        let sha: String = hashing
            .hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let size = hashing.bytes;
        let file = hashing
            .inner
            .into_inner()
            .map_err(|e| io("writing", &tmp, e.error()))?;
        crate::persist::sync_file(&file).map_err(|e| io("syncing", &tmp, e))?;
        drop(file);
        if std::fs::symlink_metadata(out).is_ok() {
            std::fs::remove_file(out).map_err(|e| io("replacing", out, e))?;
        }
        std::fs::rename(&tmp, out).map_err(|e| io("placing", out, e))?;
        Ok((sha, size))
    })();
    let (sha256, size) = match written {
        Ok(w) => w,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    // Read while a process may hold it: one that wrote meanwhile leaves this
    // copy behind rather than a half-written one.
    let after = session_now(home, id);
    if !after.as_ref().is_ok_and(|a| a.unchanged_since(&now)) {
        let _ = std::fs::remove_file(out);
        return Err(BackupError::Changed(
            "it changed while it was being packed (Claude started working in it): try again \
             once it is idle"
                .into(),
        ));
    }
    let main = transfer::transcripts_of(home, id)
        .into_iter()
        .next()
        .ok_or_else(|| BackupError::Invalid(format!("{id} has no transcript here")))?;
    let (transcript_bytes, transcript_sha256) =
        transfer::sha256_file(&main).map_err(|e| io("reading", &main, e))?;
    Ok(Packed {
        path: out.to_path_buf(),
        sha256,
        size,
        transcript_bytes,
        transcript_sha256,
        cwd: now.cwd,
        folder,
        files: files.len(),
    })
}

/// What a landing did with the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "action")]
pub enum Landed {
    /// This machine did not have it.
    New,
    /// This machine's copy is where the incoming one came from (a byte
    /// prefix of it): replaced, the old one kept for undo.
    FastForward,
    /// This machine has it exactly as it came: nothing was written.
    Same,
    /// This machine's copy went on on its own: the incoming one is the
    /// session now, and this machine's turns stay beside it in `copy`.
    Conflict { copy: PathBuf },
}

/// What a landing needs.
pub struct LandInputs<'a> {
    /// The received package (its hash checked by the caller).
    pub archive: &'a Path,
    pub id: &'a str,
    /// The config directory of the account it lands under.
    pub home: &'a ClaudeHome,
    /// The `projects/<name>` the session goes under here: its folder's here
    /// (`None`: as the package has it).
    pub project_dir: Option<&'a str>,
    pub undo_root: &'a Path,
    pub job_id: &'a str,
    /// Carry the folder's trust (its `.claude.json` entry) under this key.
    pub trust_key: Option<&'a str>,
    /// This machine's name, for a conflict copy's.
    pub host: &'a str,
    /// The account the job names.
    pub account: Option<&'a str>,
}

/// What a landing did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LandReport {
    #[serde(flatten)]
    pub landed: Landed,
    /// The folder's trust was carried here.
    pub trusted: bool,
    pub warnings: Vec<String>,
}

/// Land a received session package in `inp.home` as a data job (its undo
/// folder `<undo_root>/<job>`): staged under the directory first (ours
/// alone), moved under the folder it works in here, then placed — new,
/// a fast-forward replace, nothing when it is the same, or the incoming
/// copy with this machine's own kept beside it ([`Landed::Conflict`]).
/// Shared files: only the trust entry, only when asked, never while a
/// process uses the directory. On failure nothing it made stays. Blocking.
pub fn land(inp: &LandInputs<'_>) -> Result<(LandReport, Manifest), BackupError> {
    let id = inp.id;
    if !crate::claude::ids::is_session_id(id) {
        return Err(BackupError::Invalid(format!("{id:?} is not a session id")));
    }
    let undo_dir = inp.undo_root.join(inp.job_id);
    std::fs::create_dir_all(&undo_dir).map_err(|e| io("creating", &undo_dir, e))?;
    transfer::private_dir(inp.undo_root);
    transfer::private_dir(&undo_dir);
    let mut m = Manifest {
        created_at: now_ms(),
        to_config: Some(inp.home.claude_json()),
        kind: Some(KIND.to_string()),
        to_account: inp.account.map(str::to_string),
        ..Manifest::new(
            inp.job_id,
            Mode::Copy,
            What::Session { id: id.to_string() },
            inp.archive
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            inp.home.base().to_path_buf(),
        )
    };
    let stage_root = inp.home.base().join(format!(".mc-move-{}", inp.job_id));
    // Ours alone, like a restore's stage: one that exists already (or a
    // link planted there) is never written into.
    std::fs::create_dir(&stage_root).map_err(|e| io("creating", &stage_root, e))?;
    let result = land_staged(inp, &stage_root, &undo_dir, &mut m);
    let _ = std::fs::remove_dir_all(&stage_root);
    let report = result?;
    crate::persist::write_json_private(&undo_dir.join("manifest.json"), &m)
        .map_err(|e| io("writing", &undo_dir, e))?;
    Ok((report, m))
}

fn land_staged(
    inp: &LandInputs<'_>,
    stage_root: &Path,
    undo_dir: &Path,
    m: &mut Manifest,
) -> Result<LandReport, BackupError> {
    let id = inp.id;
    let home = inp.home;
    let progress = Progress::default();
    let dir = stage_root.join(id);
    std::fs::create_dir(&dir).map_err(|e| io("creating", &dir, e))?;
    let file = std::fs::File::open(inp.archive).map_err(|e| io("reading", inp.archive, e))?;
    let meta = unpack_from(BufReader::new(file), &dir, id)?;
    let (files, dirs) = staged(&dir);
    let mut st = Staged {
        files,
        dirs,
        dir,
        meta,
    };
    if main_transcript(&st, id).is_none() {
        return Err(BackupError::Invalid(format!(
            "the package of {id} has no transcript"
        )));
    }
    if let Some(to) = inp.project_dir.filter(|d| !d.is_empty()) {
        relocate_staged(&mut st, id, to)?;
    }
    let theirs = st
        .dir
        .join(main_transcript(&st, id).cloned().unwrap_or_default());
    let mine = transfer::transcripts_of(home, id).into_iter().next();
    let landed = match &mine {
        None => {
            place_staged(&st, home, m, &progress)?;
            Landed::New
        }
        Some(mine) => {
            let same_place = main_transcript(&st, id)
                .map(|rel| home.base().join(rel))
                .is_some_and(|p| crate::claude::paths::same_here(&p, mine));
            if same_place && same_bytes(mine, &theirs)? {
                Landed::Same
            } else if file_is_prefix_of_file(mine, &theirs)? {
                replace_local(home, id, &st, undo_dir, m, &progress)?;
                Landed::FastForward
            } else {
                let old_rel = mine.strip_prefix(home.base()).unwrap_or(mine).to_path_buf();
                let at = std::fs::metadata(mine)
                    .and_then(|x| x.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or_else(now_ms);
                let new_main = main_transcript(&st, id)
                    .map(|rel| home.base().join(rel))
                    .unwrap_or_default();
                replace_local(home, id, &st, undo_dir, m, &progress)?;
                // This machine's own turns, beside the session: a copy of the
                // one the undo folder keeps (the undo puts that one back).
                let saved = undo_dir.join("moved").join(&old_rel);
                let copy = new_main.with_file_name(conflict_name(id, inp.host, at));
                let created = copy_beside(&saved, &copy)?;
                m.created.push(created);
                Landed::Conflict { copy }
            }
        }
    };
    m.sessions.push(id.to_string());
    let mut report = LandReport {
        landed,
        trusted: false,
        warnings: Vec::new(),
    };
    // Trust only with the opt-in, never over an entry (under either
    // spelling), never while the directory is in use.
    if let Some(key) = inp.trust_key {
        let project = st.meta.get("projectEntry").filter(|e| e.is_object());
        match (project, transfer::in_use(home)) {
            (None, _) => report
                .warnings
                .push("The folder's trust did not come with it: it was not trusted there.".into()),
            (Some(_), Some(busy)) => report
                .warnings
                .push(format!("The folder's trust was not carried: {busy}")),
            (Some(project), None) => {
                if transfer::project_entry(&home.claude_json(), key).is_none() {
                    let project: Value = project.clone();
                    let key = key.to_string();
                    transfer::edit_projects(&home.claude_json(), |p| {
                        p.entry(key.clone()).or_insert_with(|| project.clone());
                    })?;
                    m.added_entries.push((key, project));
                }
                report.trusted = true;
            }
        }
    }
    Ok(report)
}

/// Whether two files hold the same bytes.
fn same_bytes(a: &Path, b: &Path) -> Result<bool, BackupError> {
    let (la, sa) = transfer::sha256_file(a).map_err(|e| io("reading", a, e))?;
    let (lb, sb) = transfer::sha256_file(b).map_err(|e| io("reading", b, e))?;
    Ok(la == lb && sa == sb)
}

/// Copy `from` to `to` (a temp name beside it, synced, renamed), never over
/// another file.
fn copy_beside(from: &Path, to: &Path) -> Result<Created, BackupError> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(BackupError::Transfer(
            transfer::TransferError::AlreadyExists(to.display().to_string()),
        ));
    }
    let dir = to
        .parent()
        .ok_or_else(|| BackupError::Invalid(format!("{} has no folder", to.display())))?;
    let tmp = dir.join(format!(
        ".conflict.mc-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> Result<(), BackupError> {
        std::fs::copy(from, &tmp).map_err(|e| io("copying", from, e))?;
        let f = std::fs::File::options()
            .write(true)
            .open(&tmp)
            .map_err(|e| io("opening", &tmp, e))?;
        crate::persist::sync_file(&f).map_err(|e| io("syncing", &tmp, e))?;
        drop(f);
        std::fs::rename(&tmp, to).map_err(|e| io("placing", to, e))
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let (size, sha256) = transfer::sha256_file(to).map_err(|e| io("verifying", to, e))?;
    Ok(Created {
        path: to.to_path_buf(),
        size,
        sha256,
        link: None,
    })
}

/// The main transcript's bytes and SHA-256 of session `id` in `home` (what a
/// landing compares an undo against).
pub fn transcript_state(home: &ClaudeHome, id: &str) -> Option<(PathBuf, u64, String)> {
    let main = transfer::transcripts_of(home, id).into_iter().next()?;
    let (size, sha) = transfer::sha256_file(&main).ok()?;
    Some((main, size, sha))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0f1e2d3c-4b5a-4968-8776-655443322110";

    fn session(home: &ClaudeHome, cwd: &str, lines: &[&str]) -> PathBuf {
        let dir = home
            .projects_dir()
            .join(crate::claude::paths::project_dir_name(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join(format!("{ID}.jsonl"));
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&t, text).unwrap();
        t
    }

    fn line(cwd: &str, n: u32) -> String {
        serde_json::json!({"type": "user", "uuid": format!("u-{n}"), "cwd": cwd,
            "message": {"role": "user", "content": format!("turn {n}")}})
        .to_string()
    }

    /// Packed on one machine, landed on another: new, then the same, then a
    /// fast-forward, then a copy that went on here becomes a conflict copy
    /// beside the incoming one — and each landing's undo puts it back.
    #[test]
    fn a_package_lands_new_same_fast_forward_and_as_a_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let src = ClaudeHome::for_account_dir(tmp.path().join("src"));
        let dst = ClaudeHome::for_account_dir(tmp.path().join("dst"));
        std::fs::create_dir_all(dst.base()).unwrap();
        let undo = tmp.path().join("undo");
        let cwd = "/Users/m/app";
        let (l1, l2, l3) = (line(cwd, 1), line(cwd, 2), line(cwd, 3));
        session(&src, cwd, &[&l1]);
        let out = tmp.path().join("pkg").join(FILE);
        let land = |job: &str| {
            land(&LandInputs {
                archive: &out,
                id: ID,
                home: &dst,
                project_dir: Some("D--dev-app"),
                undo_root: &undo,
                job_id: job,
                trust_key: None,
                host: "DESKTOP-PC",
                account: None,
            })
        };

        let p = pack(&src, ID, &out, &[]).unwrap();
        assert_eq!(p.cwd.as_deref(), Some(cwd));
        assert_eq!(p.transcript_bytes, l1.len() as u64 + 1);
        let (r, m) = land("20261111T000001-00000001").unwrap();
        assert_eq!(r.landed, Landed::New);
        let placed = dst
            .projects_dir()
            .join("D--dev-app")
            .join(format!("{ID}.jsonl"));
        assert_eq!(std::fs::read_to_string(&placed).unwrap(), format!("{l1}\n"));
        assert_eq!(m.created.len(), 1);

        let (r, _) = land("20261111T000002-00000002").unwrap();
        assert_eq!(r.landed, Landed::Same, "nothing to write");

        session(&src, cwd, &[&l1, &l2]);
        pack(&src, ID, &out, &[]).unwrap();
        let (r, _) = land("20261111T000003-00000003").unwrap();
        assert_eq!(r.landed, Landed::FastForward);
        assert_eq!(
            std::fs::read_to_string(&placed).unwrap(),
            format!("{l1}\n{l2}\n")
        );

        // It went on here (turn 3), and the source went on its own way
        // (turn 2b): the incoming copy wins, this one stays beside it.
        std::fs::write(&placed, format!("{l1}\n{l2}\n{l3}\n")).unwrap();
        let l2b = line(cwd, 22);
        session(&src, cwd, &[&l1, &l2b]);
        pack(&src, ID, &out, &[]).unwrap();
        let job = "20261111T000004-00000004";
        let (r, _) = land(job).unwrap();
        let Landed::Conflict { copy } = &r.landed else {
            panic!("a conflict: {r:?}");
        };
        assert_eq!(
            std::fs::read_to_string(&placed).unwrap(),
            format!("{l1}\n{l2b}\n")
        );
        assert_eq!(
            std::fs::read_to_string(copy).unwrap(),
            format!("{l1}\n{l2}\n{l3}\n")
        );
        assert!(copy
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(&format!("{ID}.conflict-DESKTOP-PC-")));

        // Undo: this machine's copy is back, the conflict copy gone.
        transfer::undo(&undo, job).unwrap();
        assert_eq!(
            std::fs::read_to_string(&placed).unwrap(),
            format!("{l1}\n{l2}\n{l3}\n")
        );
        assert!(!copy.exists());
    }

    /// A package carries only the session's own files: anything else in it
    /// is refused before a byte is placed.
    #[test]
    fn a_package_with_a_foreign_entry_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dst = ClaudeHome::for_account_dir(tmp.path().join("dst"));
        std::fs::create_dir_all(dst.base()).unwrap();
        let out = tmp.path().join(FILE);
        let file = std::fs::File::create(&out).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        let data = b"{}\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, "settings.json", &data[..])
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let err = land(&LandInputs {
            archive: &out,
            id: ID,
            home: &dst,
            project_dir: None,
            undo_root: &tmp.path().join("undo"),
            job_id: "20261111T000001-00000001",
            trust_key: None,
            host: "x",
            account: None,
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("not one of its own files"),
            "{err}"
        );
        assert!(!dst.base().join("settings.json").exists());
        assert!(
            std::fs::read_dir(dst.base()).unwrap().next().is_none(),
            "no stage left behind"
        );
    }
}
