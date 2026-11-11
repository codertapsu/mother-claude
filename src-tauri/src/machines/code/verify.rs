//! Proof that a landing holds what the source had: HEAD, the staged tree,
//! the working tree, `git status`, and every file byte for byte, each
//! difference explained (line endings and filters are expected; modes,
//! symlinks held as files, case, submodules and intent-to-add entries are
//! known limits; anything else fails).

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::time::Instant;

use super::apply::{load_meta, load_record};
use super::git::{GitOp, GitRunner};
use super::repo::{
    at, config_of, index_tree, raw_of, repo_info, status_diff, status_z, tree_diff, worktree_tree,
    DirCache,
};
use super::types::{
    io_err, ApplyOutcome, CodeError, LandingKind, ManifestEntry, Outcome, Raw, RepoConfig,
    VerifyCheck, VerifyReport,
};

/// Raw differences git's settings explain.
const EXPECTED_RAW: [&str; 4] = ["eol", "filter", "filemode", "symlink-as-file"];
/// Differences a target cannot avoid: reported as limits.
const LIMIT_KINDS: [&str; 5] = ["filemode", "symlink-as-file", "submodule", "case", "ita"];

struct RawDiff {
    path: String,
    kind: &'static str,
    want: Option<String>,
    got: Option<String>,
}

fn short(oid: &str) -> &str {
    oid.get(..10).unwrap_or(oid)
}

fn line_ending(crlf: bool) -> String {
    if crlf { "CRLF" } else { "LF" }.to_string()
}

fn raw_compare(
    op: &GitOp,
    top: &Path,
    manifest: &[ManifestEntry],
    cfg: &RepoConfig,
) -> Result<Vec<RawDiff>, CodeError> {
    let mut cache = DirCache::default();
    let mut diffs = Vec::new();
    for e in manifest {
        if matches!(e.raw, Raw::Missing) {
            continue;
        }
        let got = raw_of(op, top, &e.path, &mut cache)?;
        let mut push = |kind, want: Option<String>, got: Option<String>| {
            diffs.push(RawDiff {
                path: e.path.clone(),
                kind,
                want,
                got,
            })
        };
        if e.mode == "160000" {
            let head = match &got {
                Raw::Dir { head: Some(h), .. } => h.clone(),
                other => other.kind().to_string(),
            };
            if head != e.oid {
                push("submodule", Some(e.oid.clone()), Some(head));
            }
            continue;
        }
        if matches!(got, Raw::Missing) {
            push("missing", None, None);
            continue;
        }
        if e.raw.case_name() != got.case_name() {
            let name = |r: &Raw| r.case_name().unwrap_or("(as in git)").to_string();
            push("case", Some(name(&e.raw)), Some(name(&got)));
        }
        match (&e.raw, &got) {
            (
                Raw::File {
                    sha256: ws,
                    lf_sha256: wl,
                    crlf: wc,
                    exec: we,
                    ..
                },
                Raw::File {
                    sha256: gs,
                    lf_sha256: gl,
                    crlf: gc,
                    exec: ge,
                    ..
                },
            ) => {
                if gs == ws {
                    if let (Some(w), Some(g)) = (we, ge) {
                        if w != g {
                            let kind = if cfg.file_mode {
                                "exec-bit"
                            } else {
                                "filemode"
                            };
                            push(kind, Some(w.to_string()), Some(g.to_string()));
                        }
                    }
                } else if gl == wl {
                    push("eol", Some(line_ending(*wc)), Some(line_ending(*gc)));
                } else {
                    // What git would store for these bytes (clean filters, LFS).
                    let oid = op
                        .cmd(top)
                        .arg("hash-object")
                        .arg(format!("--path={}", e.path))
                        .arg(at(top, &e.path))
                        .text()?;
                    push(if oid == e.oid { "filter" } else { "content" }, None, None);
                }
            }
            (Raw::Symlink { target: w, .. }, Raw::Symlink { target: g, .. }) => {
                if w != g {
                    push("content", Some(w.clone()), Some(g.clone()));
                }
            }
            (Raw::Symlink { target: w, .. }, Raw::File { .. }) => {
                let p = at(top, &e.path);
                let text = std::fs::read(&p).map_err(io_err(format!("read {}", p.display())))?;
                let kind = if text == w.as_bytes() {
                    "symlink-as-file"
                } else {
                    "content"
                };
                push(kind, None, None);
            }
            (w, g) => push("type", Some(w.kind().into()), Some(g.kind().into())),
        }
    }
    Ok(diffs)
}

