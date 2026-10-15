//! fixtures/make-repos.sh 결정성 검사.
//!
//! 어느 플랫폼에서 만들어도 커밋 sha가 같아야 git 출력 파서의 골든 테스트를 플랫폼 사이에 공유할 수 있다.
//! sha가 같으면 저장소 내용(경로 바이트, 줄 끝, 작성자, 시각)이 바이트 단위로 같다는 뜻이다.
//! bash와 git이 필요하다. Windows는 Git for Windows의 bash를 찾아 쓴다(`PLX_BASH`로 바꿀 수 있다).

use std::path::{Path, PathBuf};
use std::process::Command;

// fixtures/make-repos.sh를 바꾸면 여기를 갱신한다.
const HISTORY_HEAD: &str = "1b019344c559bac26fac7ba37d9e3b80d57986cf";
const HISTORY_TOPIC: &str = "c1aef07a4d8678d53236db8fc8419d3464093d14";
const HISTORY_TAG_V02: &str = "f4b65c0ae0c519616e6141f186c8a230e9ebf745";
const ENCODING_HEAD: &str = "1b48386e5d6f32919d7c85347ab5cd49f8dcd889";

const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

fn bash() -> PathBuf {
    if let Some(path) = std::env::var_os("PLX_BASH").filter(|p| !p.is_empty()) {
        return path.into();
    }
    // Windows의 PATH에는 WSL의 bash.exe가 먼저 있을 수 있다. git과 같이 설치된 bash를 쓴다.
    // `git --exec-path` = <Git>/mingw64/libexec/git-core → <Git>/bin/bash.exe
    if cfg!(windows)
        && let Ok(out) = Command::new("git").arg("--exec-path").output()
    {
        let exec_path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if let Some(bash) = exec_path
            .ancestors()
            .map(|dir| dir.join("bin").join("bash.exe"))
            .find(|candidate| candidate.is_file())
        {
            return bash;
        }
    }
    PathBuf::from("bash")
}

/// 사용자 git 설정의 영향을 받지 않게 실행한다(예: status.showUntrackedFiles).
fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git 실행");
    assert!(
        out.status.success(),
        "git {args:?} 실패: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn git_text(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git(repo, args))
        .expect("UTF-8 출력")
        .trim_end()
        .to_owned()
}

fn make_fixtures() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("임시 디렉터리");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/make-repos.sh");
    // Windows 경로의 역슬래시는 bash 안에서도 통하지만 슬래시로 바꿔 넘기는 편이 안전하다.
    let slash = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let status = Command::new(bash())
        .arg(slash(&script))
        .arg(slash(dir.path()))
        .status()
        .expect("bash 실행 (Windows는 Git for Windows, 또는 PLX_BASH)");
    assert!(status.success(), "fixtures/make-repos.sh 실패: {status}");
    dir
}

