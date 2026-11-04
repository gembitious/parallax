//! reflog·stash 목록을 `git log -g`·`git stash list`와 견준다.

mod common;

use std::path::Path;

use common::{commit_all, git, git_text, scratch_repo};
use plx_core::reflog::{self, Which};
use plx_core::repo::Repo;

/// (sha, 메시지 전체, 이름, 시각) 최근 것부터
fn git_rows(dir: &Path, args: &[&str]) -> Vec<(String, String, String, i64)> {
    let mut all = args.to_vec();
    all.push("--format=%H%x1f%gs%x1f%gn%x1f%gd");
    all.push("--date=unix");
    let text = git_text(dir, &all);
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\x1f').collect();
            // %gd(--date=unix) = HEAD@{<시각>}
            let time = f[3]
                .rsplit_once('{')
                .and_then(|(_, t)| t.strip_suffix('}'))
                .and_then(|t| t.parse().ok())
                .expect("reflog 시각");
            (f[0].to_owned(), f[1].to_owned(), f[2].to_owned(), time)
        })
        .collect()
}

fn ours(dir: &Path, which: Which) -> Vec<(String, String, String, i64)> {
    let repo = Repo::discover(dir).unwrap();
    reflog::read(&repo, which)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| {
            let message = match e.action {
                Some(action) => format!("{action}: {}", e.message),
                None => e.message,
            };
            (e.sha, message, e.who.name, e.who.time)
        })
        .collect()
}

#[test]
fn head_and_stash_match_git() {
    let dir = scratch_repo("reflog");
    std::fs::write(dir.join("a.txt"), "1\n").unwrap();
    commit_all(&dir, "첫 커밋");
    std::fs::write(dir.join("a.txt"), "2\n").unwrap();
    commit_all(&dir, "fix: 둘째");
    git(&dir, &["commit", "-q", "--amend", "-m", "fix: 고친 둘째"]);
    git(&dir, &["checkout", "-q", "-b", "dev"]);
    std::fs::write(dir.join("b.txt"), "b\n").unwrap();
    commit_all(&dir, "dev 커밋");
    git(&dir, &["checkout", "-q", "main"]);
    git(&dir, &["reset", "-q", "--hard", "HEAD~1"]);
    git(&dir, &["merge", "-q", "--ff-only", "dev"]);

    // stash: 고친 것만 하나, 추적하지 않는 파일까지(-u) 하나
    std::fs::write(dir.join("a.txt"), "고침\n").unwrap();
    git(&dir, &["stash", "push", "-q", "-m", "고친 것"]);
    std::fs::write(dir.join("a.txt"), "또\n").unwrap();
    std::fs::write(dir.join("new.txt"), "새 파일\n").unwrap();
    git(&dir, &["stash", "push", "-q", "-u"]);

    let head = ours(&dir, Which::Head);
    assert_eq!(head, git_rows(&dir, &["log", "-g", "HEAD"]));
    assert!(head.len() >= 7, "{head:?}");
    let stash = ours(&dir, Which::Stash);
    assert_eq!(stash, git_rows(&dir, &["stash", "list"]));
    assert_eq!(stash.len(), 2);

    let repo = Repo::discover(&dir).unwrap();
    let list = reflog::read(&repo, Which::Stash).unwrap();
    assert_eq!(list.entries[0].index, 0);
    assert_eq!(list.entries[1].index, 1);
    // 최근 stash(-u)에는 추적하지 않던 파일을 담은 셋째 부모가 있다.
    let untracked = list.entries[0].untracked.clone().expect("셋째 부모");
    assert_eq!(untracked, git_text(&dir, &["rev-parse", "stash@{0}^3"]));
    assert_eq!(list.entries[1].untracked, None);
    assert!(list.entries.iter().all(|e| e.exists && e.summary.is_some()));

    let head = reflog::read(&repo, Which::Head).unwrap();
    let amend = head
        .entries
        .iter()
        .find(|e| e.action.as_deref() == Some("commit (amend)"))
        .expect("amend 항목");
    assert_eq!(amend.message, "fix: 고친 둘째");
    let first = head.entries.last().unwrap();
    assert_eq!(first.action.as_deref(), Some("commit (initial)"));
    assert_eq!(first.previous, None);
}

#[test]
fn no_stash_is_empty() {
    let dir = scratch_repo("reflog-empty");
    std::fs::write(dir.join("a.txt"), "1\n").unwrap();
    commit_all(&dir, "하나");
    let repo = Repo::discover(&dir).unwrap();
    let list = reflog::read(&repo, Which::Stash).unwrap();
    assert!(list.entries.is_empty());
    assert!(!list.truncated);
}

#[test]
fn worktree_has_its_own_head_reflog() {
    let dir = scratch_repo("reflog-worktree");
    std::fs::write(dir.join("a.txt"), "1\n").unwrap();
    commit_all(&dir, "하나");
    let other = dir.with_file_name("scratch-reflog-worktree-other");
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
    std::fs::write(other.join("b.txt"), "b\n").unwrap();
    commit_all(&other, "옆 워크트리");

    assert_eq!(
        ours(&other, Which::Head),
        git_rows(&other, &["log", "-g", "HEAD"])
    );
    assert_eq!(
        ours(&dir, Which::Head),
        git_rows(&dir, &["log", "-g", "HEAD"])
    );
    let main = ours(&dir, Which::Head);
    assert!(main.iter().all(|(_, m, _, _)| !m.contains("옆 워크트리")));
}

#[test]
fn missing_commit_and_broken_line() {
    let dir = scratch_repo("reflog-missing");
    std::fs::write(dir.join("a.txt"), "1\n").unwrap();
    let sha = commit_all(&dir, "하나");
    let log = dir.join(".git/logs/HEAD");
    let mut text = std::fs::read_to_string(&log).unwrap();
    // 사라진 커밋(gc가 지운 것)과 읽을 수 없는 줄
    text.push_str("이 줄은 깨졌다\n");
    text.push_str(&format!(
        "{sha} {} T <t@example.com> 1700000000 +0900\treset: moving to 없는 커밋\n",
        "1".repeat(40)
    ));
    std::fs::write(&log, text).unwrap();

    let repo = Repo::discover(&dir).unwrap();
    let list = reflog::read(&repo, Which::Head).unwrap();
    let missing = &list.entries[0];
    assert_eq!(missing.index, 0);
    assert!(!missing.exists);
    assert_eq!(missing.summary, None);
    assert_eq!(missing.action.as_deref(), Some("reset"));
    assert_eq!(missing.who.tz_offset, 9 * 3600);
    // 깨진 줄은 번호를 차지하지 않는다(git: 이 저장소의 `git rev-parse HEAD@{1}`은 첫 커밋이다).
    assert_eq!(list.entries[1].index, 1);
    assert_eq!(git_text(&dir, &["rev-parse", "HEAD@{1}"]), sha);
    assert_eq!(list.entries[1].sha, sha);
}
