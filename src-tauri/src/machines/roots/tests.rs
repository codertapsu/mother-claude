//! The roots' model, merge, import, mapping and checks, without an app.

use super::*;
use crate::claude::backup::pairs::Removed as PairRemoved;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccc";
const D: &str = "dddddddddddddddddddddddddddddddd";
const R1: &str = "rt_1111111111111111";
const R2: &str = "rt_2222222222222222";
const R3: &str = "rt_3333333333333333";
const P1: &str = "fp_1111111111111111";
const P2: &str = "fp_2222222222222222";
const P3: &str = "fp_3333333333333333";

fn root(id: &str, name: &str, paths: &[(&str, &str)], at: i64, by: &str) -> Root {
    Root {
        id: id.to_string(),
        name: name.to_string(),
        paths: paths
            .iter()
            .map(|(m, p)| (m.to_string(), p.to_string()))
            .collect(),
        updated_at: at,
        by: by.to_string(),
        pair: None,
        removed: None,
        extra: BTreeMap::new(),
    }
}

fn removed(mut r: Root, at: i64, by: &str) -> Root {
    r.removed = Some(Removed {
        at,
        by: by.to_string(),
    });
    r.paths.clear();
    r
}

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

fn pair_removed(p: &FolderPair, at: i64) -> FolderPair {
    FolderPair {
        removed: Some(PairRemoved {
            at,
            by: B.to_string(),
        }),
        ..p.clone()
    }
}

#[test]
fn root_ids_are_made_and_checked() {
    let id = new_root_id();
    assert!(is_root_id(&id), "{id}");
    assert_ne!(new_root_id(), id);
    for bad in [
        "rt_",
        "rt_0123456789abcdeF",
        "fp_0123456789abcdef",
        "rt_0123456789abcde",
    ] {
        assert!(!is_root_id(bad), "{bad}");
    }
    assert_eq!(
        root_id_of_pair("fp_0123456789abcdef").as_deref(),
        Some("rt_0123456789abcdef")
    );
    assert_eq!(root_id_of_pair("rt_0123456789abcdef"), None);
}

#[test]
fn what_is_read_is_checked() {
    let ok = root(R1, "projects", &[(A, "/Users/m/dev"), (B, r"D:\dev")], 1, A);
    assert!(ok.well_formed());
    let long = "n".repeat(MAX_NAME + 1);
    for bad in [
        root("rt_x", "projects", &[(A, "/a")], 1, A),
        root(R1, "", &[(A, "/a")], 1, A),
        root(R1, &long, &[(A, "/a")], 1, A),
        root(R1, " padded ", &[(A, "/a")], 1, A),
        root(R1, "tab\there", &[(A, "/a")], 1, A),
        root(R1, "p", &[(A, "dev")], 1, A),
        root(R1, "p", &[(A, "/a/../b")], 1, A),
        root(R1, "p", &[(A, "/a\nb")], 1, A),
        root(R1, "p", &[("A", "/a")], 1, A),
        root(R1, "p", &[], 1, A),
    ] {
        assert!(!bad.well_formed(), "{bad:?}");
    }
    let mut from_pair = ok.clone();
    from_pair.pair = Some("not a pair".into());
    assert!(!from_pair.well_formed());
    assert!(removed(ok, 2, B).well_formed(), "a tombstone has no paths");
    assert_eq!(
        clean_name(&"é".repeat(MAX_NAME)).unwrap().chars().count(),
        MAX_NAME
    );
    assert_eq!(clean_name("  dev ").unwrap(), "dev");
}