#[test]
fn fixtures_are_deterministic_and_cover_scenarios() {
    let dir = make_fixtures();
    let history = dir.path().join("history");
    let encoding = dir.path().join("encoding");

    // 플랫폼과 무관하게 같은 sha
    assert_eq!(
        git_text(&history, &["rev-parse", "HEAD"]),
        HISTORY_HEAD,
        "history HEAD"
    );
    assert_eq!(
        git_text(&history, &["rev-parse", "topic"]),
        HISTORY_TOPIC,
        "history topic"
    );
    assert_eq!(
        git_text(&history, &["rev-parse", "v0.2"]),
        HISTORY_TAG_V02,
        "history v0.2 태그 객체"
    );
    assert_eq!(
        git_text(&encoding, &["rev-parse", "HEAD"]),
        ENCODING_HEAD,
        "encoding HEAD"
    );

    // history: rename·디렉터리 이동이 rename으로 잡힌다
    let renames = git_text(
        &history,
        &["log", "--format=", "--name-status", "-M", "--diff-filter=R"],
    );
    assert!(
        renames.contains("src/old_name.rs\tsrc/new_name.rs"),
        "{renames}"
    );
    assert!(renames.contains("lib/util.rs\tcore/util.rs"), "{renames}");
    assert_eq!(
        git_text(
            &history,
            &["log", "--follow", "--format=%s", "--", "src/new_name.rs"]
        )
        .lines()
        .count(),
        2
    );

    // history: merge 2개(evil merge, 충돌 해결), merge에서만 생긴 줄
    assert_eq!(
        git_text(&history, &["rev-list", "--merges", "--count", "HEAD"]),
        "2"
    );
    // merge에서만 생긴 줄(11번째)은 blame이 merge 커밋을 가리킨다.
    let evil = git_text(
        &history,
        &[
            "blame",
            "--porcelain",
            "-L",
            "11,11",
            "HEAD",
            "--",
            "src/lib.rs",
        ],
    );
    assert!(
        evil.lines().any(|l| l == "summary Merge branch feature"),
        "{evil}"
    );
    assert_eq!(
        git_text(&history, &["show", "HEAD:src/conflict.rs"]),
        "shared line: resolved by combining main and conflict values"
    );

    // history: 파일 사이 블록 이동은 blame -C로 원래 파일을 가리킨다
    let blame = git_text(
        &history,
        &[
            "blame",
            "-C",
            "-C",
            "--porcelain",
            "-L",
            "2,2",
            "HEAD",
            "--",
            "src/target.rs",
        ],
    );
    assert!(
        blame.lines().any(|l| l == "filename src/moves.rs"),
        "{blame}"
    );

    // history: 미커밋 변경
    assert_eq!(
        git_text(&history, &["status", "--porcelain"]),
        " M src/lib.rs\n?? untracked.txt"
    );

    // encoding: 줄 끝이 blob에 그대로 남는다
    assert_eq!(
        git(&encoding, &["cat-file", "-p", "HEAD:crlf.txt"]),
        b"first\r\nsecond\r\nthird\r\n"
    );
    assert_eq!(
        git(&encoding, &["cat-file", "-p", "HEAD~3:lf-to-crlf.txt"]),
        b"one\ntwo\nthree\n"
    );
    assert_eq!(
        git(&encoding, &["cat-file", "-p", "HEAD:lf-to-crlf.txt"]),
        b"one\r\ntwo\r\nthree\r\n"
    );
    assert_eq!(
        git(&encoding, &["cat-file", "-p", "HEAD:mixed.txt"]),
        b"lf line\ncrlf line\r\nlf again\n"
    );

    // encoding: 한글·공백·이모지 경로가 quotepath 없이 그대로
    let files = git(&encoding, &["ls-files", "-z"]);
    let files: Vec<&str> = files
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| std::str::from_utf8(s).expect("UTF-8 경로"))
        .collect();
    for expected in ["한글/파일 이름.txt", "문서/설명.md", "이모지-😀.txt"] {
        assert!(files.contains(&expected), "{expected} 없음: {files:?}");
    }

    // encoding: 한글 메시지·작성자, 한글 경로 rename
    assert_eq!(
        git_text(&encoding, &["log", "-1", "--format=%an", "HEAD~1"]),
        "김개발"
    );
    assert_eq!(
        git_text(&encoding, &["log", "-1", "--format=%s", "HEAD"]),
        "설명.md를 문서/로 옮김"
    );
    let renames = git_text(
        &encoding,
        &["log", "--format=", "--name-status", "-M", "--diff-filter=R"],
    );
    assert!(renames.contains("한글/설명.md\t문서/설명.md"), "{renames}");

    // encoding: UTF-8이 아닌 바이트(EUC-KR "안녕하세요")
    assert_eq!(
        git(&encoding, &["cat-file", "-p", "HEAD:euc-kr.txt"]),
        b"\xbe\xc8\xb3\xe7\xc7\xcf\xbc\xbc\xbf\xe4\n"
    );
}
