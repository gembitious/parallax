//! 커밋 상세(gix 트리 비교)를 fixtures의 모든 커밋에서 `git diff-tree` 결과와 대조한다.

mod common;

use std::path::Path;

use common::{encoding, git, git_text, history};
use plx_core::commit::{self, FileStatus};
use plx_core::repo::Repo;

/// git이 보는 (상태 글자, 경로, 원래 경로, 더한 줄, 뺀 줄). 바이너리는 줄 수가 None.
type Row = (char, String, Option<String>, Option<(u32, u32)>);

fn diff_tree_args(dir: &Path, sha: &str) -> Vec<String> {
    let parents = git_text(dir, &["rev-list", "--parents", "-n", "1", sha]);
    let mut args: Vec<String> = ["diff-tree", "-r", "-M", "-z", "--no-commit-id"]
        .map(String::from)
        .to_vec();
    match parents.split(' ').nth(1) {
        Some(first_parent) => args.extend([first_parent.to_owned(), sha.to_owned()]),
        None => args.extend(["--root".to_owned(), sha.to_owned()]),
    }
    args
}

fn reference(dir: &Path, sha: &str) -> Vec<Row> {
    let base = diff_tree_args(dir, sha);
    let run = |extra: &str| {
        let mut args: Vec<&str> = base.iter().map(String::as_str).collect();
        args.insert(1, extra);
        git(dir, &args)
    };
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).expect("UTF-8 경로");

    // --name-status -z: 상태\0경로\0 또는 R100\0원래\0새\0
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

    // --numstat -z: 더함\t뺌\t경로\0 또는 더함\t뺌\t\0원래\0새\0. 바이너리는 -\t-
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
            fields.next().unwrap_or(""),
        );
        let path = if rest.is_empty() {
            let _old = tokens.next();
            text(tokens.next().unwrap())
        } else {
            rest.to_owned()
        };
        let stats = add.parse().ok().zip(del.parse().ok());
        let row = rows
            .iter_mut()
            .find(|r| r.1 == path)
            .unwrap_or_else(|| panic!("numstat에만 있는 경로: {path}"));
        row.3 = stats;
    }
    rows.sort_by(|a, b| a.1.cmp(&b.1));
    rows
}

fn ours(repo: &Repo, sha: &str) -> Vec<Row> {
    let detail = commit::commit_detail(repo, repo.resolve_commit(sha).unwrap()).unwrap();
    assert_eq!(detail.info.sha, sha);
    assert_eq!(detail.info.changed_files, detail.files.len());
    assert_eq!(
        detail.additions,
        detail.files.iter().map(|f| f.additions).sum::<u32>()
    );
    detail
        .files
        .into_iter()
        .map(|f| {
            let stats = (!f.binary).then_some((f.additions, f.deletions));
            (f.status.letter(), f.path, f.old_path, stats)
        })
        .collect()
}

fn compare_all(dir: &Path) -> usize {
    let repo = Repo::discover(dir).unwrap();
    let mut compared = 0;
    for sha in git_text(dir, &["rev-list", "--all"]).lines() {
        let expected = reference(dir, sha);
        // rename 유사도 점수는 비교하지 않는다(글자만).
        assert_eq!(
            ours(&repo, sha),
            expected,
            "{sha} {}",
            git_text(dir, &["log", "-1", "--format=%s", sha])
        );
        compared += expected.len();
    }
    compared
}

#[test]
fn history_commits_match_git_diff_tree() {
    let compared = compare_all(&history());
    assert!(compared > 20, "비교한 파일 변경이 너무 적다: {compared}");
}

#[test]
fn encoding_commits_match_git_diff_tree() {
    // 한글·이모지 경로, 한글 경로 rename, CRLF, EUC-KR 내용
    compare_all(&encoding());
}

#[test]
fn rename_and_binary_details() {
    let dir = history();
    let repo = Repo::discover(&dir).unwrap();
    let rename = git_text(
        &dir,
        &["log", "-1", "--format=%H", "--grep=rename old_name"],
    );
    let detail = commit::commit_detail(&repo, repo.resolve_commit(&rename).unwrap()).unwrap();
    let file = detail
        .files
        .iter()
        .find(|f| f.status == FileStatus::Renamed)
        .expect("rename");
    assert_eq!(
        (file.old_path.as_deref(), file.path.as_str()),
        (Some("src/old_name.rs"), "src/new_name.rs")
    );
    // 두 버전 내용
    let versions =
        commit::file_versions(&repo, file.old_id.as_deref(), file.new_id.as_deref()).unwrap();
    assert_eq!(
        versions.old.unwrap().text,
        String::from_utf8(git(&dir, &["show", &format!("{rename}^:src/old_name.rs")])).unwrap()
    );
    assert_eq!(
        versions.new.unwrap().text,
        String::from_utf8(git(&dir, &["show", &format!("{rename}:src/new_name.rs")])).unwrap()
    );

    let binary = git_text(&dir, &["log", "-1", "--format=%H", "--", "assets/logo.bin"]);
    let detail = commit::commit_detail(&repo, repo.resolve_commit(&binary).unwrap()).unwrap();
    let file = detail
        .files
        .iter()
        .find(|f| f.path == "assets/logo.bin")
        .unwrap();
    assert!(file.binary);
    let versions =
        commit::file_versions(&repo, file.old_id.as_deref(), file.new_id.as_deref()).unwrap();
    assert!(versions.binary && versions.old.is_none() && versions.new.is_none());
}

#[test]
fn shallow_clone_boundary_is_a_root_commit() {
    // CI의 actions/checkout처럼 얕게 클론하면 HEAD의 부모 객체가 없다. git은 경계 커밋을 루트로 본다.
    let shallow = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shallow-history");
    let _ = std::fs::remove_dir_all(&shallow);
    // file:// URL이어야 --depth가 듣는다. Windows 경로(D:/…)는 슬래시를 하나 더 붙인다.
    let path = history().display().to_string().replace('\\', "/");
    let source = if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    };
    let parent = shallow.parent().unwrap();
    git(
        parent,
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            &source,
            shallow.file_name().unwrap().to_str().unwrap(),
        ],
    );
    let repo = Repo::discover(&shallow).unwrap();
    let detail = commit::commit_detail(&repo, repo.resolve_commit("HEAD").unwrap()).unwrap();
    let tracked = git_text(&shallow, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert_eq!(detail.files.len(), tracked.lines().count());
    assert!(detail.files.iter().all(|f| f.status == FileStatus::Added));
}
