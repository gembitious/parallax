//! 두 지점 비교(작업트리 변경, 리비전 비교)를 git 결과와 대조한다.
//! 작업트리 비교는 비교한 뒤 `git add -A`로 모두 올리고 `git diff --cached -M <기준>`과 견준다(추적하지 않는 파일을
//! 더한 `git diff <기준>`과 같다).

mod common;

use std::path::Path;

use common::{commit_all, git, git_text, scratch_repo};
use plx_core::changes::{self, Comparison, Side};
use plx_core::commit::{self, FileStatus};
use plx_core::repo::Repo;

/// (상태 글자, 경로, 원래 경로, 더한·뺀 줄. 바이너리는 None)
type Row = (char, String, Option<String>, Option<(u32, u32)>);

fn git_rows(dir: &Path, args: &[&str]) -> Vec<Row> {
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).expect("UTF-8 경로");
    let run = |what: &str| {
        let mut all = vec!["diff", what, "-M", "-z"];
        all.extend_from_slice(args);
        git(dir, &all)
    };
    let out = run("--name-status");
    let mut tokens = out.split(|&b| b == 0).filter(|t| !t.is_empty());
    let mut rows = Vec::new();
    while let Some(status) = tokens.next() {
        let letter = status[0] as char;
        let first = text(tokens.next().unwrap());
        let (path, old) = if matches!(letter, 'R' | 'C') {
            (text(tokens.next().unwrap()), Some(first))
        } else {
            (first, None)
        };
        rows.push((letter, path, old, None));
    }
    let out = run("--numstat");
    let mut tokens = out.split(|&b| b == 0);
    while let Some(head) = tokens.next() {
        if head.is_empty() {
            continue;
        }
        let head = text(head);
        let mut fields = head.splitn(3, '\t');
        let (add, del, rest) = (
            fields.next().unwrap(),
            fields.next().unwrap(),
            fields.next().unwrap(),
        );
        let path = if rest.is_empty() {
            tokens.next();
            text(tokens.next().unwrap())
        } else {
            rest.to_owned()
        };
        let stat = match (add.parse(), del.parse()) {
            (Ok(a), Ok(d)) => Some((a, d)),
            _ => None,
        };
        rows.iter_mut().find(|r| r.1 == path).unwrap().3 = stat;
    }
    rows.sort_by(|a, b| a.1.cmp(&b.1));
    rows
}

