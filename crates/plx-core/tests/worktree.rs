//! 워크트리 목록: `git worktree list --porcelain`과 대조하고, 그래프 이름표에 다른 워크트리를 붙인다.

mod common;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use common::{git, git_text};
use plx_core::graph::{GraphEvent, GraphOptions, GraphSessions, LabelKind};
use plx_core::repo::RepoCache;
use plx_core::worktree::{self, Worktree};

fn git_as_user(repo: &Path, args: &[&str]) {
    let mut all = vec![
        "-c",
        "user.name=plx",
        "-c",
        "user.email=plx@example.com",
        "-c",
        "commit.gpgsign=false",
    ];
    all.extend_from_slice(args);
    git(repo, &all);
}

fn canonical(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap()
}

/// 주 작업트리, 브랜치 워크트리, detached 워크트리(잠김), 폴더를 지운 워크트리
fn setup(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let main = root.join("주 저장소");
    std::fs::create_dir(&main).unwrap();
    git(&main, &["init", "--quiet", "--initial-branch=main"]);
    git_as_user(
        &main,
        &["commit", "--quiet", "--allow-empty", "-m", "첫 커밋"],
    );
    let agent = root.join("에이전트 워크트리");
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "agent/1",
            agent.to_str().unwrap(),
        ],
    );
    git_as_user(
        &agent,
        &["commit", "--quiet", "--allow-empty", "-m", "에이전트 커밋"],
    );
    let detached = root.join("detached");
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            detached.to_str().unwrap(),
            "HEAD",
        ],
    );
    git(
        &main,
        &[
            "worktree",
            "lock",
            "--reason",
            "시험",
            detached.to_str().unwrap(),
        ],
    );
    let gone = root.join("gone");
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "gone",
            gone.to_str().unwrap(),
        ],
    );
    std::fs::remove_dir_all(&gone).unwrap();
    (main, agent, detached)
}

fn by_name<'a>(list: &'a [Worktree], name: &str) -> &'a Worktree {
    list.iter()
        .find(|w| w.name == name)
        .unwrap_or_else(|| panic!("{name}: {list:?}"))
}

#[test]
fn lists_main_and_linked_worktrees() {
    let temp = tempfile::tempdir().unwrap();
    let (main, agent, detached) = setup(temp.path());
    let repos = RepoCache::new();

    let list = worktree::list(&repos.for_path(&main).unwrap()).unwrap();
    assert_eq!(
        list.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
        ["주 저장소", "detached", "gone", "에이전트 워크트리"],
        "주 작업트리 먼저, 링크된 것은 이름 순"
    );
    let m = &list[0];
    assert!(m.main && m.current && !m.missing);
    assert_eq!(Path::new(&m.path), canonical(&main));
    assert_eq!(m.branch.as_deref(), Some("main"));
    assert_eq!(m.summary.as_deref(), Some("첫 커밋"));

    let a = by_name(&list, "에이전트 워크트리");
    assert!(!a.main && !a.current && !a.locked && !a.missing);
    assert_eq!(
        a.id.as_deref(),
        Some("에이전트-워크트리"),
        "git이 공백을 바꾼 id"
    );
    assert_eq!(m.id, None);
    assert_eq!(a.branch.as_deref(), Some("agent/1"));
    assert_eq!(
        a.head.as_deref(),
        Some(git_text(&agent, &["rev-parse", "HEAD"]).as_str())
    );
    assert_eq!(a.summary.as_deref(), Some("에이전트 커밋"));
    assert!(a.time.is_some());

    let d = by_name(&list, "detached");
    assert_eq!(d.branch, None);
    assert!(d.locked);
    assert_eq!(
        d.head.as_deref(),
        Some(git_text(&detached, &["rev-parse", "HEAD"]).as_str())
    );
    assert!(by_name(&list, "gone").missing);

    // git worktree list와 같은 경로·HEAD
    let porcelain = git_text(&main, &["worktree", "list", "--porcelain"]);
    for block in porcelain.split("\n\n") {
        let path = block
            .lines()
            .find_map(|l| l.strip_prefix("worktree "))
            .unwrap();
        let head = block.lines().find_map(|l| l.strip_prefix("HEAD "));
        let ours = list
            .iter()
            .find(|w| Path::new(&w.path) == canonical_or(Path::new(path)))
            .unwrap_or_else(|| panic!("{path}: {list:?}"));
        assert_eq!(ours.head.as_deref(), head, "{path}");
    }

    // 링크된 워크트리에서 보면 그것이 지금 보는 작업트리다.
    let from_agent = worktree::list(&repos.for_path(&agent).unwrap()).unwrap();
    assert!(by_name(&from_agent, "에이전트 워크트리").current);
    assert!(!from_agent[0].current);
    assert_eq!(from_agent.len(), list.len());
}

/// 지운 폴더는 canonicalize하지 못한다.
fn canonical_or(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| {
        let parent = path.parent().map(canonical).unwrap_or_default();
        parent.join(path.file_name().unwrap())
    })
}

#[test]
fn graph_labels_name_other_worktrees() {
    let temp = tempfile::tempdir().unwrap();
    let (main, agent, _) = setup(temp.path());
    let sessions = GraphSessions::new();
    let repos = RepoCache::new();
    let (tx, rx) = mpsc::channel();
    let id = sessions
        .open(&repos, &main, GraphOptions::default(), move |e| {
            tx.send(e).is_ok()
        })
        .unwrap();
    loop {
        if rx.recv_timeout(Duration::from_secs(30)).unwrap().is_end() {
            break;
        }
    }
    let rows = sessions.rows(id, 0, 100).unwrap().rows;
    let agent_head = git_text(&agent, &["rev-parse", "HEAD"]);
    let row = rows.iter().find(|r| r.sha == agent_head).unwrap();
    let label = row.labels.iter().find(|l| l.name == "agent/1").unwrap();
    assert_eq!(label.kind, LabelKind::Branch);
    assert_eq!(label.worktree.as_deref(), Some("에이전트 워크트리"));
    // 지금 보는 작업트리의 브랜치에는 워크트리 이름이 없다.
    let main_label = rows
        .iter()
        .flat_map(|r| &r.labels)
        .find(|l| l.name == "main")
        .unwrap();
    assert!(main_label.current);
    assert_eq!(main_label.worktree, None);
    // 다른 워크트리의 detached HEAD
    assert!(
        rows.iter()
            .flat_map(|r| &r.labels)
            .any(|l| l.kind == LabelKind::Head && l.worktree.as_deref() == Some("detached"))
    );
    let _ = GraphEvent::Cancelled;
}
