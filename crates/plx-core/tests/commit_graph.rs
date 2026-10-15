//! 가속 설정: commit-graph·Bloom 필터 읽기(단일 파일, 층 사슬)와 쓰기를 git이 만든 파일로 확인한다.

mod common;

use std::path::{Path, PathBuf};

use common::{NULL_DEVICE, git_text};
use plx_core::commit_graph::{self, GraphStatus};
use plx_core::repo::Repo;

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
    let written = commit_graph::write(&Repo::discover(&dir).unwrap(), |_| {}).unwrap();
    assert!(written.accelerated(), "{written:?}");
    assert_eq!(written.commits, 7);
    assert_eq!(written.bloom_version, Some(written.write_version));
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