#[test]
fn unknown_fields_survive_and_a_hello_without_roots_still_reads() {
    let v = json!({
        "id": R1, "name": "dev", "paths": { A: "/a" }, "updatedAt": 3, "by": A,
        "later": { "x": 1 }
    });
    let r: Root = serde_json::from_value(v).unwrap();
    assert_eq!(r.extra["later"], json!({ "x": 1 }));
    assert_eq!(serde_json::to_value(&r).unwrap()["later"]["x"], 1);
    let hello: Hello = serde_json::from_value(json!({
        "machine": {
            "id": A, "name": "Mac", "os": "macos", "key": "k",
            "versions": { "app": "0.10.2", "protocol": 1 }
        },
        "relay": { "mode": "public" },
        "grantsYouHave": {},
        "capabilities": ["hello"]
    }))
    .unwrap();
    assert_eq!(hello.roots_digest, None, "an older machine sends no digest");
    assert!(serde_json::to_value(&hello)
        .unwrap()
        .get("rootsDigest")
        .is_none());
}

// ─── mapping ───────────────────────────────────────────────────────────────────

#[test]
fn the_longest_root_maps_both_ways() {
    let roots = [
        root(R1, "dev", &[(A, "/Users/m/dev"), (B, r"D:\dev")], 1, A),
        root(
            R2,
            "work",
            &[(A, "/Users/m/dev/work"), (B, r"E:\work")],
            2,
            A,
        ),
    ];
    assert_eq!(
        map_in(&roots, A, B, "/Users/m/dev/app/src").as_deref(),
        Some(r"D:\dev\app\src")
    );
    assert_eq!(
        map_in(&roots, A, B, "/Users/m/dev/work/api").as_deref(),
        Some(r"E:\work\api")
    );
    assert_eq!(
        map_in(&roots, B, A, r"E:\work\api").as_deref(),
        Some("/Users/m/dev/work/api")
    );
    assert_eq!(
        map_in(&roots, B, A, r"D:\dev").as_deref(),
        Some("/Users/m/dev"),
        "the root itself"
    );
}

#[test]
fn windows_paths_map_whatever_their_case_and_separators() {
    let roots = [root(
        R1,
        "dev",
        &[(A, "/Users/m/dev"), (B, r"D:\Dev")],
        1,
        A,
    )];
    for p in [r"d:\dev\app", "D:/DEV/app", r"d:/Dev\app\"] {
        assert_eq!(
            map_in(&roots, B, A, p).as_deref(),
            Some("/Users/m/dev/app"),
            "{p}"
        );
    }
    // A Mac's paths are compared exactly, as the folder pairs do.
    assert_eq!(map_in(&roots, A, B, "/users/m/dev/app"), None);
    assert_eq!(
        map_in(&roots, A, B, "/Users/m/dev/app").as_deref(),
        Some(r"D:\Dev\app")
    );
}

#[test]
fn a_path_outside_every_root_maps_nowhere() {
    let roots = [root(
        R1,
        "dev",
        &[(A, "/Users/m/dev"), (B, r"D:\dev")],
        1,
        A,
    )];
    for p in [
        "/Users/m/dev2/app",
        "/Users/m",
        "/Users/other/dev/app",
        "dev/app",
        "/Users/m/dev/../x",
    ] {
        assert_eq!(map_in(&roots, A, B, p), None, "{p}");
    }
    assert_eq!(map_in(&roots, B, A, r"D:\development\app"), None);
    assert_eq!(
        map_in(&roots, A, C, "/Users/m/dev/app"),
        None,
        "no path on C"
    );
    assert_eq!(
        map_in(&roots, A, A, "/Users/m/dev/app"),
        None,
        "one machine"
    );
}

#[test]
fn nested_roots_map_by_the_deepest() {
    let roots = [
        root(R2, "app", &[(A, "/Users/m/dev/app"), (B, r"F:\app")], 1, A),
        root(R1, "dev", &[(A, "/Users/m/dev"), (B, r"D:\dev")], 9, A),
    ];
    assert_eq!(
        map_in(&roots, A, B, "/Users/m/dev/app/x").as_deref(),
        Some(r"F:\app\x")
    );
    assert_eq!(
        map_in(&roots, A, B, "/Users/m/dev/apps").as_deref(),
        Some(r"D:\dev\apps"),
        "whole folders only"
    );
    assert_eq!(
        map_in(&roots, B, A, r"F:\app\x").as_deref(),
        Some("/Users/m/dev/app/x")
    );
    // Two roots with the same folder here: the one written last.
    let tie = [
        root(R1, "one", &[(A, "/Users/m/dev"), (B, r"D:\dev")], 1, A),
        root(R2, "two", &[(A, "/Users/m/dev"), (B, r"E:\dev")], 2, A),
    ];
    assert_eq!(
        map_in(&tie, A, B, "/Users/m/dev/x").as_deref(),
        Some(r"E:\dev\x")
    );
}

