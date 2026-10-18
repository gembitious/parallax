//! 가속 설정: commit-graph·Bloom 필터 읽기(단일 파일, 층 사슬)와 쓰기를 git이 만든 파일로 확인한다.

mod common;

use std::path::{Path, PathBuf};

use common::{NULL_DEVICE, git_text};
use plx_core::commit_graph::{self, GraphStatus};
use plx_core::repo::Repo;

/// 테스트를 돌리는 git의 (주, 부) 버전
fn git_version() -> (u32, u32) {
    let text = git_text(Path::new("."), &["--version"]);
    let version = text.split_whitespace().nth(2).unwrap();
    let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap());
    (parts.next().unwrap(), parts.next().unwrap())
}

/// `file://` URL(로컬 클론에서도 --depth·--filter가 듣는다). Windows 경로(D:/…)는 슬래시를 하나 더 붙인다.
fn file_url(path: &Path) -> String {
    let path = path.display().to_string().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

/// 층 파일의 BDAT 머리에 적힌 필터 버전을 바꾼다(git 2.46 없이 v2 층을 흉내 낸다: 우리는 머리만 읽는다).
fn set_bloom_version(file: &Path, version: u32) {
    let mut bytes = std::fs::read(file).unwrap();
    let chunks = usize::from(bytes[6]);
    let offset = (0..chunks)
        .find_map(|i| {
            let entry = &bytes[8 + 12 * i..8 + 12 * (i + 1)];
            (&entry[..4] == b"BDAT").then(|| u64::from_be_bytes(entry[4..].try_into().unwrap()))
        })
        .expect("BDAT 청크") as usize;
    bytes[offset..offset + 4].copy_from_slice(&version.to_be_bytes());
    let mut permissions = std::fs::metadata(file).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(file, permissions).unwrap();
    std::fs::write(file, bytes).unwrap();
}

fn run(repo: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "-c",
            "core.quotepath=false",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// 커밋 `n`개(한글 경로 포함)를 가진 새 저장소
fn repo(name: &str, n: usize) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("한글")).unwrap();
    run(&dir, &["init", "-q", "-b", "main"]);
    for i in 0..n {
        std::fs::write(
            dir.join("한글").join(format!("파일 {}.txt", i % 3)),
            format!("{i}\n"),
        )
        .unwrap();
        run(&dir, &["add", "-A"]);
        run(&dir, &["commit", "-q", "-m", &format!("commit {i}")]);
    }
    dir
}

fn status(dir: &Path) -> GraphStatus {
    commit_graph::status(&Repo::discover(dir).unwrap()).unwrap()
}

#[test]
fn no_graph_then_graph_without_bloom() {
    let dir = repo("graph-plain", 5);
    let none = status(&dir);
    assert!(
        !none.graph && !none.bloom && !none.accelerated(),
        "{none:?}"
    );
    assert!(!none.head_covered);

    run(&dir, &["commit-graph", "write", "--reachable"]);
    let plain = status(&dir);
    assert!(plain.graph && !plain.bloom, "{plain:?}");
    assert_eq!(plain.commits, 5);
    assert!(plain.head_covered);
    assert!(!plain.accelerated());

    // 그 뒤 커밋은 그래프에 없다
    std::fs::write(dir.join("new.txt"), "x\n").unwrap();
    run(&dir, &["add", "-A"]);
    run(&dir, &["commit", "-q", "-m", "after"]);
    assert!(!status(&dir).head_covered);
}

