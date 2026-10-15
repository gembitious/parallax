//! Quick Open 파일 목록: 인덱스의 추적 파일이 `git ls-files`와 같고(작업트리에 없는 항목·서브모듈 제외, 충돌은
//! 한 번), 검색이 한글·초성·이모지·`:줄`을 다루며, 인덱스가 바뀌면(git add) 다시 읽는다.

mod common;

use std::path::Path;
use std::process::Command;

use common::{NULL_DEVICE, encoding, git, git_text, history};
use plx_core::files::{self, FileLists};
use plx_core::repo::Repo;

fn ls_files(repo: &Path, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["ls-files", "-z"];
    args.extend_from_slice(extra);
    let mut paths: Vec<String> = git(repo, &args)
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8(p.to_vec()).expect("UTF-8 경로"))
        .collect();
    // 충돌 단계(1~3)는 경로 하나로
    paths.dedup();
    paths
}

fn tracked(repo: &Path) -> Vec<String> {
    let opened = Repo::discover(repo).unwrap();
    files::tracked(&opened)
        .unwrap()
        .paths()
        .map(str::to_owned)
        .collect()
}

fn clone(source: &Path, target: &Path) {
    git(
        target.parent().unwrap(),
        &[
            "clone",
            "--quiet",
            source.to_str().unwrap(),
            target.to_str().unwrap(),
        ],
    );
}

fn git_as_user(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=plx",
            "-c",
            "user.email=plx@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git 실행")
}

#[test]
fn tracked_files_match_git_ls_files() {
    for repo in [history(), encoding()] {
        assert_eq!(
            tracked(&repo),
            ls_files(&repo, &[]),
            "{}",
            repo.display()
        );
    }
}

#[test]
fn leaves_out_entries_outside_the_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("저장소");
    clone(&history(), &repo);
    // sparse checkout이 쓰는 skip-worktree 표시와 서브모듈(gitlink)
    git(&repo, &["update-index", "--skip-worktree", "src/lib.rs"]);
    let head = git_text(&repo, &["rev-parse", "HEAD"]);
    git(
        &repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},vendor/sub"),
        ],
    );
    let ours = tracked(&repo);
    assert!(!ours.iter().any(|p| p == "src/lib.rs"), "{ours:?}");
    assert!(!ours.iter().any(|p| p == "vendor/sub"), "{ours:?}");
    let mut expected = ls_files(&repo, &[]);
    expected.retain(|p| p != "src/lib.rs" && p != "vendor/sub");
    assert_eq!(ours, expected);
}

#[test]
fn conflicted_paths_appear_once() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(repo.join("a.txt"), "base\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    assert!(git_as_user(&repo, &["commit", "--quiet", "-m", "base"]).status.success());
    git(&repo, &["checkout", "--quiet", "-b", "other"]);
    std::fs::write(repo.join("a.txt"), "other\n").unwrap();
    assert!(git_as_user(&repo, &["commit", "--quiet", "-am", "other"]).status.success());
    git(&repo, &["checkout", "--quiet", "main"]);
    std::fs::write(repo.join("a.txt"), "main\n").unwrap();
    assert!(git_as_user(&repo, &["commit", "--quiet", "-am", "main"]).status.success());
    let merge = git_as_user(&repo, &["merge", "--quiet", "other"]);
    assert!(!merge.status.success(), "충돌이 나야 한다");
    assert!(git_text(&repo, &["ls-files", "--stage"]).lines().count() >= 3);
    assert_eq!(tracked(&repo), ["a.txt"]);
}

#[test]
fn empty_repository_has_no_files() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "--quiet"]);
    let repo = Repo::discover(dir.path()).unwrap();
    let found = FileLists::new().search(&repo, "a", 10).unwrap();
    assert_eq!((found.files, found.matched), (0, 0));
}