#[test]
fn one_root_maps_between_any_two_of_its_machines() {
    let roots = [root(
        R1,
        "projects",
        &[
            (A, "/Users/m/projects"),
            (B, r"D:\projects"),
            (C, "/home/m/projects"),
        ],
        1,
        A,
    )];
    assert_eq!(
        map_in(&roots, C, B, "/home/m/projects/app").as_deref(),
        Some(r"D:\projects\app")
    );
    assert_eq!(
        map_in(&roots, B, C, r"d:\projects\app").as_deref(),
        Some("/home/m/projects/app")
    );
    // From a machine not known by name.
    assert_eq!(
        map_from_any(&roots, A, "/home/m/projects/app").as_deref(),
        Some("/Users/m/projects/app")
    );
    assert_eq!(
        map_from_any(&roots, A, r"D:\projects\app").as_deref(),
        Some("/Users/m/projects/app")
    );
    assert_eq!(
        map_from_any(&roots, A, "/Users/m/projects/app"),
        None,
        "this machine's own folder is no other machine's"
    );
    assert_eq!(
        map_from_any(&roots, D, "/home/m/projects/app"),
        None,
        "a machine the root does not name"
    );
}

#[test]
fn a_removed_root_maps_nothing() {
    let gone = removed(
        root(R1, "dev", &[(A, "/Users/m/dev"), (B, r"D:\dev")], 1, A),
        2,
        A,
    );
    assert_eq!(
        map_in(std::slice::from_ref(&gone), A, B, "/Users/m/dev/app"),
        None
    );
    assert_eq!(map_from_any(&[gone], B, "/Users/m/dev/app"), None);
}

// ─── merging ───────────────────────────────────────────────────────────────────

#[test]
fn the_last_writer_wins_whatever_the_order() {
    let old = root(R1, "dev", &[(A, "/Users/m/dev"), (B, r"D:\dev")], 1, A);
    let new = root(R1, "code", &[(A, "/Users/m/code"), (B, r"D:\code")], 5, B);
    let mut x = vec![old.clone()];
    assert_eq!(merge(&mut x, std::slice::from_ref(&new)), [R1]);
    let mut y = vec![new.clone()];
    assert!(merge(&mut y, std::slice::from_ref(&old)).is_empty());
    assert_eq!(x, y);
    assert_eq!(x[0].name, "code");
    // At the same time: the higher machine id.
    let a = root(R2, "from-a", &[(A, "/a")], 7, A);
    let b = root(R2, "from-b", &[(A, "/b")], 7, B);
    let mut p = vec![a.clone()];
    merge(&mut p, std::slice::from_ref(&b));
    let mut q = vec![b];
    merge(&mut q, &[a]);
    assert_eq!(p, q);
    assert_eq!(p[0].name, "from-b");
}

