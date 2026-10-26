//! 커밋 상세(gix 트리 비교)를 fixtures의 모든 커밋에서 `git diff-tree` 결과와 대조한다.

mod common;

use std::path::Path;

use common::{commit_all, encoding, git, git_text, history, scratch_repo};
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

/// 상세의 행. 줄 수를 시간 안에 다 세지 못했으면 나머지는 [`commit::file_stats`]로 센다(화면과 같다).
fn ours(repo: &Repo, sha: &str) -> Vec<Row> {
    let mut detail = commit::commit_detail(repo, repo.resolve_commit(sha).unwrap(), None).unwrap();
    assert_eq!(detail.info.sha, sha);
    assert_eq!(detail.info.changed_files, detail.files.len());
    if !detail.counted {
        let pending: Vec<_> = detail
            .files
            .iter()
            .filter(|f| f.stat.is_none())
            .map(request)
            .collect();
        let mut stats = commit::file_stats(repo, &pending).unwrap().into_iter();
        for file in detail.files.iter_mut().filter(|f| f.stat.is_none()) {
            file.stat = stats.next();
        }
    }
    detail
        .files
        .into_iter()
        .map(|f| {
            let stat = f.stat.expect("줄 수");
            assert!(!stat.missing);
            let stats = (!stat.binary).then_some((stat.additions, stat.deletions));
            (f.status.letter(), f.path, f.old_path, stats)
        })
        .collect()
}

fn request(file: &commit::FileChange) -> commit::StatRequest {
    commit::StatRequest {
        path: file.path.clone(),
        old_path: file.old_path.clone(),
        old_id: file.old_id.clone(),
        new_id: file.new_id.clone(),
        submodule: file.submodule,
        worktree: false,
    }
}

