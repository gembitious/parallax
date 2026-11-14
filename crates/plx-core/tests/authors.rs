//! 작성자 보기(M18)를 `git shortlog -sne --no-merges`와 견준다(mailmap, 경로, 모든 참조, 한글 이름).

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{encoding, git, git_at, git_text, history, scratch_repo};
use plx_core::authors::{self, Scope};
use plx_core::cache::DiskCache;
use plx_core::repo::Repo;

/// git이 센 (`이름 <메일>` → 커밋 수)
fn shortlog(dir: &Path, revs: &[&str], path: Option<&str>) -> BTreeMap<String, u32> {
    let mut args = vec!["shortlog", "-sne", "--no-merges"];
    args.extend_from_slice(revs);
    args.push("--");
    args.extend(path);
    git_text(dir, &args)
        .lines()
        .map(|line| {
            let (count, who) = line.trim().split_once('\t').unwrap();
            (who.to_owned(), count.trim().parse().unwrap())
        })
        .collect()
}

fn ours(list: &authors::Authors) -> BTreeMap<String, u32> {
    list.authors
        .iter()
        .map(|a| (format!("{} <{}>", a.name, a.email), a.commits))
        .collect()
}

fn commit_as(dir: &Path, who: (&str, &str), message: &str, t: i64) {
    git(dir, &["add", "-A"]);
    git_at(
        dir,
        &[
            "-c",
            &format!("user.name={}", who.0),
            "-c",
            &format!("user.email={}", who.1),
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            message,
        ],
        t,
    );
}

const KIM: (&str, &str) = ("김철수", "kim@example.com");
const KIM_OLD: (&str, &str) = ("kim", "KIM@old.example.com");
const LEE: (&str, &str) = ("Lee", "lee@example.com");
const PARK: (&str, &str) = ("Park", "park@example.com");

/// main: KIM ×2(하나는 dir/), KIM_OLD(mailmap으로 KIM), LEE(dir/), PARK의 side를 LEE가 merge. side에만 PARK 하나 더.
fn people_repo(name: &str) -> std::path::PathBuf {
    let dir = scratch_repo(name);
    std::fs::write(dir.join("a.txt"), "1\n").unwrap();
    commit_as(&dir, KIM, "첫 커밋", 1_700_000_000);
    std::fs::create_dir_all(dir.join("dir")).unwrap();
    std::fs::write(dir.join("dir/b.txt"), "1\n").unwrap();
    commit_as(&dir, KIM, "dir", 1_700_000_100);
    std::fs::write(dir.join("a.txt"), "2\n").unwrap();
    commit_as(&dir, KIM_OLD, "옛 메일", 1_700_000_200);
    std::fs::write(dir.join("dir/b.txt"), "2\n").unwrap();
    commit_as(&dir, LEE, "lee", 1_700_000_300);
    git(&dir, &["checkout", "-q", "-b", "side"]);
    std::fs::write(dir.join("c.txt"), "1\n").unwrap();
    commit_as(&dir, PARK, "park", 1_700_000_400);
    git(&dir, &["checkout", "-q", "main"]);
    git_at(
        &dir,
        &[
            "-c",
            "user.name=Lee",
            "-c",
            "user.email=lee@example.com",
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "merge side",
            "side",
        ],
        1_700_000_500,
    );
    git(&dir, &["checkout", "-q", "side"]);
    std::fs::write(dir.join("c.txt"), "2\n").unwrap();
    commit_as(&dir, PARK, "park 2", 1_700_000_600);
    git(&dir, &["checkout", "-q", "main"]);
    std::fs::write(
        dir.join(".mailmap"),
        "김철수 <kim@example.com> <KIM@old.example.com>\n",
    )
    .unwrap();
    commit_as(&dir, KIM, "mailmap", 1_700_000_700);
    dir
}

