//! 브랜치·태그 목록을 `git for-each-ref`와, 앞섬·뒤처짐을 `git rev-list --left-right --count`와 견준다.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{commit_all, commit_at, git, git_text, scratch_repo};
use plx_core::refs::{self, Kind};
use plx_core::repo::Repo;

/// 원격을 흉내 낸 저장소(bare)를 만들어 `origin`으로 붙인 저장소
fn with_remote(name: &str) -> std::path::PathBuf {
    let remote = scratch_repo(&format!("{name}-remote"));
    std::fs::write(remote.join("a.txt"), "1\n").unwrap();
    commit_at(&remote, "원격 첫 커밋", 1_700_000_000);
    std::fs::write(remote.join("a.txt"), "2\n").unwrap();
    commit_at(&remote, "원격 둘째", 1_700_000_100);
    git(&remote, &["branch", "old"]);
    git(&remote, &["branch", "feature"]);

    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("scratch-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    let parent = dir.parent().unwrap();
    git(
        parent,
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            dir.to_str().unwrap(),
        ],
    );
    git(&dir, &["config", "user.name", "T"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    dir
}

/// git이 본 (전체 이름 → (짧은 이름, 커밋)). 기호 참조(`origin/HEAD`)는 뺀다. 커밋은 `^{}`로 끝까지 벗긴다(태그의 태그).
fn git_refs(dir: &Path) -> BTreeMap<String, (String, String)> {
    let text = git_text(
        dir,
        &[
            "for-each-ref",
            "--format=%(symref)%09%(refname)%09%(refname:lstrip=2)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    );
    let names: Vec<(String, String)> = text
        .lines()
        .filter(|line| line.starts_with('\t'))
        .map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            (f[1].to_owned(), f[2].to_owned())
        })
        .collect();
    let mut args = vec!["rev-parse".to_owned()];
    args.extend(names.iter().map(|(full, _)| format!("{full}^{{}}")));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let shas = git_text(dir, &args);
    names
        .into_iter()
        .zip(shas.lines())
        .map(|((full, short), sha)| (full, (short, sha.to_owned())))
        .collect()
}

#[test]
fn list_matches_git() {
    let dir = with_remote("refs-list");
    git(&dir, &["checkout", "-q", "-b", "feature", "origin/feature"]);
    std::fs::write(dir.join("b.txt"), "b\n").unwrap();
    commit_at(&dir, "기능 커밋", 1_700_000_500);
    git(&dir, &["tag", "light"]);
    git(&dir, &["tag", "-a", "v1.0", "-m", "첫 릴리스", "HEAD~1"]);
    // 태그의 태그도 커밋까지 벗긴다
    git(&dir, &["tag", "-a", "v1.0-again", "-m", "다시", "v1.0"]);
    git(&dir, &["branch", "새-브랜치", "main"]);

    let repo = Repo::discover(&dir).unwrap();
    let list = refs::list(&repo).unwrap();
    let ours: BTreeMap<String, (String, String)> = list
        .refs
        .iter()
        .map(|r| (r.full.clone(), (r.name.clone(), r.sha.clone())))
        .collect();
    assert_eq!(ours, git_refs(&dir));
    assert!(!list.truncated);
    assert!(!list.detached);
    assert_eq!(
        list.head.as_deref(),
        Some(git_text(&dir, &["rev-parse", "HEAD"]).as_str())
    );

    // 지금 브랜치가 맨 위, 그다음 로컬 브랜치는 최근 커밋부터, 원격, 태그 순
    let kinds: Vec<Kind> = list.refs.iter().map(|r| r.kind).collect();
    let mut sorted = kinds.clone();
    sorted.sort_by_key(|k| match k {
        Kind::Branch => 0,
        Kind::Remote => 1,
        Kind::Tag => 2,
    });
    assert_eq!(kinds, sorted);
    assert_eq!(list.refs[0].name, "feature");
    assert!(list.refs[0].current);
    assert_eq!(list.refs.iter().filter(|r| r.current).count(), 1);
    assert_eq!(list.refs[0].summary.as_deref(), Some("기능 커밋"));
    assert_eq!(list.refs[0].time, 1_700_000_500);
    assert_eq!(list.refs[0].worktrees.len(), 1);
    assert!(list.refs[0].worktrees[0].current);

    let tag = |name: &str| list.refs.iter().find(|r| r.name == name).unwrap();
    assert!(tag("v1.0").annotated);
    assert!(tag("v1.0-again").annotated);
    assert!(!tag("light").annotated);
    assert_eq!(tag("v1.0").summary.as_deref(), Some("원격 둘째"));
    assert!(list.refs.iter().all(|r| r.exists));
    assert!(list.refs.iter().all(|r| r.name != "origin/HEAD"));
}

#[test]
fn tracking_matches_rev_list() {
    let dir = with_remote("refs-tracking");
    // main: 앞섬 2, 뒤처짐 1 (원격에 새 커밋, 로컬에 두 커밋)
    let remote = dir.with_file_name("scratch-refs-tracking-remote");
    std::fs::write(remote.join("r.txt"), "r\n").unwrap();
    commit_all(&remote, "원격 새 커밋");
    git(&dir, &["fetch", "-q"]);
    std::fs::write(dir.join("l.txt"), "1\n").unwrap();
    commit_all(&dir, "로컬 하나");
    std::fs::write(dir.join("l.txt"), "2\n").unwrap();
    commit_all(&dir, "로컬 둘");
    // old: upstream을 원격에서 지웠다(gone)
    git(&dir, &["branch", "-q", "--track", "old", "origin/old"]);
    git(&remote, &["branch", "-D", "old"]);
    git(&dir, &["fetch", "-q", "--prune"]);
    // feature: upstream과 같다, solo: upstream 없음
    git(
        &dir,
        &["branch", "-q", "--track", "feature", "origin/feature"],
    );
    git(&dir, &["branch", "-q", "--no-track", "solo"]);

    let repo = Repo::discover(&dir).unwrap();
    let tracking = refs::tracking(&repo).unwrap();
    let by: BTreeMap<&str, &refs::Tracking> =
        tracking.iter().map(|t| (t.full.as_str(), t)).collect();
    assert!(!by.contains_key("refs/heads/solo"));

    for (branch, upstream) in [("main", "origin/main"), ("feature", "origin/feature")] {
        let t = by[format!("refs/heads/{branch}").as_str()];
        assert_eq!(t.upstream, format!("refs/remotes/{upstream}"));
        let counts = git_text(
            &dir,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{branch}...{upstream}"),
            ],
        );
        let mut n = counts.split_whitespace().map(|n| n.parse::<u64>().unwrap());
        assert_eq!((t.ahead, t.behind), (n.next().unwrap(), n.next().unwrap()));
        assert!(!t.gone);
    }
    assert_eq!(
        (by["refs/heads/main"].ahead, by["refs/heads/main"].behind),
        (2, 1)
    );
    let old = by["refs/heads/old"];
    assert!(old.gone);
    assert_eq!(old.upstream, "refs/remotes/origin/old");
}

#[test]
fn detached_head_and_other_worktree() {
    let dir = scratch_repo("refs-worktree");
    std::fs::write(dir.join("a.txt"), "1\n").unwrap();
    let first = commit_all(&dir, "하나");
    let other = dir.with_file_name("scratch-refs-worktree-other");
    let _ = std::fs::remove_dir_all(&other);
    git(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            other.to_str().unwrap(),
        ],
    );
    std::fs::write(dir.join("a.txt"), "2\n").unwrap();
    commit_all(&dir, "둘");
    git(&dir, &["checkout", "-q", "--detach", &first]);

    let repo = Repo::discover(&dir).unwrap();
    let list = refs::list(&repo).unwrap();
    assert!(list.detached);
    assert_eq!(list.head.as_deref(), Some(first.as_str()));
    assert!(list.refs.iter().all(|r| !r.current));
    let side = list.refs.iter().find(|r| r.name == "side").unwrap();
    assert_eq!(side.worktrees.len(), 1);
    assert!(!side.worktrees[0].current);
    assert_eq!(side.worktrees[0].name, "scratch-refs-worktree-other");
    let main = list.refs.iter().find(|r| r.name == "main").unwrap();
    assert!(main.worktrees.is_empty());
}