fn our_rows(repo: &Repo, comparison: &Comparison) -> Vec<Row> {
    let mut rows: Vec<Row> = comparison
        .files
        .iter()
        .map(|f| {
            let stat = match f.stat {
                Some(stat) => stat,
                None => commit::file_stats(repo, &[request(f)]).unwrap()[0],
            };
            (
                f.status.letter(),
                f.path.clone(),
                f.old_path.clone(),
                (!stat.binary).then_some((stat.additions, stat.deletions)),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.1.cmp(&b.1));
    rows
}

fn request(f: &commit::FileChange) -> commit::StatRequest {
    commit::StatRequest {
        path: f.path.clone(),
        old_path: f.old_path.clone(),
        old_id: f.old_id.clone(),
        new_id: f.new_id.clone(),
        submodule: f.submodule,
        worktree: f.worktree,
    }
}

fn head(dir: &Path) -> gix::ObjectId {
    gix::ObjectId::from_hex(git_text(dir, &["rev-parse", "HEAD"]).as_bytes()).unwrap()
}

fn worktree(dir: &Path) -> (Repo, Comparison) {
    let repo = Repo::discover(dir).unwrap();
    let index = std::fs::read(dir.join(".git/index")).unwrap();
    let comparison = changes::compare(&repo, Some(head(dir)), Side::Worktree, false).unwrap();
    // 읽기 전용: 인덱스(stat)를 고쳐 쓰지 않는다.
    assert_eq!(std::fs::read(dir.join(".git/index")).unwrap(), index);
    (repo, comparison)
}

/// 비교한 뒤 모두 올려 git이 보는 것과 견준다.
fn assert_like_git(dir: &Path) -> Comparison {
    let (repo, comparison) = worktree(dir);
    let ours = our_rows(&repo, &comparison);
    git(dir, &["add", "-A"]);
    let theirs = git_rows(dir, &["--cached", "HEAD"]);
    assert_eq!(ours, theirs);
    comparison
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn base_repo(name: &str) -> std::path::PathBuf {
    let dir = scratch_repo(name);
    write(&dir, "a.txt", &common::ten_lines(&[]));
    write(&dir, "b.txt", "b\n");
    write(&dir, "dir/c.txt", "one\ntwo\nthree\n");
    write(&dir, "moved.txt", &"moving line of text\n".repeat(20));
    write(&dir, "한글 경로/파일.txt", "가\n나\n");
    write(&dir, ".gitignore", "*.log\n");
    commit_all(&dir, "base");
    dir
}

#[test]
fn clean_worktree_has_no_changes() {
    let dir = base_repo("changes-clean");
    let (_, comparison) = worktree(&dir);
    assert!(comparison.files.is_empty(), "{:?}", comparison.files);
    assert!(comparison.target.is_none());
    assert_eq!(comparison.base.unwrap().summary, "base");
}

#[test]
fn worktree_changes_match_git() {
    let dir = base_repo("changes-worktree");
    write(&dir, "a.txt", &common::ten_lines(&[3, 7]));
    std::fs::remove_file(dir.join("b.txt")).unwrap();
    write(&dir, "new/untracked.txt", "fresh\nfile\n");
    write(&dir, "ignored.log", "nope\n");
    // 올리지 않은 rename(옮기고 조금 고침)
    std::fs::remove_file(dir.join("moved.txt")).unwrap();
    write(
        &dir,
        "elsewhere/moved.txt",
        &format!("{}changed\n", "moving line of text\n".repeat(19)),
    );
    write(&dir, "한글 경로/파일.txt", "가\n다\n");
    // 올렸다가 되돌린 파일은 바뀌지 않은 것이다.
    write(&dir, "dir/c.txt", "staged\n");
    git(&dir, &["add", "dir/c.txt"]);
    write(&dir, "dir/c.txt", "one\ntwo\nthree\n");
    let comparison = assert_like_git(&dir);
    assert!(comparison.files.iter().all(|f| f.path != "ignored.log"));
    let moved = comparison
        .files
        .iter()
        .find(|f| f.path == "elsewhere/moved.txt")
        .unwrap();
    assert_eq!(moved.status, FileStatus::Renamed);
    assert!(moved.worktree);
    let deleted = comparison.files.iter().find(|f| f.path == "b.txt").unwrap();
    assert!(!deleted.worktree && deleted.new_id.is_none());
}

#[test]
fn staged_and_worktree_changes_combine() {
    let dir = base_repo("changes-staged");
    // 올린 새 파일, 올린 뒤 더 고친 파일, git mv
    write(&dir, "staged.txt", "s\n");
    write(&dir, "a.txt", &common::ten_lines(&[1]));
    git(&dir, &["add", "staged.txt", "a.txt"]);
    write(&dir, "a.txt", &common::ten_lines(&[1, 2]));
    git(&dir, &["mv", "dir/c.txt", "dir/d.txt"]);
    assert_like_git(&dir);
}

#[cfg(unix)]
#[test]
fn mode_and_symlink_changes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = base_repo("changes-mode");
    let path = dir.join("b.txt");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("a.txt", dir.join("link")).unwrap();
    let comparison = assert_like_git(&dir);
    let b = comparison.files.iter().find(|f| f.path == "b.txt").unwrap();
    assert_eq!(b.new_mode.as_deref(), Some("100755"));
    let link = comparison.files.iter().find(|f| f.path == "link").unwrap();
    assert_eq!(link.new_mode.as_deref(), Some("120000"));
}

#[test]
fn crlf_worktree_is_compared_after_clean() {
    let dir = base_repo("changes-crlf");
    git(&dir, &["config", "core.autocrlf", "true"]);
    // 줄 끝만 CRLF로 바꾼 파일은 git에 넣으면 같다.
    write(&dir, "b.txt", "b\r\n");
    write(&dir, "dir/c.txt", "one\r\nTWO\r\nthree\r\n");
    let comparison = assert_like_git(&dir);
    assert_eq!(comparison.files.len(), 1);
}

#[test]
fn versions_and_stats_read_the_worktree() {
    let dir = base_repo("changes-versions");
    write(&dir, "a.txt", &common::ten_lines(&[5]));
    let (repo, comparison) = worktree(&dir);
    let file = &comparison.files[0];
    assert!(file.worktree);
    let versions = commit::file_versions(
        &repo,
        file.old_id.as_deref(),
        file.new_id.as_deref(),
        Some(&file.path),
        false,
        true,
    )
    .unwrap();
    assert_eq!(versions.new.unwrap().text, common::ten_lines(&[5]));
    assert_eq!(versions.old.unwrap().text, common::ten_lines(&[]));
    let stats = commit::file_stats(&repo, &[request(file)]).unwrap();
    assert_eq!((stats[0].additions, stats[0].deletions), (1, 1));
}

#[test]
fn untracked_files_are_limited() {
    let dir = base_repo("changes-untracked");
    for i in 0..changes::UNTRACKED_LIMIT + 5 {
        write(&dir, &format!("many/{i:05}.txt"), "x\n");
    }
    let (_, comparison) = worktree(&dir);
    assert_eq!(comparison.files.len(), changes::UNTRACKED_LIMIT);
    assert_eq!(comparison.untracked_omitted, 5);
    assert_eq!(comparison.files[0].path, "many/00000.txt");
}

#[test]
fn repository_without_commits_compares_with_empty_tree() {
    let dir = scratch_repo("changes-unborn");
    write(&dir, "first.txt", "hello\n");
    let repo = Repo::discover(&dir).unwrap();
    let comparison = changes::compare(&repo, None, Side::Worktree, false).unwrap();
    assert!(comparison.base.is_none());
    assert_eq!(comparison.files.len(), 1);
    assert_eq!(comparison.files[0].status, FileStatus::Added);
}

#[test]
fn revisions_compare_like_git_diff() {
    let dir = base_repo("changes-revs");
    let base = git_text(&dir, &["rev-parse", "HEAD"]);
    git(&dir, &["checkout", "-q", "-b", "topic"]);
    write(&dir, "a.txt", &common::ten_lines(&[2]));
    git(&dir, &["mv", "moved.txt", "dir/moved.txt"]);
    let topic = commit_all(&dir, "topic");
    git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "b.txt", "main moved on\n");
    let main = commit_all(&dir, "main");
    let repo = Repo::discover(&dir).unwrap();
    let id = |s: &str| gix::ObjectId::from_hex(s.as_bytes()).unwrap();

    // 두 점: main ↔ topic (main의 b.txt 변경도 보인다)
    let two = changes::compare(&repo, Some(id(&main)), Side::Commit(id(&topic)), false).unwrap();
    assert_eq!(our_rows(&repo, &two), git_rows(&dir, &[&main, &topic]));
    assert!(two.files.iter().all(|f| !f.worktree));

    // 세 점: 갈라진 지점 ↔ topic
    let three = changes::compare(&repo, Some(id(&main)), Side::Commit(id(&topic)), true).unwrap();
    assert_eq!(three.base.as_ref().unwrap().sha, base);
    assert!(three.merge_base);
    assert_eq!(
        our_rows(&repo, &three),
        git_rows(&dir, &[&format!("{main}...{topic}")])
    );
}

#[test]
fn ref_names_list_branches_remotes_and_tags() {
    let dir = base_repo("changes-refs");
    let head = git_text(&dir, &["rev-parse", "HEAD"]);
    git(&dir, &["branch", "topic"]);
    git(&dir, &["branch", "기능/한글"]);
    git(&dir, &["tag", "v1.0"]);
    git(&dir, &["tag", "-a", "-m", "annotated", "v2.0"]);
    git(&dir, &["update-ref", "refs/remotes/origin/main", &head]);
    git(
        &dir,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    git(&dir, &["checkout", "-q", "topic"]);
    let repo = Repo::discover(&dir).unwrap();
    let names: Vec<(String, changes::RefKind, bool)> = changes::ref_names(&repo)
        .unwrap()
        .into_iter()
        .map(|r| (r.name, r.kind, r.current))
        .collect();
    use changes::RefKind::{Branch, Remote, Tag};
    assert_eq!(
        names,
        vec![
            ("topic".to_owned(), Branch, true),
            ("main".to_owned(), Branch, false),
            ("기능/한글".to_owned(), Branch, false),
            ("origin/main".to_owned(), Remote, false),
            ("v1.0".to_owned(), Tag, false),
            ("v2.0".to_owned(), Tag, false),
        ]
    );
}
