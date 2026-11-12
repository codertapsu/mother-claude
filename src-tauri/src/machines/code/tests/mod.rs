//! Spike S2's matrix, ported: throwaway repositories in temp folders, git
//! isolated from this machine's configuration (`GitRunner::isolated`).

mod cases;
mod special;
mod sync;
mod task;
mod work;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::repo::{self, DirCache};
use super::{
    apply, check, haves, snapshot, verify, Applied, ApplyRequest, ApplyStop, GitOp, GitRunner,
    Landing, Raw, SnapshotMeta, SnapshotRequest, Transfer, VerifyReport,
};

pub(super) fn runner() -> GitRunner {
    GitRunner::new().isolated()
}

pub(super) fn s(p: &Path) -> &str {
    p.to_str().expect("an ASCII temp path")
}

/// The fixtures' own git: isolated, hooks off, like the code's.
pub(super) struct Fx {
    pub op: GitOp,
}

impl Fx {
    fn new() -> Self {
        Self {
            op: runner().begin().expect("git runs"),
        }
    }

    pub fn git(&self, cwd: &Path, args: &[&str]) -> String {
        self.op
            .git(cwd, args)
            .unwrap_or_else(|e| panic!("git {} in {}: {e}", args.join(" "), cwd.display()))
    }

    pub fn ok(&self, cwd: &Path, args: &[&str]) -> bool {
        self.op.cmd(cwd).args(args).ok().expect("git runs")
    }
}

pub(super) fn put(dir: &Path, rel: &str, content: impl AsRef<[u8]>) {
    let p = repo::at(dir, rel);
    std::fs::create_dir_all(p.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&p, content).expect("write");
}

/// Deterministic bytes that do not compress (xorshift32), a NUL first.
pub(super) fn noise(seed: u32, len: usize) -> Vec<u8> {
    let mut x = seed.max(1);
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    if let Some(b) = out.first_mut() {
        *b = 0;
    }
    out
}

pub(super) const BASE_FILES: &[(&str, &str)] = &[
    ("a.txt", "alpha\n"),
    ("b.txt", "bravo\n"),
    ("c.txt", "charlie\n"),
    ("d.txt", "delta\n"),
    ("e.txt", "echo\n"),
    ("src/main.js", "console.log(1);\n"),
    ("old-name.txt", "rename me\n"),
    ("disk-rename.txt", "moved on disk\n"),
    (".gitignore", "*.log\nbuild/\nnode_modules/\n"),
];

pub(super) struct World {
    _tmp: tempfile::TempDir,
    pub dir: PathBuf,
    pub origin: PathBuf,
    pub seed: PathBuf,
    pub src: PathBuf,
    pub dst: PathBuf,
    pub markers: PathBuf,
    pub fx: Fx,
}

/// One move from one checkout to another.
pub(super) struct Round {
    pub meta: SnapshotMeta,
    pub applied: Result<Applied, ApplyStop>,
    pub report: Option<VerifyReport>,
    pub dir: PathBuf,
}

impl Round {
    pub fn applied(&self) -> &Applied {
        match &self.applied {
            Ok(a) => a,
            Err(e) => panic!("not applied: {e}"),
        }
    }

    pub fn report(&self) -> &VerifyReport {
        self.report.as_ref().expect("verified")
    }

    pub fn assert_pass(&self) {
        let r = self.report();
        assert_eq!(
            r.outcome,
            super::Outcome::Pass,
            "fails {:?}; limits {:?}",
            r.fails,
            r.limits
        );
    }
}