#[test]
fn counts_like_git_shortlog() {
    let dir = people_repo("authors-shortlog");
    let repo = Repo::discover(&dir).unwrap();

    let head = authors::list(&repo, Scope::Head, None, None).unwrap();
    assert_eq!(ours(&head), shortlog(&dir, &["HEAD"], None));
    // mailmap으로 옛 메일이 합쳐지고 merge는 세지 않는다
    assert_eq!(head.authors[0].name, "김철수");
    assert_eq!(head.authors[0].commits, 4);
    assert_eq!(head.authors[0].first, 1_700_000_000);
    assert_eq!(head.authors[0].last, 1_700_000_700);
    assert_eq!(head.commits, 6);
    assert!(!head.cached);

    let scoped = authors::list(&repo, Scope::Head, Some("./dir/"), None).unwrap();
    assert_eq!(scoped.path.as_deref(), Some("dir"));
    assert_eq!(ours(&scoped), shortlog(&dir, &["HEAD"], Some("dir")));
    assert_eq!(scoped.commits, 2);

    git(&dir, &["tag", "v1", "side"]);
    let all = authors::list(&repo, Scope::All, None, None).unwrap();
    assert_eq!(
        ours(&all),
        shortlog(&dir, &["--branches", "--remotes", "--tags"], None)
    );
    assert_eq!(
        all.authors
            .iter()
            .find(|a| a.name == "Park")
            .unwrap()
            .commits,
        2
    );

    // `.`과 빈 경로는 저장소 전체
    assert_eq!(
        authors::list(&repo, Scope::Head, Some("."), None)
            .unwrap()
            .path,
        None
    );
    // 없는 경로는 아무도 없다(git도 오류 없이 빈 결과)
    let none = authors::list(&repo, Scope::Head, Some("없는/경로"), None).unwrap();
    assert!(none.authors.is_empty());
}

#[test]
fn fixtures_like_git_shortlog() {
    for dir in [history(), encoding()] {
        let repo = Repo::discover(&dir).unwrap();
        let head = authors::list(&repo, Scope::Head, None, None).unwrap();
        assert_eq!(ours(&head), shortlog(&dir, &["HEAD"], None), "{dir:?}");
        let all = authors::list(&repo, Scope::All, None, None).unwrap();
        assert_eq!(
            ours(&all),
            shortlog(&dir, &["--branches", "--remotes", "--tags"], None),
            "{dir:?}"
        );
        for path in git_text(&dir, &["ls-files"]).lines().take(5) {
            let scoped = authors::list(&repo, Scope::Head, Some(path), None).unwrap();
            assert_eq!(
                ours(&scoped),
                shortlog(&dir, &["HEAD"], Some(path)),
                "{dir:?} {path}"
            );
        }
    }
}

#[test]
fn empty_repo_has_no_authors() {
    let dir = scratch_repo("authors-empty");
    let repo = Repo::discover(&dir).unwrap();
    for scope in [Scope::Head, Scope::All] {
        let list = authors::list(&repo, scope, None, None).unwrap();
        assert!(list.authors.is_empty());
        assert_eq!(list.commits, 0);
    }
}

#[test]
fn caches_until_refs_move() {
    let dir = people_repo("authors-cache");
    let cache_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("authors-cache-results");
    let _ = std::fs::remove_dir_all(&cache_dir);
    let cache = DiskCache::new(cache_dir, 1 << 20);
    let repo = Repo::discover(&dir).unwrap();

    let first = authors::list(&repo, Scope::Head, None, Some(&cache)).unwrap();
    assert!(!first.cached);
    let again = authors::list(&repo, Scope::Head, None, Some(&cache)).unwrap();
    assert!(again.cached);
    assert_eq!(again.authors, first.authors);
    // 경로가 다르면 다른 결과
    assert!(
        !authors::list(&repo, Scope::Head, Some("dir"), Some(&cache))
            .unwrap()
            .cached
    );
    let all = authors::list(&repo, Scope::All, None, Some(&cache)).unwrap();
    assert!(!all.cached);
    assert!(
        authors::list(&repo, Scope::All, None, Some(&cache))
            .unwrap()
            .cached
    );

    // 새 커밋(HEAD가 움직인다), 다른 브랜치만 움직여도 모든 참조의 결과는 다시 센다
    commit_as(&dir, LEE, "또", 1_700_001_000);
    let moved = authors::list(&repo, Scope::Head, None, Some(&cache)).unwrap();
    assert!(!moved.cached);
    assert_eq!(moved.commits, first.commits + 1);
    git(&dir, &["branch", "extra", "side~1"]);
    assert!(
        !authors::list(&repo, Scope::All, None, Some(&cache))
            .unwrap()
            .cached
    );

    // mailmap을 바꾸면 다른 결과(옛 메일이 따로 보인다)
    std::fs::remove_file(dir.join(".mailmap")).unwrap();
    let repo = Repo::discover(&dir).unwrap();
    let unmapped = authors::list(&repo, Scope::Head, None, Some(&cache)).unwrap();
    assert!(!unmapped.cached);
    assert_eq!(ours(&unmapped), shortlog(&dir, &["HEAD"], None));
}