fn versions(repo: &Repo, file: &commit::FileChange) -> commit::FileVersions {
    commit::file_versions(
        repo,
        file.old_id.as_deref(),
        file.new_id.as_deref(),
        Some(&file.path),
        false,
        false,
    )
    .unwrap()
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
    let detail = commit::commit_detail(&repo, repo.resolve_commit(&rename).unwrap(), None).unwrap();
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
    let versions = versions(&repo, file);
    assert_eq!(
        versions.old.unwrap().text,
        String::from_utf8(git(&dir, &["show", &format!("{rename}^:src/old_name.rs")])).unwrap()
    );
    assert_eq!(
        versions.new.unwrap().text,
        String::from_utf8(git(&dir, &["show", &format!("{rename}:src/new_name.rs")])).unwrap()
    );

    let binary = git_text(&dir, &["log", "-1", "--format=%H", "--", "assets/logo.bin"]);
    let detail = commit::commit_detail(&repo, repo.resolve_commit(&binary).unwrap(), None).unwrap();
    let file = detail
        .files
        .iter()
        .find(|f| f.path == "assets/logo.bin")
        .unwrap();
    assert!(file.stat.unwrap().binary);
    let versions = commit::file_versions(
        &repo,
        file.old_id.as_deref(),
        file.new_id.as_deref(),
        Some(&file.path),
        false,
        false,
    )
    .unwrap();
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
    let detail = commit::commit_detail(&repo, repo.resolve_commit("HEAD").unwrap(), None).unwrap();
    let tracked = git_text(&shallow, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert_eq!(detail.files.len(), tracked.lines().count());
    assert!(detail.files.iter().all(|f| f.status == FileStatus::Added));
}

#[test]
fn many_renamed_files_are_paired_like_git() {
    // 비슷하지만 같지 않은 파일 40개를 옮긴다. gix의 rename 추적은 짝 수(40×40)를 한도 1,000과 견줘 건너뛰었다.
    let dir = scratch_repo("commit-many-renames");
    let body = |i: usize, edited: bool| -> String {
        (0..20)
            .map(|line| {
                if edited && line == 0 {
                    format!("edited header of file {i}\n")
                } else {
                    format!("file {i} line {line}: the quick brown fox jumps over the lazy dog\n")
                }
            })
            .collect()
    };
    std::fs::create_dir_all(dir.join("old")).unwrap();
    for i in 0..40 {
        std::fs::write(dir.join(format!("old/f{i}.txt")), body(i, false)).unwrap();
    }
    commit_all(&dir, "add");
    std::fs::remove_dir_all(dir.join("old")).unwrap();
    std::fs::create_dir_all(dir.join("new")).unwrap();
    for i in 0..40 {
        std::fs::write(dir.join(format!("new/f{i}.txt")), body(i, true)).unwrap();
    }
    let head = commit_all(&dir, "move and edit");

    let repo = Repo::discover(&dir).unwrap();
    assert_eq!(ours(&repo, &head), reference(&dir, &head));
    let plain = commit::commit_detail(&repo, repo.resolve_commit(&head).unwrap(), None).unwrap();
    assert_eq!(plain.files.len(), 40);
    assert!(plain.files.iter().all(|f| f.status == FileStatus::Renamed));
    // 화면이 고른 파일(히스토리에서 열었다)도 같다. 이 커밋에 없는 파일은 그대로다.
    // 줄 수는 시간 안에 센 만큼만 들어 있어 호출마다 다를 수 있으니 빼고 견준다.
    let without_stats = |files: &[commit::FileChange]| -> Vec<commit::FileChange> {
        files
            .iter()
            .cloned()
            .map(|f| commit::FileChange { stat: None, ..f })
            .collect()
    };
    for focus in ["new/f7.txt", "no/such"] {
        let focused =
            commit::commit_detail(&repo, repo.resolve_commit(&head).unwrap(), Some(focus)).unwrap();
        assert_eq!(without_stats(&focused.files), without_stats(&plain.files));
    }
}

#[test]
fn focused_file_pairs_like_file_history_when_git_would_not() {
    // 원본 하나에 새 파일 둘(이름이 같은 것과 더 비슷한 것): 커밋 전체로는 git처럼 한쪽만 rename이다. 히스토리에서
    // 연 파일(focus)은 파일 히스토리가 이어 붙인 원본을 보인다(`--follow`처럼 그 파일만 견준다).
    let dir = scratch_repo("commit-focus-rename");
    let lines = |tag: &str| -> String {
        (0..30)
            .map(|i| format!("{tag} line {i}: the quick brown fox jumps over the lazy dog\n"))
            .collect()
    };
    std::fs::write(dir.join("a.txt"), lines("base")).unwrap();
    commit_all(&dir, "add");
    std::fs::remove_file(dir.join("a.txt")).unwrap();
    std::fs::create_dir_all(dir.join("x")).unwrap();
    std::fs::write(dir.join("x/a.txt"), lines("base") + "one more\n").unwrap();
    std::fs::write(dir.join("b.txt"), lines("base")).unwrap();
    let head = commit_all(&dir, "copy and move");
    let repo = Repo::discover(&dir).unwrap();
    assert_eq!(ours(&repo, &head), reference(&dir, &head));
    let id = repo.resolve_commit(&head).unwrap();
    let plain = commit::commit_detail(&repo, id, None).unwrap();
    let renamed: Vec<_> = plain
        .files
        .iter()
        .filter(|f| f.status == FileStatus::Renamed)
        .collect();
    assert_eq!(renamed.len(), 1, "{:?}", plain.files);
}

// ---------------------------------------------------------------- 리뷰에서 찾은 경우(회귀 테스트)

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn body(tag: &str, n: usize) -> String {
    (0..n)
        .map(|i| format!("{tag} line {i}: the quick brown fox jumps over the lazy dog\n"))
        .collect()
}

fn detail(repo: &Repo, sha: &str) -> commit::CommitDetail {
    commit::commit_detail(repo, repo.resolve_commit(sha).unwrap(), None).unwrap()
}

/// 고친 rename 둘과 새 파일 수백 개: git의 rename 한도는 짝 수 1,000²다(gix는 1,000과 견줘 추가 + 삭제로 보였다).
#[test]
fn renames_next_to_hundreds_of_new_files() {
    for extra in [400usize, 600] {
        let dir = scratch_repo(&format!("commit-rename-limit-{extra}"));
        write(&dir, "src/alpha.rs", body("alpha", 30).as_bytes());
        write(&dir, "src/beta.rs", body("beta", 30).as_bytes());
        commit_all(&dir, "base");
        std::fs::remove_file(dir.join("src/alpha.rs")).unwrap();
        std::fs::remove_file(dir.join("src/beta.rs")).unwrap();
        write(
            &dir,
            "lib/alpha.rs",
            format!("// moved\n{}", body("alpha", 30)).as_bytes(),
        );
        write(
            &dir,
            "lib/beta.rs",
            format!("// moved\n{}", body("beta", 30)).as_bytes(),
        );
        for i in 0..extra {
            write(
                &dir,
                &format!("vendor/v{i}.txt"),
                format!("vendored {i}\n").as_bytes(),
            );
        }
        let sha = commit_all(&dir, "move two files, vendor many");
        let repo = Repo::discover(&dir).unwrap();
        assert_eq!(ours(&repo, &sha), reference(&dir, &sha), "{extra}");
    }
}

/// 새 파일마다 옛 파일 둘을 섞었다(섞은 비율이 파일마다 다르다). 새 파일마다 후보 넷을 두고 모두를 비슷한 순으로
/// 짝짓는 순서가 git과 같아야 같은 짝이 나온다. 크기가 크게 다른 짝(크기로 거른다)과 바이너리도 섞는다.
#[test]
fn crossed_renames_pick_like_git() {
    let dir = scratch_repo("commit-crossed-renames");
    let n = 120;
    let lines = |i: usize| -> Vec<String> {
        (0..30)
            .map(|k| {
                format!(
                    "old {i} row {k}: {}\n",
                    "the quick brown fox ".repeat(1 + (i + k) % 3)
                )
            })
            .collect()
    };
    for i in 0..n {
        write(&dir, &format!("old/o{i}.txt"), lines(i).concat().as_bytes());
    }
    let bin: Vec<u8> = (0..3000u32).flat_map(|i| (i * 7).to_le_bytes()).collect();
    write(&dir, "old/blob.bin", &bin);
    write(&dir, "old/tiny.txt", b"tiny\n");
    commit_all(&dir, "base");
    std::fs::remove_dir_all(dir.join("old")).unwrap();
    for i in 0..n {
        let (own, next) = (lines(i), lines((i + 1) % n));
        // 앞쪽은 자기 줄, 뒤쪽은 다음 파일의 줄. 자기 줄의 비율이 i마다 다르다(40%..80%).
        let keep = 12 + i % 13;
        let mixed: String = (0..30)
            .map(|k| {
                if k < keep {
                    own[k].clone()
                } else {
                    next[k].clone()
                }
            })
            .collect();
        write(&dir, &format!("new/n{i}.txt"), mixed.as_bytes());
    }
    let mut bin2 = bin.clone();
    bin2.truncate(9000);
    write(&dir, "new/blob2.bin", &bin2);
    write(&dir, "new/huge.txt", "tiny\n".repeat(400).as_bytes());
    let sha = commit_all(&dir, "shuffle");
    let repo = Repo::discover(&dir).unwrap();
    let expected = reference(&dir, &sha);
    let renamed = expected.iter().filter(|r| r.0 == 'R').count();
    assert!(renamed > n / 2, "git이 짝지은 수 {renamed}");
    assert_eq!(ours(&repo, &sha), expected);
}

/// 조금 고쳐 옮긴 바이너리, 빈 파일, 내용이 같은 파일: git과 같다.
#[test]
fn binary_empty_and_exact_renames_match_git() {
    let dir = scratch_repo("commit-binary-rename");
    let mut bin: Vec<u8> = (0..4000u32).flat_map(|i| i.to_le_bytes()).collect();
    write(&dir, "img/logo.bin", &bin);
    write(&dir, "img/keep.txt", b"stay\n");
    write(&dir, "pkg/__init__.py", b"");
    write(&dir, "pkg/mod.py", b"x = 1\n");
    write(&dir, "a/x.txt", b"same\n");
    commit_all(&dir, "base");
    std::fs::remove_file(dir.join("img/logo.bin")).unwrap();
    bin[100] ^= 0xff;
    write(&dir, "assets/logo.bin", &bin);
    git(&dir, &["mv", "pkg", "lib"]);
    std::fs::create_dir_all(dir.join("b")).unwrap();
    std::fs::rename(dir.join("a/x.txt"), dir.join("b/x.txt")).unwrap();
    let sha = commit_all(&dir, "move things");
    let repo = Repo::discover(&dir).unwrap();
    assert_eq!(ours(&repo, &sha), reference(&dir, &sha));
}

/// 하위 모듈(gitlink)의 줄 수는 git처럼 `Subproject commit` 한 줄이다(더함 1, 바꿈 1/1).
#[test]
fn gitlink_counts_like_git() {
    let dir = scratch_repo("commit-gitlink");
    write(&dir, "a.txt", b"one\n");
    let base = commit_all(&dir, "base");
    git(
        &dir,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{base},sub"),
        ],
    );
    git(&dir, &["commit", "-q", "-m", "add gitlink"]);
    let added = git_text(&dir, &["rev-parse", "HEAD"]);
    git(
        &dir,
        &[
            "update-index",
            "--cacheinfo",
            &format!("160000,{added},sub"),
        ],
    );
    git(&dir, &["commit", "-q", "-m", "bump gitlink"]);
    let bumped = git_text(&dir, &["rev-parse", "HEAD"]);
    let repo = Repo::discover(&dir).unwrap();
    for sha in [&added, &bumped] {
        assert_eq!(ours(&repo, sha), reference(&dir, sha));
        let d = detail(&repo, sha);
        assert!(d.files[0].submodule);
        assert_eq!(d.files[0].new_mode.as_deref(), Some("160000"));
    }
}