impl World {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().expect("temp dir");
        let dir = dunce::canonicalize(tmp.path()).expect("canonical temp dir");
        Self {
            origin: dir.join("origin.git"),
            seed: dir.join("seed"),
            src: dir.join("src"),
            dst: dir.join("dst"),
            markers: dir.join("markers"),
            dir,
            _tmp: tmp,
            fx: Fx::new(),
        }
    }

    pub fn worktrees(&self) -> PathBuf {
        self.dir.join("appdata").join("machines").join("worktrees")
    }

    pub fn init_origin(&self) {
        self.fx
            .git(&self.dir, &["init", "--bare", "-q", s(&self.origin)]);
        self.fx
            .git(&self.dir, &["clone", "-q", s(&self.origin), s(&self.seed)]);
    }

    pub fn commit_seed(&self, msg: &str) {
        self.fx.git(&self.seed, &["add", "-A"]);
        self.fx
            .git(&self.seed, &["commit", "-q", "--allow-empty", "-m", msg]);
    }

    pub fn push_seed(&self) {
        self.fx.git(
            &self.seed,
            &["push", "-q", "origin", "HEAD:refs/heads/main"],
        );
    }

    /// origin.git with one commit per list of files on main.
    pub fn seed_with(&self, commits: &[&[(&str, &str)]]) {
        self.init_origin();
        for files in commits {
            for (rel, content) in *files {
                put(&self.seed, rel, content);
            }
            self.commit_seed("commit");
        }
        self.push_seed();
    }

    pub fn clone_to(&self, to: &Path, cfg: &[(&str, &str)]) {
        let mut args = vec!["clone".to_string(), "-q".to_string()];
        for (k, v) in cfg {
            args.push("-c".into());
            args.push(format!("{k}={v}"));
        }
        args.push(s(&self.origin).into());
        args.push(s(to).into());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.fx.git(&self.dir, &args);
    }

    pub fn clones(&self) {
        self.clone_to(&self.src, &[]);
        self.clone_to(&self.dst, &[]);
    }

    /// Hooks in both repositories that leave a mark when they run.
    pub fn hooks(&self) {
        std::fs::create_dir_all(&self.markers).expect("markers");
        let log = s(&self.markers.join("fired")).replace('\\', "/");
        for repo in [&self.src, &self.dst] {
            // Not `--git-path hooks`: that follows our core.hooksPath.
            let common = self.fx.git(repo, &["rev-parse", "--git-common-dir"]);
            let hooks = repo.join(common).join("hooks");
            std::fs::create_dir_all(&hooks).expect("hooks dir");
            for name in [
                "post-checkout",
                "post-merge",
                "post-index-change",
                "reference-transaction",
                "pre-push",
                "pre-commit",
                "post-commit",
                "post-rewrite",
                "pre-auto-gc",
            ] {
                let p = hooks.join(name);
                std::fs::write(
                    &p,
                    format!("#!/bin/sh\necho \"{name}\" >> '{log}'\nexit 0\n"),
                )
                .expect("hook");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))
                        .expect("chmod");
                }
            }
        }
    }

    pub fn fired(&self) -> Vec<String> {
        std::fs::read_to_string(self.markers.join("fired"))
            .map(|t| t.lines().map(String::from).collect())
            .unwrap_or_default()
    }

    /// Moves `session` from `from` to `to` through the move folder `name`.
    #[allow(clippy::too_many_arguments)]
    pub fn move_code(
        &self,
        from: &Path,
        to: &Path,
        session: &str,
        epoch: u64,
        transfer: Transfer,
        landing: Landing,
        name: &str,
    ) -> Round {
        let git = runner();
        let dir = self.dir.join(name);
        let mut req = SnapshotRequest::new(session, epoch, "machine-a", &dir);
        if transfer == Transfer::Bundle {
            req.haves = haves(&git, to, Some(session)).expect("haves").haves;
        }
        req.transfer = transfer;
        let meta = snapshot(&git, from, &req).expect("snapshot");
        let mut areq = ApplyRequest::new(session, landing);
        areq.machine = "machine-b".into();
        let applied = apply(&git, to, &dir, &areq);
        let report = applied
            .as_ref()
            .ok()
            .map(|a| verify(&git, &a.path, &dir).expect("verify"));
        Round {
            meta,
            applied,
            report,
            dir,
        }
    }

    /// src → dst.
    pub fn round(&self, session: &str, transfer: Transfer, landing: Landing) -> Round {
        self.move_code(&self.src, &self.dst, session, 1, transfer, landing, "move")
    }
}

pub(super) fn remote() -> Transfer {
    Transfer::Remote {
        remote: "origin".into(),
        push_branch: false,
    }
}