/// Proves the landing at `path` (the checkout or the worktree) against the
/// move in `in_dir`.
pub fn verify(git: &GitRunner, path: &Path, in_dir: &Path) -> Result<VerifyReport, CodeError> {
    let started = Instant::now();
    let op = git.begin()?;
    let meta = load_meta(in_dir)?;
    let record = load_record(in_dir)?;
    let applied = match (record.outcome, &record.applied) {
        (ApplyOutcome::Applied, Some(a)) => a.clone(),
        _ => return Err(CodeError::NotApplied),
    };
    let info = repo_info(&op, path)?;
    let top = info.top.clone();
    let cfg = config_of(&op, &top)?;
    let mut checks = Vec::new();
    let mut fails = Vec::new();
    let mut limits: BTreeSet<&'static str> = BTreeSet::new();

    let want_sym = match applied.landing {
        LandingKind::Worktree => applied.wt_branch.clone(),
        LandingKind::Checkout => meta.branch.clone(),
    };
    let at_head = |sym: &Option<String>, head: &Option<String>| {
        format!(
            "{} @ {}",
            sym.as_deref().unwrap_or("detached"),
            head.as_deref().map(short).unwrap_or("(none)")
        )
    };
    let head_ok = info.head == meta.base && info.sym == want_sym;
    let head_detail = format!(
        "want {}, got {}",
        at_head(&want_sym, &meta.base),
        at_head(&info.sym, &info.head)
    );
    if !head_ok {
        fails.push(format!("HEAD: {head_detail}"));
    }
    checks.push(VerifyCheck {
        check: "HEAD at the base, on the branch".into(),
        ok: head_ok,
        details: if head_ok {
            Vec::new()
        } else {
            vec![head_detail]
        },
    });

    let idx_now = index_tree(&op, &info)?;
    let wt_now = worktree_tree(&op, &info, true)?;
    let idx_diff = if idx_now == meta.idx_tree {
        Vec::new()
    } else {
        tree_diff(&op, &top, &meta.idx_tree, &idx_now)?
    };
    let wt_diff = if wt_now == meta.wt_tree {
        Vec::new()
    } else {
        tree_diff(&op, &top, &meta.wt_tree, &wt_now)?
    };
    for (check, diff) in [
        ("index = the staged tree", &idx_diff),
        ("working tree = the snapshot tree", &wt_diff),
    ] {
        checks.push(VerifyCheck {
            check: check.into(),
            ok: diff.is_empty(),
            details: diff
                .iter()
                .map(|c| format!("{} {}", c.kind, c.path))
                .collect(),
        });
    }
    let status = status_z(&op, &top)?;
    let st_diff = if status == meta.status {
        Vec::new()
    } else {
        status_diff(&meta.status, &status)
    };
    checks.push(VerifyCheck {
        check: "git status as on the source".into(),
        ok: st_diff.is_empty(),
        details: st_diff
            .iter()
            .map(|d| format!("{}: want {:?}, got {:?}", d.path, d.want, d.got))
            .collect(),
    });
    let raw = raw_compare(&op, &top, &meta.manifest, &cfg)?;
    checks.push(VerifyCheck {
        check: "every file byte for byte (or explained)".into(),
        ok: raw.iter().all(|d| EXPECTED_RAW.contains(&d.kind)),
        details: raw
            .iter()
            .map(|d| format!("{} {}", d.kind, d.path))
            .collect(),
    });

    let mut tree_paths = HashSet::new();
    for c in idx_diff.iter().chain(wt_diff.iter()) {
        tree_paths.insert(c.path.clone());
        if LIMIT_KINDS.contains(&c.kind) {
            limits.insert(c.kind);
        } else {
            fails.push(format!(
                "tree {}: {} ({} {} → {} {})",
                c.kind,
                c.path,
                c.src_mode,
                short(&c.src_oid),
                c.dst_mode,
                short(&c.dst_oid)
            ));
        }
    }
    for d in &st_diff {
        if d.ita {
            limits.insert("ita");
        } else if !tree_paths.contains(&d.path) {
            fails.push(format!(
                "status: {} want {:?} got {:?}",
                d.path, d.want, d.got
            ));
        }
    }
    for d in &raw {
        if EXPECTED_RAW.contains(&d.kind) {
            continue;
        }
        if LIMIT_KINDS.contains(&d.kind) {
            limits.insert(d.kind);
        } else {
            let detail = match (&d.want, &d.got) {
                (Some(w), Some(g)) => format!(" (want {w}, got {g})"),
                _ => String::new(),
            };
            fails.push(format!("file {}: {}{detail}", d.kind, d.path));
        }
    }
    let outcome = if !fails.is_empty() {
        Outcome::Fail
    } else if !limits.is_empty() {
        Outcome::Limit
    } else {
        Outcome::Pass
    };
    let raw_kinds: BTreeSet<&str> = raw.iter().map(|d| d.kind).collect();
    Ok(VerifyReport {
        outcome,
        limits: limits.into_iter().map(String::from).collect(),
        fails,
        raw_kinds: raw_kinds.into_iter().map(String::from).collect(),
        checks,
        config: cfg,
        ms: started.elapsed().as_millis() as u64,
    })
}