#[test]
fn a_removal_sticks() {
    let live = root(R1, "dev", &[(A, "/Users/m/dev")], 1, A);
    let gone = removed(live.clone(), 3, B);
    // Edited later on a machine that had not heard of the removal.
    let later = root(R1, "dev", &[(A, "/Users/m/dev2")], 9, A);
    let mut x = vec![live.clone()];
    merge(&mut x, std::slice::from_ref(&gone));
    assert!(!x[0].live());
    assert!(x[0].paths.is_empty());
    assert!(
        merge(&mut x, &[later, live.clone()]).is_empty(),
        "no copy brings it back"
    );
    assert!(!x[0].live());
    // Of two removals, the earlier is kept.
    merge(&mut x, &[removed(live, 2, C)]);
    assert_eq!(x[0].removed.as_ref().map(|r| r.at), Some(2));
    // A tombstone that arrives with paths keeps none.
    let mut with_paths = removed(root(R2, "w", &[], 1, A), 4, A);
    with_paths.paths.insert(A.into(), "/w".into());
    let mut y = Vec::new();
    merge(&mut y, &[with_paths]);
    assert!(y[0].paths.is_empty());
}

fn permutations(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for p in permutations(n - 1) {
        for i in 0..=p.len() {
            let mut q = p.clone();
            q.insert(i, n - 1);
            out.push(q);
        }
    }
    out
}

/// Every machine ends with the same roots, whatever order the copies came
/// in, and merging what it has again changes nothing.
#[test]
fn merging_is_the_same_in_any_order_and_twice() {
    let versions = [
        root(R1, "dev", &[(A, "/Users/m/dev")], 1, A),
        root(R1, "dev2", &[(A, "/Users/m/dev")], 4, B),
        removed(root(R1, "dev", &[], 3, C), 5, C),
        root(R2, "work", &[(B, r"D:\work")], 2, B),
        root(R2, "work", &[(B, r"D:\work"), (C, "/work")], 2, C),
        root(R3, "x", &[(C, "/x")], 5, C),
    ];
    let mut first: Option<Vec<Root>> = None;
    for order in permutations(versions.len()) {
        let mut out = Vec::new();
        for i in order {
            merge(&mut out, std::slice::from_ref(&versions[i]));
        }
        let again = out.clone();
        assert!(merge(&mut out, &again).is_empty());
        match &first {
            None => first = Some(out),
            Some(f) => assert_eq!(&out, f),
        }
    }
    let end = first.unwrap();
    assert!(!end[0].live(), "R1 removed");
    assert_eq!(end[1].paths.len(), 2, "R2: the higher machine id at a tie");
    assert_eq!(digest_of(&end), digest_of(&end.clone()));
}

#[test]
fn too_many_tombstones_drop_the_earliest_removed() {
    let mut roots: Vec<Root> = (0..MAX_TOMBSTONES + 3)
        .map(|i| removed(root(&format!("rt_{i:016x}"), "x", &[], 1, A), i as i64, A))
        .collect();
    roots.push(root(R1, "live", &[(A, "/a")], 1, A));
    tidy(&mut roots);
    assert_eq!(roots.iter().filter(|r| !r.live()).count(), MAX_TOMBSTONES);
    assert!(roots
        .iter()
        .all(|r| r.removed.as_ref().is_none_or(|x| x.at >= 3)));
    assert!(roots.iter().any(|r| r.id == R1));
    assert!(roots.windows(2).all(|w| w[0].id < w[1].id), "sorted by id");
}

// ─── importing folder pairs ────────────────────────────────────────────────────

#[test]
fn folder_pairs_are_imported_once_as_two_entry_roots() {
    let pairs = [
        pair(P1, "/Users/m/dev", r"D:\dev", 10),
        pair(P2, "/Users/m/work", r"E:\work", 20),
    ];
    let (mut roots, mut imported) = (Vec::new(), Vec::new());
    let done = import_into(&mut roots, &mut imported, &pairs);
    assert_eq!(done.added, [R1, R2]);
    assert_eq!(done.noted, [P1, P2]);
    let r = &roots[0];
    assert_eq!(r.name, "dev");
    assert_eq!(r.paths.len(), 2);
    assert_eq!(r.pair.as_deref(), Some(P1));
    assert_eq!((r.updated_at, r.by.as_str()), (10, A));
    assert!(r.well_formed());
    // They map as the pairs did.
    for (from, to, p) in [
        (B, A, r"d:\dev\app"),
        (A, B, "/Users/m/work/api"),
        (A, C, "/Users/m/dev/x"),
    ] {
        assert_eq!(
            map_in(&roots, from, to, p),
            crate::claude::backup::pairs::map_between(&pairs, from, to, p),
            "{p}"
        );
    }
    // Twice: nothing.
    assert_eq!(
        import_into(&mut roots, &mut imported, &pairs),
        Imported::default()
    );
}

