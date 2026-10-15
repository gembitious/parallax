//! 저장소 읽기(gix)를 fixtures에서 git 결과와 대조한다.

mod common;

use common::{encoding, git, git_text, history};
use plx_core::repo::Repo;

#[test]
fn discover_and_relative_paths() {
    let dir = history();
    let repo = Repo::discover(&dir.join("src/lib.rs")).unwrap();
    // Windows에서도 `\\?\` 없는 경로다(git 작업 디렉터리·화면 표시용).
    assert_eq!(repo.workdir(), dunce::canonicalize(&dir).unwrap());
    assert!(!repo.workdir().to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(
        repo.relative_path(&dir.join("src/lib.rs")).unwrap(),
        "src/lib.rs"
    );
    assert_eq!(
        repo.relative_path(&dir.join("src/../core/util.rs"))
            .unwrap(),
        "core/util.rs"
    );
    // 지금은 없는(삭제된) 경로도 된다 — 파일 히스토리에 쓴다
    assert_eq!(
        repo.relative_path(&dir.join("docs/gone.md")).unwrap(),
        "docs/gone.md"
    );
    assert!(
        repo.relative_path(&std::env::temp_dir().join("elsewhere.txt"))
            .is_err()
    );

    let enc = encoding();
    let repo = Repo::discover(&enc.join("한글")).unwrap();
    assert_eq!(
        repo.relative_path(&enc.join("한글/파일 이름.txt")).unwrap(),
        "한글/파일 이름.txt"
    );
}

#[cfg(unix)]
#[test]
fn relative_path_through_a_symlink() {
    let link = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("history-link");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(history(), &link).unwrap();
    let repo = Repo::discover(&link.join("src/lib.rs")).unwrap();
    assert_eq!(
        repo.relative_path(&link.join("src/lib.rs")).unwrap(),
        "src/lib.rs"
    );
}

#[test]
fn not_a_repository() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("not-a-repo");
    std::fs::create_dir_all(&dir).unwrap();
    // CARGO_TARGET_TMPDIR가 이 저장소(parallax) 안이라 위로 올라가면 저장소를 찾는다. 그 경우는 건너뛴다.
    if let Ok(repo) = Repo::discover(&dir) {
        assert_ne!(repo.workdir(), dunce::canonicalize(&dir).unwrap());
    }
}

#[test]
fn revisions_and_blobs() {
    let dir = history();
    let repo = Repo::discover(&dir).unwrap();
    for rev in ["HEAD", "HEAD~1", "v0.1", "v0.2", "topic", "main~3"] {
        let expected = git_text(&dir, &["rev-parse", &format!("{rev}^{{commit}}")]);
        assert_eq!(
            repo.resolve_commit(rev).unwrap().to_string(),
            expected,
            "{rev}"
        );
    }
    assert!(repo.resolve_commit("no-such-ref").is_err());
    assert_eq!(
        repo.head().unwrap().unwrap(),
        repo.resolve_commit("HEAD").unwrap()
    );

    let head = repo.resolve_commit("HEAD").unwrap();
    assert_eq!(
        repo.blob_at(head, "src/lib.rs").unwrap().unwrap(),
        git(&dir, &["show", "HEAD:src/lib.rs"])
    );
    assert_eq!(repo.blob_at(head, "no/such.rs").unwrap(), None);
    let v01 = repo.resolve_commit("v0.1").unwrap();
    assert_eq!(repo.blob_at(v01, "src/new_name.rs").unwrap(), None);
    assert!(
        repo.blob_at(head, "src").is_err(),
        "디렉터리는 파일이 아니다"
    );

    let enc = encoding();
    let repo = Repo::discover(&enc).unwrap();
    let head = repo.resolve_commit("HEAD").unwrap();
    assert_eq!(
        repo.blob_at(head, "한글/파일 이름.txt").unwrap().unwrap(),
        git(&enc, &["show", "HEAD:한글/파일 이름.txt"])
    );
    assert_eq!(
        repo.blob_at(head, "crlf.txt").unwrap().unwrap(),
        b"first\r\nsecond\r\nthird\r\n"
    );
}

/// gix로 센 변경 파일 수가 git과 같은지 확인한다(rename은 하나로 센다).
fn changed_files_by_git(dir: &std::path::Path, sha: &str) -> usize {
    let parents = git_text(dir, &["rev-list", "--parents", "-n", "1", sha]);
    let first_parent = parents.split(' ').nth(1);
    let out = match first_parent {
        Some(parent) => git_text(dir, &["diff-tree", "-r", "-M", "--name-only", parent, sha]),
        None => git_text(
            dir,
            &[
                "diff-tree",
                "-r",
                "--root",
                "--no-commit-id",
                "--name-only",
                sha,
            ],
        ),
    };
    out.lines().filter(|l| !l.is_empty()).count()
}

#[test]
fn commit_info_matches_git() {
    let dir = history();
    let repo = Repo::discover(&dir).unwrap();
    for rev in git_text(&dir, &["rev-list", "--all"]).lines() {
        let info = repo.commit_info(repo.resolve_commit(rev).unwrap()).unwrap();
        assert_eq!(info.sha, rev);
        assert_eq!(
            info.summary,
            git_text(&dir, &["log", "-1", "--format=%s", rev])
        );
        assert_eq!(
            info.message,
            git_text(&dir, &["log", "-1", "--format=%B", rev])
        );
        assert_eq!(
            info.author.name,
            git_text(&dir, &["log", "-1", "--format=%an", rev])
        );
        assert_eq!(
            info.author.time.to_string(),
            git_text(&dir, &["log", "-1", "--format=%at", rev])
        );
        assert_eq!(info.author.tz_offset, 9 * 3600);
        assert_eq!(
            info.parents.join(" "),
            git_text(&dir, &["log", "-1", "--format=%P", rev])
        );
        assert_eq!(
            info.changed_files,
            changed_files_by_git(&dir, rev),
            "{rev} {}",
            info.summary
        );
    }
}

#[test]
fn commit_info_decodes_korean_and_euc_kr() {
    let dir = encoding();
    let repo = Repo::discover(&dir).unwrap();
    let head = repo
        .commit_info(repo.resolve_commit("HEAD").unwrap())
        .unwrap();
    assert_eq!(head.summary, "인코딩 테스트: EUC-KR 커밋 메시지");
    assert_eq!(head.changed_files, 1);
    let korean = repo
        .commit_info(repo.resolve_commit("HEAD~2").unwrap())
        .unwrap();
    assert_eq!(korean.author.name, "김개발");
    assert_eq!(korean.summary, "한글 파일 수정");
}