/// `.gitattributes`의 `binary`·`-diff`는 줄로 비교하지 않는다(git numstat `-`).
#[test]
fn diff_attributes_make_files_binary() {
    let dir = scratch_repo("commit-attributes");
    write(&dir, ".gitattributes", b"*.dat binary\n*.lock -diff\n");
    write(&dir, "x.dat", b"a\nb\n");
    write(&dir, "y.lock", b"a\nb\n");
    write(&dir, "z.txt", b"a\nb\n");
    commit_all(&dir, "base");
    for name in ["x.dat", "y.lock", "z.txt"] {
        write(&dir, name, b"a\nc\n");
    }
    let sha = commit_all(&dir, "edit");
    let repo = Repo::discover(&dir).unwrap();
    assert_eq!(ours(&repo, &sha), reference(&dir, &sha));
    let d = detail(&repo, &sha);
    for file in &d.files {
        assert_eq!(
            versions(&repo, file).binary,
            file.path != "z.txt",
            "{}",
            file.path
        );
    }
}

/// 줄 diff에 보이지 않는 바뀜(줄 끝, 끝 개행, BOM, 인코딩, 권한)은 따로 알린다(diff가 비어 보였다).
#[test]
fn invisible_changes_are_noted() {
    let dir = scratch_repo("commit-invisible");
    write(&dir, "crlf.txt", b"one\ntwo\nthree\n");
    write(&dir, "eol.txt", b"one\ntwo");
    write(&dir, "bom.txt", b"one\ntwo\n");
    write(&dir, "euc.txt", b"\xbe\xc8\xb3\xe7\n");
    write(&dir, "run.sh", b"echo hi\n");
    commit_all(&dir, "base");
    write(&dir, "crlf.txt", b"one\r\ntwo\r\nthree\r\n");
    write(&dir, "eol.txt", b"one\ntwo\n");
    write(&dir, "bom.txt", b"\xef\xbb\xbfone\ntwo\n");
    write(&dir, "euc.txt", "안녕\n".as_bytes());
    git(&dir, &["update-index", "--chmod=+x", "run.sh"]);
    git(&dir, &["add", "crlf.txt", "eol.txt", "bom.txt", "euc.txt"]);
    git(&dir, &["commit", "-q", "-m", "invisible changes"]);
    let sha = git_text(&dir, &["rev-parse", "HEAD"]);
    let repo = Repo::discover(&dir).unwrap();
    let d = detail(&repo, &sha);
    let notes =
        |path: &str| versions(&repo, d.files.iter().find(|f| f.path == path).unwrap()).notes;
    assert_eq!(notes("crlf.txt"), ["줄 끝: LF → CRLF"]);
    assert_eq!(notes("eol.txt"), ["끝 개행을 더했다"]);
    assert_eq!(notes("bom.txt"), ["UTF-8 BOM을 더했다"]);
    assert_eq!(notes("euc.txt"), ["인코딩: EUC-KR → UTF-8"]);
    let run = d.files.iter().find(|f| f.path == "run.sh").unwrap();
    assert_eq!(
        (run.old_mode.as_deref(), run.new_mode.as_deref()),
        (Some("100644"), Some("100755"))
    );
}