#[test]
fn a_root_the_user_removed_is_never_imported_again() {
    let pairs = [pair(P1, "/Users/m/dev", r"D:\dev", 10)];
    let (mut roots, mut imported) = (Vec::new(), Vec::new());
    import_into(&mut roots, &mut imported, &pairs);
    roots[0] = removed(roots[0].clone(), 50, A);
    assert_eq!(
        import_into(&mut roots, &mut imported, &pairs),
        Imported::default()
    );
    // Even without the record of the import, its tombstone keeps it out…
    let done = import_into(&mut roots, &mut Vec::new(), &pairs);
    assert!(done.added.is_empty());
    assert_eq!(roots.len(), 1);
    assert!(!roots[0].live());
    // …and so does a copy another machine imported before the removal.
    let mut theirs = Vec::new();
    import_into(&mut theirs, &mut Vec::new(), &pairs);
    assert!(merge(&mut roots, &theirs).is_empty());
    assert!(!roots[0].live());
}

#[test]
fn two_machines_import_one_pair_as_one_root() {
    let pairs = [pair(P1, "/Users/m/dev", r"D:\dev", 10)];
    let (mut on_a, mut on_b) = (Vec::new(), Vec::new());
    import_into(&mut on_a, &mut Vec::new(), &pairs);
    import_into(&mut on_b, &mut Vec::new(), &pairs);
    assert_eq!(on_a, on_b);
    assert!(merge(&mut on_a, &on_b).is_empty());
    assert_eq!(digest_of(&on_a), digest_of(&on_b));
}

#[test]
fn a_pair_removed_in_the_backups_removes_its_root_unless_edited() {
    let pairs = [
        pair(P1, "/Users/m/dev", r"D:\dev", 10),
        pair(P2, "/Users/m/work", r"E:\work", 20),
    ];
    let (mut roots, mut imported) = (Vec::new(), Vec::new());
    import_into(&mut roots, &mut imported, &pairs);
    // P2's root got a third machine's folder since.
    roots[1].paths.insert(C.into(), "/home/m/work".into());
    roots[1].updated_at = 30;
    let gone: Vec<FolderPair> = pairs.iter().map(|p| pair_removed(p, 40)).collect();
    let done = import_into(&mut roots, &mut imported, &gone);
    assert_eq!(done.removed, [R1]);
    assert!(!roots[0].live());
    assert_eq!(roots[0].removed.as_ref().map(|r| r.at), Some(40));
    assert!(roots[1].live(), "edited since: the user's now");
    // A pair first seen removed is noted, never imported.
    let p3 = pair(P3, "/Users/m/x", r"C:\x", 5);
    let done = import_into(&mut roots, &mut imported, &[pair_removed(&p3, 6)]);
    assert!(done.added.is_empty());
    assert_eq!(done.noted, [P3]);
    assert!(import_into(&mut roots, &mut imported, &[p3])
        .added
        .is_empty());
}

#[test]
fn a_malformed_pair_is_not_imported() {
    let mut bad = pair(P1, "/Users/m/dev", "dev", 10);
    let done = import_into(&mut Vec::new(), &mut Vec::new(), std::slice::from_ref(&bad));
    assert_eq!(done, Imported::default());
    bad.id = "fp_nope".into();
    bad.folders.insert(B.into(), r"D:\dev".into());
    let done = import_into(&mut Vec::new(), &mut Vec::new(), &[bad]);
    assert_eq!(done, Imported::default());
}

// ─── changes asked for ─────────────────────────────────────────────────────────