#[test]
fn write_creates_bloom_filters_without_touching_history() {
    let dir = repo("graph-write", 7);
    let before = (
        git_text(&dir, &["rev-parse", "HEAD"]),
        git_text(&dir, &["for-each-ref"]),
        git_text(&dir, &["status", "--porcelain"]),
    );
    // git은 2초가 지나야 진행을 보이므로 작은 저장소에서는 진행 알림이 없다(형식은 단위 테스트에서 본다).
    let ready = status(&dir);
    assert_eq!(ready.blocked, None, "{ready:?}");
    assert_eq!(ready.write_version, 1, "필터가 없으면 git 기본값 v1로 쓴다");
    let written = commit_graph::write(&Repo::discover(&dir).unwrap(), |_| {}).unwrap();
    assert!(written.accelerated() && written.head_covered, "{written:?}");
    assert!(!written.writing && written.blocked.is_none(), "{written:?}");
    assert_eq!(written.commits, 7);
    assert_eq!(written.bloom_version, Some(1));
    let after = (
        git_text(&dir, &["rev-parse", "HEAD"]),
        git_text(&dir, &["for-each-ref"]),
        git_text(&dir, &["status", "--porcelain"]),
    );
    assert_eq!(before, after, "커밋·참조·작업트리는 그대로다");
    // git도 필터를 읽는다: 경로 히스토리가 같다
    let log = git_text(&dir, &["log", "--format=%H", "--", "한글/파일 1.txt"]);
    assert_eq!(log.lines().count(), 2);
}

#[test]
fn split_chains_are_read_layer_by_layer() {
    let dir = repo("graph-split", 4);
    run(
        &dir,
        &[
            "commit-graph",
            "write",
            "--reachable",
            "--split",
            "--changed-paths",
        ],
    );
    for i in 0..3 {
        std::fs::write(dir.join(format!("more{i}.txt")), "x\n").unwrap();
        run(&dir, &["add", "-A"]);
        run(&dir, &["commit", "-q", "-m", &format!("more {i}")]);
    }
    run(
        &dir,
        &[
            "commit-graph",
            "write",
            "--reachable",
            "--split=no-merge",
            "--changed-paths",
        ],
    );
    let chain =
        std::fs::read_to_string(dir.join(".git/objects/info/commit-graphs/commit-graph-chain"))
            .unwrap();
    assert_eq!(chain.lines().count(), 2, "층 둘");
    let split = status(&dir);
    assert!(split.accelerated(), "{split:?}");
    assert_eq!(split.commits, 7);

    // 새 층에 필터가 없으면(필터 없이 쓴 층) 가속되지 않은 것으로 본다
    std::fs::write(dir.join("last.txt"), "x\n").unwrap();
    run(&dir, &["add", "-A"]);
    run(&dir, &["commit", "-q", "-m", "last"]);
    run(
        &dir,
        &[
            "commit-graph",
            "write",
            "--reachable",
            "--split=no-merge",
            "--no-changed-paths",
        ],
    );
    let mixed = status(&dir);
    assert!(mixed.graph && mixed.head_covered, "{mixed:?}");
    assert!(!mixed.bloom && !mixed.accelerated(), "{mixed:?}");
    assert_eq!(mixed.commits, 8);
}

#[test]
fn empty_repository_has_nothing_to_walk() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("graph-empty");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    run(&dir, &["init", "-q"]);
    let empty = status(&dir);
    assert!(!empty.graph && empty.head_covered, "{empty:?}");
}

#[test]
fn new_commits_keep_the_filters_in_use() {
    let dir = repo("graph-new-commits", 3);
    commit_graph::write(&Repo::discover(&dir).unwrap(), |_| {}).unwrap();
    std::fs::write(dir.join("new.txt"), "x\n").unwrap();
    run(&dir, &["add", "-A"]);
    run(&dir, &["commit", "-q", "-m", "after"]);
    // 그래프 밖의 커밋(새 커밋, 브랜치에서 닿지 않는 detached HEAD)은 필터 없이 걷지만 나머지는 필터를 쓴다.
    let after = status(&dir);
    assert!(after.accelerated() && !after.head_covered, "{after:?}");
}

#[test]
fn a_single_file_wins_over_a_chain() {
    // git은 객체 디렉터리마다 info/commit-graph를 먼저 읽는다. 사슬에 필터가 있어도 단일 파일에 없으면 쓰지 않는다.
    let dir = repo("graph-single-and-chain", 3);
    run(
        &dir,
        &["commit-graph", "write", "--reachable", "--no-changed-paths"],
    );
    let info = dir.join(".git/objects/info");
    let single = std::fs::read(info.join("commit-graph")).unwrap();
    std::fs::remove_file(info.join("commit-graph")).unwrap();
    run(
        &dir,
        &[
            "commit-graph",
            "write",
            "--reachable",
            "--split",
            "--changed-paths",
        ],
    );
    assert!(status(&dir).accelerated(), "사슬만 있으면 필터를 쓴다");
    std::fs::write(info.join("commit-graph"), single).unwrap();
    let both = status(&dir);
    assert!(both.graph && !both.bloom && !both.accelerated(), "{both:?}");
}

