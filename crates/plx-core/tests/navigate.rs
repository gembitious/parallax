//! blame 화면 이동(`,`, `<`, `>`)을 fixtures에서 git 결과와 대조한다.

mod common;

use common::{git_text, history};
use plx_core::navigate::{self, Direction, Version};
use plx_core::repo::Repo;

fn at(rev: &str, path: &str) -> Version {
    Version {
        rev: Some(rev.to_owned()),
        path: path.to_owned(),
    }
}

fn working(path: &str) -> Version {
    Version {
        rev: None,
        path: path.to_owned(),
    }
}

/// 첫 부모를 따라간 파일 이력(최신 순). merge가 바꾼 버전도 들어 있다.
fn follow_log(dir: &std::path::Path, path: &str) -> Vec<String> {
    git_text(
        dir,
        &[
            "log",
            "--first-parent",
            "--follow",
            "--format=%H",
            "--",
            path,
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

fn walk(repo: &Repo, from: Version, direction: Direction) -> Vec<Version> {
    let mut seen = Vec::new();
    let mut current = from;
    while let Some(next) = navigate::neighbor(repo, &current, direction).unwrap() {
        assert!(!seen.contains(&next), "되돌아왔다: {next:?} ({seen:?})");
        assert!(seen.len() < 100, "끝나지 않는다");
        seen.push(next.clone());
        current = next;
    }
    seen
}

#[test]
fn older_and_newer_follow_a_rename() {
    let dir = history();
    let repo = Repo::discover(&dir).unwrap();
    let log = follow_log(&dir, "src/new_name.rs");
    assert_eq!(log.len(), 2, "rename 커밋과 처음 만든 커밋: {log:?}");
    let (renamed, added) = (&log[0], &log[1]);

    // 작업트리는 HEAD와 같으므로 HEAD 버전(= rename 커밋 버전)을 건너뛰고 rename 전으로 간다.
    let older = walk(&repo, working("src/new_name.rs"), Direction::Older);
    assert_eq!(older, [at(added, "src/old_name.rs")]);

    // 거꾸로: rename을 따라 새 이름으로, 그다음 작업트리
    let newer = walk(&repo, at(added, "src/old_name.rs"), Direction::Newer);
    assert_eq!(
        newer,
        [at(renamed, "src/new_name.rs"), working("src/new_name.rs")]
    );
}

#[test]
fn older_walks_the_follow_log_and_newer_comes_back() {
    let dir = history();
    let repo = Repo::discover(&dir).unwrap();
    // src/lib.rs는 작업트리에서 고쳐져 있어 HEAD 버전부터 시작한다.
    let log = follow_log(&dir, "src/lib.rs");
    let older = walk(&repo, working("src/lib.rs"), Direction::Older);
    let revs: Vec<&str> = older.iter().map(|v| v.rev.as_deref().unwrap()).collect();
    assert_eq!(revs, log);
    assert!(older.iter().all(|v| v.path == "src/lib.rs"));

    assert_eq!(
        git_text(&dir, &["log", "-1", "--format=%s", &log[0]]),
        "Merge branch feature",
        "merge가 바꾼 버전(evil merge)을 건너뛰지 않는다"
    );

    // 가장 오래된 버전에서 다음으로 가면 같은 길을 거꾸로 지나 작업트리에 닿는다.
    let newer = walk(&repo, older.last().unwrap().clone(), Direction::Newer);
    let mut expected: Vec<Version> = older[..older.len() - 1].iter().rev().cloned().collect();
    expected.push(working("src/lib.rs"));
    assert_eq!(newer, expected);
}

#[test]
fn newer_from_a_merged_branch_reaches_the_merge() {
    let dir = history();
    let repo = Repo::discover(&dir).unwrap();
    // feature 브랜치 커밋은 첫 부모 줄기에 없다. 다음 버전은 그 브랜치를 합친 merge 커밋의 버전이다.
    let side = git_text(
        &dir,
        &["log", "-1", "--format=%H", "--grep=feature: change line 2"],
    );
    let merge = git_text(
        &dir,
        &["log", "-1", "--format=%H", "--grep=Merge branch feature"],
    );
    assert_eq!(
        navigate::neighbor(&repo, &at(&side, "src/lib.rs"), Direction::Newer).unwrap(),
        Some(at(&merge, "src/lib.rs"))
    );
    // 이전 버전은 그 브랜치를 따라간다.
    let side_parent = git_text(&dir, &["rev-parse", &format!("{side}^")]);
    let older = navigate::neighbor(&repo, &at(&side, "src/lib.rs"), Direction::Older).unwrap();
    let expected = git_text(
        &dir,
        &["log", "-1", "--format=%H", &side_parent, "--", "src/lib.rs"],
    );
    assert_eq!(older, Some(at(&expected, "src/lib.rs")));
}

#[test]
fn no_neighbors_at_the_ends() {
    let dir = history();
    let repo = Repo::discover(&dir).unwrap();
    // 추적하지 않는 파일에는 이전 리비전이 없고, 작업트리보다 새 버전은 없다.
    assert_eq!(
        navigate::neighbor(&repo, &working("untracked.txt"), Direction::Older).unwrap(),
        None
    );
    assert_eq!(
        navigate::neighbor(&repo, &working("src/lib.rs"), Direction::Newer).unwrap(),
        None
    );
}

#[test]
fn map_line_between_versions() {
    let dir = history();
    let repo = Repo::discover(&dir).unwrap();
    let head = git_text(&dir, &["rev-parse", "HEAD"]);
    let v = working("src/lib.rs");
    assert_eq!(navigate::map_line(&repo, &v, &v, 7).unwrap(), 7);

    // 작업트리 src/lib.rs는 HEAD에서 4번째 줄을 고치고 11번째 줄 뒤에 한 줄을 더했다(fixtures/make-repos.sh).
    let diff = git_text(&dir, &["diff", "-U0", "--", "src/lib.rs"]);
    assert!(
        diff.contains("@@ -4 +4 @@") && diff.contains("@@ -11,0 +12 @@"),
        "{diff}"
    );
    let to = at(&head, "src/lib.rs");
    for line in [1, 2, 3, 5, 10, 11] {
        assert_eq!(navigate::map_line(&repo, &v, &to, line).unwrap(), line);
    }
    // 바뀐 줄은 그 자리, 더한 줄은 끼어든 자리(HEAD의 끝 줄)로 간다.
    assert_eq!(navigate::map_line(&repo, &v, &to, 4).unwrap(), 4);
    assert_eq!(navigate::map_line(&repo, &v, &to, 12).unwrap(), 11);
    // 거꾸로도 된다.
    assert_eq!(navigate::map_line(&repo, &to, &v, 11).unwrap(), 11);

    // 없는 버전
    let error = navigate::map_line(&repo, &v, &at(&head, "no/such.rs"), 1).unwrap_err();
    assert!(error.to_string().contains("no/such.rs"), "{error}");
}