/// A folder in this machine's form.
fn here_path(parts: &[&str]) -> String {
    match PathForm::here() {
        PathForm::Windows => format!(r"C:\{}", parts.join("\\")),
        PathForm::Posix => format!("/{}", parts.join("/")),
    }
}

/// A full path, but in the other system's form.
fn other_form() -> &'static str {
    match PathForm::here() {
        PathForm::Windows => "/Users/m/dev",
        PathForm::Posix => r"D:\dev",
    }
}

/// This machine is A; B runs Windows, C macOS, D an OS nobody said.
fn known() -> Known {
    let m = |id: &str, os: Option<&str>, here: bool| {
        (
            id.to_string(),
            KnownMachine {
                id: id.to_string(),
                name: format!("machine {}", &id[..1]),
                os: os.map(str::to_string),
                here,
                paired: !here,
            },
        )
    };
    Known {
        here: A.to_string(),
        machines: [
            m(A, Some(super::super::protocol::this_os()), true),
            m(B, Some("windows"), false),
            m(C, Some("macos"), false),
            m(D, None, false),
        ]
        .into_iter()
        .collect(),
        home: Some(PathBuf::from(here_path(&["Users", "me"]))),
    }
}

fn input(id: Option<&str>, name: &str, paths: &[(&str, &str)]) -> RootInput {
    RootInput {
        id: id.map(str::to_string),
        name: name.to_string(),
        paths: paths
            .iter()
            .map(|(m, p)| (m.to_string(), p.to_string()))
            .collect(),
    }
}

fn upsert(list: Vec<RootInput>) -> Change {
    Change {
        upsert: list,
        remove: Vec::new(),
    }
}

fn remove(id: &str) -> Change {
    Change {
        upsert: Vec::new(),
        remove: vec![id.to_string()],
    }
}

#[test]
fn a_change_adds_edits_and_removes_roots() {
    let k = known();
    let mine = here_path(&["dev"]);
    let mut roots = Vec::new();
    let done = apply_change(
        &mut roots,
        &upsert(vec![input(
            None,
            " projects ",
            &[
                (A, &mine),
                (B, "d:/dev/"),
                (C, "/Users/m/dev/"),
                (D, "/srv/dev"),
            ],
        )]),
        &k,
        100,
    )
    .unwrap();
    assert_eq!(done.added.len(), 1);
    let r = roots[0].clone();
    assert_eq!(r.name, "projects");
    assert_eq!(r.paths[A], mine);
    assert_eq!(r.paths[B], r"D:\dev", "written the way Windows writes it");
    assert_eq!(r.paths[C], "/Users/m/dev");
    assert_eq!(r.paths[D], "/srv/dev");
    assert_eq!((r.updated_at, r.by.as_str()), (100, A));
    assert!(r.well_formed());

    // `~` is this machine's home folder.
    apply_change(
        &mut roots,
        &upsert(vec![input(None, "home", &[(A, "~/work")])]),
        &k,
        101,
    )
    .unwrap();
    let home = roots.iter().find(|x| x.name == "home").unwrap();
    assert_eq!(home.paths[A], here_path(&["Users", "me", "work"]));

    // An edit is later than the copy it replaces, even on a clock behind.
    let edit = upsert(vec![input(
        Some(&r.id),
        "code",
        &[(A, &mine), (B, r"D:\code")],
    )]);
    let done = apply_change(&mut roots, &edit, &k, 50).unwrap();
    assert_eq!(done.changed, [(r.id.clone(), "code".to_string())]);
    let e = roots.iter().find(|x| x.id == r.id).unwrap();
    assert_eq!(e.updated_at, 101);
    assert_eq!(e.paths.len(), 2, "a machine left out loses its path");
    // The same again changes nothing.
    assert!(apply_change(&mut roots, &edit, &k, 200).unwrap().is_empty());

    // Removed: a tombstone, which nothing edits or removes again.
    let done = apply_change(&mut roots, &remove(&r.id), &k, 300).unwrap();
    assert_eq!(done.removed, [(r.id.clone(), "code".to_string())]);
    let t = roots.iter().find(|x| x.id == r.id).unwrap();
    assert!(!t.live() && t.paths.is_empty());
    assert_eq!(t.removed.as_ref().map(|x| x.by.as_str()), Some(A));
    let code = |c: &Change| {
        apply_change(&mut roots.clone(), c, &k, 400)
            .unwrap_err()
            .code
    };
    assert_eq!(code(&edit), "root_removed");
    assert_eq!(code(&remove(&r.id)), "root_removed");
    assert_eq!(code(&remove(R3)), "root_not_found");
    assert_eq!(code(&remove("rt_nope")), "roots_bad_request");
}