/// 부분 클론(`--filter=blob:none`): 받지 않은 blob이 있어도 상세·hover 카드가 뜬다(파일마다 "내용 없음").
#[test]
fn partial_clone_details_without_blobs() {
    let src = scratch_repo("commit-partial-source");
    git(&src, &["config", "uploadpack.allowFilter", "true"]);
    write(&src, "a.txt", body("a", 10).as_bytes());
    write(&src, "b.txt", body("b", 10).as_bytes());
    commit_all(&src, "one");
    write(&src, "a.txt", body("a2", 10).as_bytes());
    commit_all(&src, "two");
    std::fs::remove_file(src.join("b.txt")).unwrap();
    write(
        &src,
        "c/b2.txt",
        format!("// moved\n{}", body("b", 10)).as_bytes(),
    );
    commit_all(&src, "three: rename with edit");
    let dst = Path::new(env!("CARGO_TARGET_TMPDIR")).join("commit-partial-clone");
    let _ = std::fs::remove_dir_all(&dst);
    let url = format!("file://{}", src.to_string_lossy().replace('\\', "/"));
    git(
        src.parent().unwrap(),
        &[
            "clone",
            "-q",
            "--no-checkout",
            "--filter=blob:none",
            &url,
            &dst.to_string_lossy(),
        ],
    );
    let repo = Repo::discover(&dst).unwrap();
    for rev in ["HEAD", "HEAD~1", "HEAD~2"] {
        let id = repo.resolve_commit(rev).unwrap();
        let d = commit::commit_detail(&repo, id, None).unwrap_or_else(|e| panic!("{rev}: {e}"));
        assert!(!d.files.is_empty());
        assert!(
            d.files.iter().all(|f| f.stat.is_some_and(|s| s.missing)),
            "{rev}"
        );
        assert!(repo.commit_info(id).is_ok(), "{rev} hover");
        let file = &d.files[0];
        let v = versions(&repo, file);
        assert!(v.missing && v.old.is_none() && v.new.is_none());
    }
    let d = commit::commit_detail(&repo, repo.resolve_commit("HEAD").unwrap(), None).unwrap();
    assert!(
        d.renames_incomplete,
        "blob이 없어 rename을 다 견주지 못했다"
    );
}

