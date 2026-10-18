//! 에디터로 열 곳: 리비전의 줄을 작업트리 파일의 줄로 옮긴다.

mod common;

use std::path::Path;

use common::{NULL_DEVICE, git, git_text};
use plx_core::editor;
use plx_core::repo::Repo;

fn commit(repo: &Path, message: &str) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "user.name=plx", "-c", "user.email=plx@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-am",
            message,
        ])
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn maps_revision_lines_to_the_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("저장소");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::create_dir(root.join("소스")).unwrap();
    let file = root.join("소스/파일.txt");
    std::fs::write(&file, "a\nb\nc\n").unwrap();
    git(&root, &["add", "-A"]);
    commit(&root, "처음");
    let first = git_text(&root, &["rev-parse", "HEAD"]);
    // 작업트리에서 위에 두 줄을 더하고 c를 고친다(커밋하지 않는다).
    std::fs::write(&file, "x\ny\na\nb\nC\n").unwrap();

    let repo = Repo::discover(&root).unwrap();
    let at = |rev: Option<&str>, line| {
        editor::locate(&repo, Some("소스/파일.txt"), rev, Some(line))
            .unwrap()
            .line
    };
    assert_eq!(at(Some(&first), 2), Some(4), "b는 두 줄 내려갔다");
    assert_eq!(at(Some(&first), 3), Some(5), "고친 줄은 그 부분의 시작");
    assert_eq!(at(None, 2), Some(2), "작업트리 줄은 그대로");

    let target = editor::locate(&repo, Some("소스/파일.txt"), None, None).unwrap();
    assert_eq!(target.path, repo.workdir().join("소스/파일.txt"));
    assert_eq!(target.line, None);

    // 경로가 없으면 작업트리 폴더
    let folder = editor::locate(&repo, None, None, None).unwrap();
    assert_eq!(folder.path, repo.workdir());

    // 작업트리에 없는 파일
    let err = editor::locate(&repo, Some("지운.txt"), Some(&first), Some(1)).unwrap_err();
    assert!(err.to_string().contains("지운.txt"), "{err}");

    // 그 리비전에 없는 경로(나중에 생긴 파일)는 줄을 그대로 쓴다
    std::fs::write(root.join("새 파일.txt"), "1\n2\n3\n").unwrap();
    let new = editor::locate(&repo, Some("새 파일.txt"), Some(&first), Some(3)).unwrap();
    assert_eq!(new.line, Some(3));
}

/// fixtures의 rename(`src/old_name.rs` → `src/new_name.rs`, `lib/util.rs` → `core/util.rs`)을 앞으로 따라가
/// 작업트리의 새 이름을 연다.
#[test]
fn follows_renames_to_the_worktree() {
    let root = common::history();
    let repo = Repo::discover(&root).unwrap();
    let added = |path: &str| {
        git_text(
            &root,
            &["log", "--format=%H", "--diff-filter=A", "--", path],
        )
    };

    let old = added("src/old_name.rs");
    assert_eq!(
        plx_core::navigate::worktree_path(&repo, &old, "src/old_name.rs")
            .unwrap()
            .as_deref(),
        Some("src/new_name.rs")
    );
    let util = added("lib/util.rs");
    assert_eq!(
        plx_core::navigate::worktree_path(&repo, &util, "lib/util.rs")
            .unwrap()
            .as_deref(),
        Some("core/util.rs")
    );
    // 그대로 있는 파일은 그 경로
    assert_eq!(
        plx_core::navigate::worktree_path(&repo, &old, "src/lib.rs")
            .unwrap()
            .as_deref(),
        Some("src/lib.rs")
    );

    // 에디터로 열 곳: 새 이름, 줄은 옛 버전의 줄을 옮긴 것
    let target = editor::locate(&repo, Some("src/old_name.rs"), Some(&old), Some(2)).unwrap();
    assert_eq!(target.path, repo.workdir().join("src/new_name.rs"));
    let expected = plx_core::navigate::map_line(
        &repo,
        &plx_core::navigate::Version {
            rev: Some(old.clone()),
            path: "src/old_name.rs".into(),
        },
        &plx_core::navigate::Version {
            rev: None,
            path: "src/new_name.rs".into(),
        },
        2,
    )
    .unwrap();
    assert_eq!(target.line, Some(expected));
}

/// 지운 파일(rename이 아니다)은 연 곳이 없다. fixtures의 `docs/notes.md`는 지운 뒤 다른 내용으로 다시 만들었으므로
/// 작업트리에 있는 그 경로를 연다.
#[test]
fn deleted_files_have_no_worktree_path() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    std::fs::write(root.join("gone.txt"), "a\n").unwrap();
    std::fs::write(root.join("keep.txt"), "b\n").unwrap();
    git(&root, &["add", "-A"]);
    commit(&root, "첫");
    let first = git_text(&root, &["rev-parse", "HEAD"]);
    git(&root, &["rm", "-q", "gone.txt"]);
    commit(&root, "지움");
    let repo = Repo::discover(&root).unwrap();
    assert_eq!(
        plx_core::navigate::worktree_path(&repo, &first, "gone.txt").unwrap(),
        None
    );
    let err = editor::locate(&repo, Some("gone.txt"), Some(&first), Some(1)).unwrap_err();
    assert!(err.to_string().contains("gone.txt"), "{err}");

    let history = common::history();
    let fixture = Repo::discover(&history).unwrap();
    let notes = git_text(
        &history,
        &[
            "log",
            "--format=%H",
            "--diff-filter=A",
            "--",
            "docs/notes.md",
        ],
    );
    let oldest = notes.lines().last().unwrap();
    let target = editor::locate(&fixture, Some("docs/notes.md"), Some(oldest), Some(1)).unwrap();
    assert_eq!(target.path, fixture.workdir().join("docs/notes.md"));
}