#[test]
fn layers_with_different_filters_are_not_accelerated() {
    // git은 맨 위 층과 필터 머리(버전, 해시 수, 비트)가 다른 층의 필터를 버린다.
    let dir = repo("graph-mixed-versions", 3);
    run(
        &dir,
        &[
            "commit-graph",
            "write",
            "--reachable",
            "--split",
            "--changed-paths",
        ],
    );
    std::fs::write(dir.join("more.txt"), "x\n").unwrap();
    run(&dir, &["add", "-A"]);
    run(&dir, &["commit", "-q", "-m", "more"]);
    run(
        &dir,
        &[
            "commit-graph",
            "write",
            "--reachable",
            "--split=no-merge",
            "--changed-paths",
        ],
    );
    let graphs = dir.join(".git/objects/info/commit-graphs");
    let chain = std::fs::read_to_string(graphs.join("commit-graph-chain")).unwrap();
    let top: Vec<&str> = chain.lines().collect();
    assert_eq!(top.len(), 2);
    assert!(status(&dir).accelerated());
    set_bloom_version(&graphs.join(format!("graph-{}.graph", top[1])), 2);
    let mixed = status(&dir);
    assert!(mixed.graph && !mixed.bloom, "{mixed:?}");
    assert_eq!(mixed.bloom_version, Some(2), "맨 위 층의 버전");
    if git_version() < (2, 46) {
        // v2 그래프를 다시 쓰면 git 2.39~2.45가 비정상 종료한다.
        let blocked = mixed.blocked.as_deref().unwrap_or_default();
        assert!(blocked.contains("v2"), "{mixed:?}");
        let refused = commit_graph::write(&Repo::discover(&dir).unwrap(), |_| {}).unwrap_err();
        assert!(refused.to_string().contains("v2"), "{refused}");
    } else {
        assert_eq!(mixed.blocked, None, "{mixed:?}");
        assert_eq!(mixed.write_version, 2, "그래프에 있는 버전으로 다시 쓴다");
    }
}

#[test]
fn a_v2_graph_follows_the_git_version() {
    let dir = repo("graph-v2", 3);
    run(
        &dir,
        &["commit-graph", "write", "--reachable", "--changed-paths"],
    );
    set_bloom_version(&dir.join(".git/objects/info/commit-graph"), 2);
    let v2 = status(&dir);
    assert_eq!(v2.bloom_version, Some(2));
    if git_version() < (2, 46) {
        assert!(
            !v2.bloom && v2.blocked.is_some(),
            "이 git은 v2를 읽지 못한다: {v2:?}"
        );
    } else {
        assert!(v2.accelerated() && v2.blocked.is_none(), "{v2:?}");
        assert_eq!(v2.write_version, 2);
    }
}