#[test]
fn a_change_is_checked_whole() {
    let k = known();
    let mine = here_path(&["dev"]);
    let dotted = here_path(&["a", "..", "b"]);
    let code = |c: &Change| apply_change(&mut Vec::new(), c, &k, 1).unwrap_err().code;
    for (paths, why) in [
        (vec![(A, "dev")], "roots_bad_path"),
        (vec![(A, other_form())], "roots_bad_path"),
        (vec![(A, dotted.as_str())], "roots_bad_path"),
        (vec![(B, "/Users/m/dev")], "roots_bad_path"),
        (vec![(C, r"D:\dev")], "roots_bad_path"),
        (vec![(D, "dev")], "roots_bad_path"),
        (vec![(A, mine.as_str()), (B, "")], "roots_bad_path"),
        (
            vec![("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee", "/x")],
            "roots_unknown_machine",
        ),
        (vec![("not a machine", "/x")], "roots_unknown_machine"),
        (vec![], "roots_no_path"),
    ] {
        assert_eq!(
            code(&upsert(vec![input(None, "dev", &paths)])),
            why,
            "{paths:?}"
        );
    }
    let long = "n".repeat(MAX_NAME + 1);
    for name in ["", "  ", long.as_str(), "a\u{7}b"] {
        assert_eq!(
            code(&upsert(vec![input(None, name, &[(A, &mine)])])),
            "roots_bad_name",
            "{name:?}"
        );
    }
    // Names are unique, whatever their case.
    assert_eq!(
        code(&upsert(vec![
            input(None, "dev", &[(A, &mine)]),
            input(None, "DEV", &[(A, &here_path(&["other"]))]),
        ])),
        "roots_duplicate_name"
    );
    // One id named twice.
    assert_eq!(
        code(&Change {
            upsert: vec![input(Some(R1), "dev", &[(A, &mine)])],
            remove: vec![R1.to_string()],
        }),
        "roots_bad_request"
    );
    // At most 64.
    let many: Vec<RootInput> = (0..=MAX_ROOTS)
        .map(|i| {
            let name = format!("r{i}");
            input(None, &name, &[(A, &here_path(&[&name]))])
        })
        .collect();
    assert_eq!(code(&upsert(many)), "roots_too_many");
    // A machine whose OS is not known takes a path of either form.
    let mut roots = Vec::new();
    apply_change(
        &mut roots,
        &upsert(vec![input(None, "d", &[(D, r"d:\x\")])]),
        &k,
        1,
    )
    .unwrap();
    assert_eq!(roots[0].paths[D], r"D:\x");
}

#[test]
fn an_exchange_is_made_again_only_when_something_changed() {
    let ex = Exchanges::default();
    let one = Exchange {
        mine: "m".into(),
        theirs: Some("t".into()),
        may_send: true,
        accept: false,
    };
    assert!(ex.begin(B, one.clone()));
    assert!(!ex.begin(B, one.clone()), "nothing changed");
    assert!(ex.begin(C, one.clone()), "per machine");
    assert!(ex.begin(
        B,
        Exchange {
            accept: true,
            ..one.clone()
        }
    ));
    ex.forget(B);
    assert!(ex.begin(B, one), "tried again after a failure");
}
