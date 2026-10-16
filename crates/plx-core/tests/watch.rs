//! 저장소 감시: 커밋·브랜치·워크트리 변화를 알리고, 상관없는 쓰기(index 등)는 알리지 않는다.

mod common;

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use common::git;
use plx_core::repo::RepoCache;
use plx_core::watch::{Change, FileChange, RepoChange, Watchers};

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

fn commit(repo: &Path, message: &str) {
    git_as_user(repo, &["commit", "--quiet", "--allow-empty", "-m", message]);
}

fn head(repo: &Path) -> String {
    String::from_utf8(git(repo, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned()
}

/// 다음 변화(없으면 None). 감시가 늦게 붙는 플랫폼(FSEvents)을 생각해 넉넉히 기다린다.
fn next(rx: &mpsc::Receiver<RepoChange>, wait: Duration) -> Option<RepoChange> {
    rx.recv_timeout(wait).ok()
}

#[test]
fn commits_branches_and_worktrees_are_reported() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("저장소");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch=main"]);
    commit(&repo, "첫 커밋");

    let (tx, rx) = mpsc::channel();
    let watchers = Watchers::new(move |change| {
        if let Change::Repo(change) = change {
            let _ = tx.send(change);
        }
    });
    let repos = RepoCache::new();
    let (id, workdir) = watchers.watch(&repos, &repo.join(".")).unwrap();
    assert_eq!(watchers.watching(), 1);
    // 감시가 자리를 잡을 때까지(macOS FSEvents는 조금 늦다)
    std::thread::sleep(Duration::from_millis(300));

    commit(&repo, "둘째 커밋");
    let change = next(&rx, Duration::from_secs(10)).expect("커밋을 알린다");
    assert_eq!(Path::new(&change.repo), Path::new(&workdir));
    assert!(change.head_changed);
    assert_eq!(change.head.as_deref(), Some(head(&repo).as_str()));
    assert!(
        next(&rx, Duration::from_millis(400)).is_none(),
        "한 번만 알린다"
    );

    // HEAD는 그대로, 다른 브랜치가 생겼다.
    git(&repo, &["branch", "곁가지"]);
    let change = next(&rx, Duration::from_secs(10)).expect("브랜치를 알린다");
    assert!(!change.head_changed);
    assert!(change.refs_changed);

    // index만 바꾸는 명령은 알리지 않는다.
    std::fs::write(repo.join("새 파일.txt"), "내용\n").unwrap();
    git(&repo, &["add", "새 파일.txt"]);
    git(&repo, &["status", "--short"]);
    assert!(
        next(&rx, Duration::from_millis(600)).is_none(),
        "index 쓰기는 무시"
    );

    // 링크된 워크트리에서 다른 브랜치에 커밋하면 참조가 바뀐다(이 작업트리의 HEAD는 그대로).
    let worktree = temp.path().join("워크트리");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "에이전트",
            worktree.to_str().unwrap(),
        ],
    );
    let _ = next(&rx, Duration::from_secs(10)); // 브랜치 생성
    commit(&worktree, "에이전트 커밋");
    let change = next(&rx, Duration::from_secs(10)).expect("다른 워크트리의 커밋을 알린다");
    assert!(!change.head_changed);
    assert!(change.refs_changed);

    // 워크트리를 직접 보면 그 HEAD가 바뀐 것도 안다.
    let (wid, wt_workdir) = watchers.watch(&repos, &worktree).unwrap();
    assert_ne!(wt_workdir, workdir);
    assert_eq!(watchers.watching(), 2);
    std::thread::sleep(Duration::from_millis(300));
    commit(&worktree, "에이전트 커밋 2");
    let mut changes = Vec::new();
    while let Some(change) = next(&rx, Duration::from_secs(3)) {
        changes.push(change);
    }
    let in_worktree = changes
        .iter()
        .find(|c| Path::new(&c.repo) == Path::new(&wt_workdir))
        .expect("워크트리 변화");
    assert!(in_worktree.head_changed);
    assert_eq!(in_worktree.head.as_deref(), Some(head(&worktree).as_str()));

    assert!(watchers.unwatch(wid));
    assert!(watchers.unwatch(id));
    assert!(!watchers.unwatch(id));
    assert_eq!(watchers.watching(), 0);
    commit(&repo, "감시를 끝낸 뒤");
    assert!(next(&rx, Duration::from_millis(600)).is_none());
}