/// S2's standard dirty state: staged, staged then changed, unstaged,
/// deleted (staged and on disk), renamed (staged and on disk), new staged,
/// untracked, ignored.
pub(super) fn dirty_mix(fx: &Fx, s: &Path) {
    put(s, "a.txt", "alpha 2\n");
    fx.git(s, &["add", "a.txt"]);
    put(s, "b.txt", "bravo 2\n");
    fx.git(s, &["add", "b.txt"]);
    put(s, "b.txt", "bravo 3\n");
    put(s, "c.txt", "charlie 2\n");
    fx.git(s, &["rm", "-q", "d.txt"]);
    std::fs::remove_file(s.join("e.txt")).expect("rm e.txt");
    fx.git(s, &["mv", "old-name.txt", "new-name.txt"]);
    std::fs::rename(s.join("disk-rename.txt"), s.join("moved-on-disk.txt")).expect("rename");
    put(s, "f.txt", "foxtrot\n");
    fx.git(s, &["add", "f.txt"]);
    put(s, "g.txt", "golf\n");
    put(s, "notes/todo.md", "- [ ] move\n");
    put(s, "debug.log", "ignored\n");
    put(s, "build/out.bin", [0u8, 1, 2, 3]);
}

/// What a person would notice changed in a checkout (our own refs aside).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Probe {
    pub sym: Option<String>,
    pub head: Option<String>,
    pub status: String,
    pub index_sha: Option<String>,
    pub index_tree: String,
    pub refs: Vec<String>,
    pub files: BTreeMap<String, String>,
    pub worktrees: Vec<String>,
}

impl Probe {
    /// Everything, the index file's bytes included (a snapshot's promise).
    pub fn untouched(&self) -> Self {
        Self {
            index_tree: String::new(),
            ..self.clone()
        }
    }

    /// Everything, the index by its tree (an undo's promise).
    pub fn restored(&self) -> Self {
        Self {
            index_sha: None,
            ..self.clone()
        }
    }

    /// The checkout alone (a worktree landing's promise).
    pub fn checkout(&self) -> Self {
        Self {
            refs: Vec::new(),
            worktrees: Vec::new(),
            index_tree: String::new(),
            ..self.clone()
        }
    }
}

pub(super) fn probe(fx: &Fx, repo_path: &Path) -> Probe {
    let info = repo::repo_info(&fx.op, repo_path).expect("repo");
    let top = &info.top;
    let status = repo::status_z(&fx.op, top).expect("status");
    let index_sha = std::fs::read(&info.index_path)
        .ok()
        .map(|b| check::sha256_hex(&b));
    let index_tree = repo::index_tree(&fx.op, &info).expect("index tree");
    let refs = fx
        .git(top, &["for-each-ref", "--format=%(refname) %(objectname)"])
        .lines()
        .filter(|l| !l.starts_with("refs/mother-claude/"))
        .map(String::from)
        .collect();
    let listed = fx
        .op
        .cmd(top)
        .args(["ls-files", "-z", "-c", "-o", "--exclude-standard"])
        .raw()
        .expect("ls-files");
    let mut cache = DirCache::default();
    let mut files = BTreeMap::new();
    for rel in listed.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let rel = String::from_utf8_lossy(rel).into_owned();
        let raw = repo::raw_of(&fx.op, top, &rel, &mut cache).expect("raw");
        let v = match raw {
            Raw::File { sha256, exec, .. } => format!("file {sha256} {exec:?}"),
            Raw::Symlink { target, .. } => format!("link {target}"),
            Raw::Dir { head, .. } => format!("dir {head:?}"),
            other => other.kind().to_string(),
        };
        files.insert(rel, v);
    }
    let worktrees = fx
        .git(top, &["worktree", "list", "--porcelain"])
        .lines()
        .filter(|l| l.starts_with("worktree "))
        .map(String::from)
        .collect();
    Probe {
        sym: info.sym,
        head: info.head,
        status,
        index_sha,
        index_tree,
        refs,
        files,
        worktrees,
    }
}
