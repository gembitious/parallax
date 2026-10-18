//! `plx` 인자를 상주 인스턴스에 넘기기 전에 대상 저장소를 여는 확인(`cli::check_repo`). 실제 저장소로 본다.

mod common;

use std::path::Path;

use common::{commit_all, scratch_repo};
use plx_core::cli::{self, Parsed, Request};

fn request(cwd: &Path, args: &[&str]) -> Request {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    match cli::parse(&args, cwd, cli::stat) {
        Ok(Parsed::Request(request)) => request,
        other => panic!("{args:?} → {other:?}"),
    }
}

#[test]
fn requests_in_a_repository_pass() {
    let repo = scratch_repo("cli-check");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/a.rs"), "one\n").unwrap();
    let head = commit_all(&repo, "first");
    let sub = repo.join("src");
    for args in [
        vec![],
        vec!["a.rs:1"],
        vec!["log", "a.rs"],
        // 지금은 없는(지운) 경로의 히스토리
        vec!["log", "gone/old.rs"],
        vec!["log", "-L", "1,1:a.rs"],
        vec!["show", "HEAD"],
        vec!["show", &head[..10]],
    ] {
        let request = request(&sub, &args);
        assert_eq!(cli::check_repo(&request, &sub), Ok(()), "{args:?}");
    }
}

#[test]
fn outside_a_repository_or_unknown_revision_fails() {
    let repo = scratch_repo("cli-check-bad");
    std::fs::write(repo.join("a.rs"), "one\n").unwrap();
    commit_all(&repo, "first");
    let show = request(&repo, &["show", "no-such-branch"]);
    let error = cli::check_repo(&show, &repo).unwrap_err().to_string();
    assert!(error.contains("리비전을 찾을 수 없다"), "{error}");

    // 저장소 밖: 임시 디렉터리 아래의 저장소가 아닌 곳
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("b.rs"), "x\n").unwrap();
    for args in [vec![], vec!["b.rs"], vec!["log", "b.rs"]] {
        let request = request(outside.path(), &args);
        let error = cli::check_repo(&request, outside.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("git 저장소가 아니다"), "{args:?}: {error}");
    }
}