/// 그래프 이름표는 다른 워크트리가 체크아웃한 것에 달라진다: 다른 워크트리의 HEAD 이동, 워크트리를 만들고 지우는
/// 것도 (참조 변화로) 알린다. 첫 워크트리를 만들기 전에는 `<공통>/worktrees`가 없다.
#[test]
fn other_worktrees_heads_are_reported() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch=main"]);
    commit(&repo, "첫 커밋");
    git(&repo, &["branch", "other"]);
    let (tx, rx) = mpsc::channel();
    let watchers = Watchers::new(move |change| {
        if let Change::Repo(change) = change {
            let _ = tx.send(change);
        }
    });
    let repos = RepoCache::new();
    let _ = watchers.watch(&repos, &repo).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    // 참조를 바꾸지 않는 워크트리(detached)를 만든다: 워크트리가 생긴 것을 안다.
    let wt = temp.path().join("wt");
    git(
        &repo,
        &["worktree", "add", "--quiet", wt.to_str().unwrap(), "main~0"],
    );
    let change = next(&rx, Duration::from_secs(10)).expect("워크트리가 생긴 것을 알린다");
    assert!(change.refs_changed && !change.head_changed);
    while next(&rx, Duration::from_millis(400)).is_some() {}
    std::thread::sleep(Duration::from_millis(300));

    // 그 워크트리에서 다른 브랜치로 바꾼다(커밋도 참조도 그대로).
    git(&wt, &["switch", "--quiet", "other"]);
    let change = next(&rx, Duration::from_secs(10)).expect("다른 워크트리의 체크아웃을 알린다");
    assert!(change.refs_changed && !change.head_changed);
    while next(&rx, Duration::from_millis(400)).is_some() {}

    // 다른 워크트리의 index 쓰기는 알리지 않는다.
    std::fs::write(wt.join("새 파일.txt"), "내용\n").unwrap();
    git(&wt, &["add", "새 파일.txt"]);
    assert!(
        next(&rx, Duration::from_millis(600)).is_none(),
        "다른 워크트리의 index 쓰기는 무시"
    );

    // 워크트리를 지운다.
    git(
        &repo,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
    let change = next(&rx, Duration::from_secs(10)).expect("워크트리가 없어진 것을 알린다");
    assert!(change.refs_changed);
}

#[test]
fn several_subscriptions_share_one_watch() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    let watchers = Watchers::new(|_| {});
    let repos = RepoCache::new();
    let (a, _) = watchers.watch(&repos, &repo).unwrap();
    let (b, _) = watchers.watch(&repos, &repo).unwrap();
    assert_ne!(a, b);
    assert_eq!(watchers.watching(), 1);
    watchers.unwatch(a);
    assert_eq!(watchers.watching(), 1);
    watchers.unwatch(b);
    assert_eq!(watchers.watching(), 0);
    assert!(
        watchers.watch(&repos, temp.path()).is_err(),
        "저장소가 아니다"
    );
}

#[test]
fn file_edits_are_reported() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("한글 폴더");
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join("파일 이름.rs");
    std::fs::write(&file, "하나\n").unwrap();
    let (tx, rx) = mpsc::channel::<FileChange>();
    let watchers = Watchers::new(move |change| {
        if let Change::File(change) = change {
            let _ = tx.send(change);
        }
    });
    let (id, path) = watchers.watch_file(&file).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    std::fs::write(&file, "하나\n둘\n").unwrap();
    let change = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("고친 파일을 알린다");
    assert_eq!(change.path, path);
    assert!(
        rx.recv_timeout(Duration::from_millis(400)).is_err(),
        "한 번만 알린다"
    );

    // 같은 폴더의 다른 파일은 알리지 않는다.
    std::fs::write(dir.join("다른 파일.txt"), "x").unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());

    // 편집기처럼 임시 파일에 쓰고 rename해도 알린다.
    let tmp = dir.join(".파일 이름.rs.tmp");
    std::fs::write(&tmp, "하나\n둘\n셋\n").unwrap();
    std::fs::rename(&tmp, &file).unwrap();
    assert!(rx.recv_timeout(Duration::from_secs(10)).is_ok(), "rename");

    std::fs::remove_file(&file).unwrap();
    assert!(rx.recv_timeout(Duration::from_secs(10)).is_ok(), "지움");

    assert!(watchers.unwatch(id));
    assert_eq!(watchers.watching(), 0);
    assert!(
        watchers
            .watch_file(&temp.path().join("없는 폴더/a.rs"))
            .is_err()
    );
}