/// UTF-8이 아닌 경로는 표시만 한다(그 경로로 blame·히스토리를 열 수 없다).
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_paths_are_marked() {
    use std::os::unix::ffi::OsStrExt;
    let dir = scratch_repo("commit-latin1");
    std::fs::write(
        dir.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt")),
        b"x\n",
    )
    .unwrap();
    let sha = commit_all(&dir, "latin1 name");
    let repo = Repo::discover(&dir).unwrap();
    let d = detail(&repo, &sha);
    assert!(d.files[0].path_lossy);
}

/// 큰 파일은 내용을 보내지 않고 크기만 알린다. 화면이 "그래도 보기"로 다시 요청하면 보낸다.
#[test]
fn large_files_are_sent_on_request() {
    let dir = scratch_repo("commit-large");
    let mut big = String::new();
    for i in 0.. {
        if big.len() as u64 > commit::MAX_VERSIONS_BYTES {
            break;
        }
        big.push_str(&format!("row {i} value {}\n", i * 7));
    }
    write(&dir, "big.csv", big.as_bytes());
    let sha = commit_all(&dir, "big");
    let repo = Repo::discover(&dir).unwrap();
    let d = detail(&repo, &sha);
    let file = &d.files[0];
    let v = versions(&repo, file);
    assert_eq!(v.too_large, Some(big.len() as u64));
    assert!(v.new.is_none());
    let full = commit::file_versions(
        &repo,
        file.old_id.as_deref(),
        file.new_id.as_deref(),
        Some(&file.path),
        true,
        false,
    )
    .unwrap();
    assert_eq!(full.new.unwrap().text, big);
}