#[test]
fn finds_korean_paths_by_syllables_and_initials() {
    let repo = Repo::discover(&encoding()).unwrap();
    let lists = FileLists::new();
    let top = |q: &str| {
        lists
            .search(&repo, q, 5)
            .unwrap()
            .items
            .first()
            .map(|item| item.path.clone())
    };
    assert_eq!(top("파일 이름").as_deref(), Some("한글/파일 이름.txt"));
    assert_eq!(top("ㅍㅇ ㅇㄹ").as_deref(), Some("한글/파일 이름.txt"));
    assert_eq!(top("ㅅㅁ").as_deref(), Some("문서/설명.md"));
    // 조합 중인 음절(`설명`을 치다가 `서`·`설`·`설며`)
    for typing in ["서", "설", "설며"] {
        assert_eq!(top(typing).as_deref(), Some("문서/설명.md"), "{typing}");
    }
    // 이모지는 UTF-16 두 단위지만 위치는 코드 포인트 단위다
    let found = lists.search(&repo, "😀", 5).unwrap();
    assert_eq!(found.items[0].path, "이모지-😀.txt");
    assert_eq!(found.items[0].positions, [4]);
}

#[test]
fn line_suffix_and_pasted_paths() {
    let repo = Repo::discover(&encoding()).unwrap();
    let lists = FileLists::new();
    let found = lists.search(&repo, "설명.md:3", 5).unwrap();
    assert_eq!(found.line, Some(3));
    assert_eq!(found.items[0].path, "문서/설명.md");
    assert_eq!(found.repo, repo.workdir().display().to_string());
    // 작업트리 절대 경로를 붙여 넣어도 된다
    let pasted = format!("{}/문서/설명.md:7", repo.workdir().display());
    let found = lists.search(&repo, &pasted, 5).unwrap();
    assert_eq!(found.line, Some(7));
    assert_eq!(found.items[0].path, "문서/설명.md");
}

#[test]
fn recent_files_come_first_for_an_empty_query() {
    let repo = Repo::discover(&history()).unwrap();
    let lists = FileLists::new();
    let first = |lists: &FileLists| -> Vec<String> {
        lists
            .search(&repo, "", 3)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.path)
            .collect()
    };
    let all = ls_files(&history(), &[]);
    assert_eq!(first(&lists), all[..3]);
    lists.touch(&repo, "src/lib.rs");
    lists.touch(&repo, "core/util.rs");
    lists.touch(&repo, "src/lib.rs");
    lists.touch(&repo, "지운/파일.rs");
    assert_eq!(first(&lists), ["src/lib.rs", "core/util.rs", &all[0]]);
}

#[test]
fn reloads_when_the_index_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo_dir = dir.path().join("repo");
    clone(&history(), &repo_dir);
    let repo = Repo::discover(&repo_dir).unwrap();
    let lists = FileLists::new();
    let found = lists.search(&repo, "새로운", 10).unwrap();
    assert!(found.load_ms.is_some(), "처음은 읽는다");
    assert!(found.items.is_empty());
    let again = lists.search(&repo, "새로운", 10).unwrap();
    assert!(again.load_ms.is_none(), "인덱스가 그대로면 다시 읽지 않는다");

    std::fs::write(repo_dir.join("새로운 파일.txt"), "내용\n").unwrap();
    git(&repo_dir, &["add", "-A"]);
    let found = lists.search(&repo, "새로운", 10).unwrap();
    assert!(found.load_ms.is_some(), "git add 뒤에는 다시 읽는다");
    assert_eq!(found.items[0].path, "새로운 파일.txt");
    assert_eq!(found.files, ls_files(&repo_dir, &[]).len());
}

#[test]
fn typing_narrows_within_previous_results() {
    let repo = Repo::discover(&history()).unwrap();
    let lists = FileLists::new();
    let mut typed = String::new();
    for c in "srclib".chars() {
        typed.push(c);
        let narrowed = lists.search(&repo, &typed, 100).unwrap();
        let fresh = FileLists::new().search(&repo, &typed, 100).unwrap();
        assert_eq!(narrowed.items, fresh.items, "{typed}");
        assert_eq!(narrowed.matched, fresh.matched, "{typed}");
        assert_eq!(narrowed.narrowed, typed.len() > 1, "{typed}");
    }
    assert_eq!(
        lists.search(&repo, "srclib", 1).unwrap().items[0].path,
        "src/lib.rs"
    );
}