#[test]
fn repositories_where_git_ignores_the_graph_are_blocked() {
    let src = repo("graph-compat-source", 4);
    run(
        &src,
        &["commit-graph", "write", "--reachable", "--changed-paths"],
    );
    let parent = src.parent().unwrap();
    let check = |name: &str, dir: &Path, reason: &str| {
        let st = status(dir);
        assert!(!st.accelerated(), "{name}: {st:?}");
        let blocked = st.blocked.clone().unwrap_or_default();
        assert!(blocked.contains(reason), "{name}: {st:?}");
        let before: Vec<_> = std::fs::read_dir(dir.join(".git/objects/info"))
            .map(|d| d.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        let refused = commit_graph::write(&Repo::discover(dir).unwrap(), |_| {}).unwrap_err();
        assert!(refused.to_string().contains(reason), "{name}: {refused}");
        let after: Vec<_> = std::fs::read_dir(dir.join(".git/objects/info"))
            .map(|d| d.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert_eq!(before, after, "{name}: 아무것도 쓰지 않는다");
    };

    // 얕은 클론: git은 그래프를 읽지도 쓰지도 않는다(쓰기는 성공으로 끝난다).
    let shallow = parent.join("graph-compat-shallow");
    let _ = std::fs::remove_dir_all(&shallow);
    run(
        parent,
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            &file_url(&src),
            "graph-compat-shallow",
        ],
    );
    check("얕은 클론", &shallow, "얕은 클론");

    // git replace로 부모를 바꾼 저장소. 그래프 파일이 있어도 git은 읽지 않는다.
    let replaced = parent.join("graph-compat-replace");
    let _ = std::fs::remove_dir_all(&replaced);
    run(
        parent,
        &["clone", "-q", &file_url(&src), "graph-compat-replace"],
    );
    run(
        &replaced,
        &["commit-graph", "write", "--reachable", "--changed-paths"],
    );
    assert!(status(&replaced).accelerated());
    run(&replaced, &["replace", "--graft", "HEAD", "HEAD~2"]);
    check("git replace", &replaced, "git replace");
    // core.useReplaceRefs=false면 git은 바꾸는 참조를 보지 않는다.
    run(&replaced, &["config", "core.useReplaceRefs", "false"]);
    assert!(status(&replaced).accelerated());
    run(&replaced, &["config", "--unset", "core.useReplaceRefs"]);
    run(
        &replaced,
        &[
            "replace",
            "-d",
            &git_text(&replaced, &["rev-parse", "HEAD"]),
        ],
    );
    assert!(status(&replaced).accelerated());

    // core.commitGraph=false
    run(&replaced, &["config", "core.commitGraph", "false"]);
    check(
        "core.commitGraph=false",
        &replaced,
        "core.commitGraph=false",
    );
    run(&replaced, &["config", "--unset", "core.commitGraph"]);

    // info/grafts
    let head = git_text(&replaced, &["rev-parse", "HEAD"]);
    std::fs::write(replaced.join(".git/info/grafts"), format!("{head}\n")).unwrap();
    check("info/grafts", &replaced, "info/grafts");
    std::fs::remove_file(replaced.join(".git/info/grafts")).unwrap();
    assert!(status(&replaced).accelerated());
}

#[test]
fn treeless_partial_clones_are_blocked() {
    let src = repo("graph-partial-source", 3);
    run(&src, &["config", "uploadpack.allowFilter", "true"]);
    let parent = src.parent().unwrap();
    for (name, filter, blocked) in [
        ("graph-partial-tree", "tree:0", true),
        ("graph-partial-blob", "blob:none", false),
    ] {
        let dir = parent.join(name);
        let _ = std::fs::remove_dir_all(&dir);
        run(
            parent,
            &[
                "clone",
                "-q",
                &format!("--filter={filter}"),
                &file_url(&src),
                name,
            ],
        );
        let st = status(&dir);
        assert_eq!(st.blocked.is_some(), blocked, "{filter}: {st:?}");
        if blocked {
            assert!(st.blocked.unwrap().contains(filter));
        } else {
            // 블롭만 받지 않은 클론은 트리가 있어 필터를 만들 수 있다(받아 오는 것 없이).
            let packs = |dir: &Path| {
                std::fs::read_dir(dir.join(".git/objects/pack"))
                    .unwrap()
                    .count()
            };
            let before = packs(&dir);
            let written = commit_graph::write(&Repo::discover(&dir).unwrap(), |_| {}).unwrap();
            assert!(written.accelerated(), "{written:?}");
            assert_eq!(packs(&dir), before, "객체를 받아 오지 않는다");
        }
    }
}

#[test]
fn alternates_share_their_graph() {
    // git clone --shared(·--reference)는 원본의 객체 디렉터리를 alternates로 쓰고 git은 그 그래프를 읽는다.
    let src = repo("graph-alt-source", 3);
    run(
        &src,
        &["commit-graph", "write", "--reachable", "--changed-paths"],
    );
    let parent = src.parent().unwrap();
    let shared = parent.join("graph-alt-shared");
    let _ = std::fs::remove_dir_all(&shared);
    run(
        parent,
        &[
            "clone",
            "-q",
            "--shared",
            &src.display().to_string(),
            "graph-alt-shared",
        ],
    );
    assert!(!shared.join(".git/objects/info/commit-graph").exists());
    let st = status(&shared);
    assert!(st.accelerated() && st.head_covered, "{st:?}");
}
