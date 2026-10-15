//! fixtures/make-repos.sh 결정성 검사.
//!
//! 어느 플랫폼에서 만들어도 커밋 sha가 같아야 git 출력 파서의 골든 테스트를 플랫폼 사이에 공유할 수 있다.
//! sha가 같으면 저장소 내용(경로 바이트, 줄 끝, 작성자, 시각)이 바이트 단위로 같다는 뜻이다.
//! bash와 git이 필요하다. Windows는 Git for Windows의 bash를 찾아 쓴다(`PLX_BASH`로 바꿀 수 있다).

mod common;

use common::{encoding, git, git_text, history};

// fixtures/make-repos.sh를 바꾸면 여기를 갱신한다.
const HISTORY_HEAD: &str = "1b019344c559bac26fac7ba37d9e3b80d57986cf";
const HISTORY_TOPIC: &str = "c1aef07a4d8678d53236db8fc8419d3464093d14";
const HISTORY_TAG_V02: &str = "f4b65c0ae0c519616e6141f186c8a230e9ebf745";
const ENCODING_HEAD: &str = "ad2e88fbcca1721f2faa1b0daf3c0a75565eb505";

#[test]
fn shas_are_the_same_on_every_platform() {
    assert_eq!(git_text(&history(), &["rev-parse", "HEAD"]), HISTORY_HEAD);
    assert_eq!(git_text(&history(), &["rev-parse", "topic"]), HISTORY_TOPIC);
    assert_eq!(
        git_text(&history(), &["rev-parse", "v0.2"]),
        HISTORY_TAG_V02
    );
    assert_eq!(git_text(&encoding(), &["rev-parse", "HEAD"]), ENCODING_HEAD);
}

#[test]
fn history_scenarios() {
    let history = history();

    // rename·디렉터리 이동이 rename으로 잡힌다
    let renames = git_text(
        &history,
        &["log", "--format=", "--name-status", "-M", "--diff-filter=R"],
    );
    assert!(
        renames.contains("src/old_name.rs\tsrc/new_name.rs"),
        "{renames}"
    );
    assert!(renames.contains("lib/util.rs\tcore/util.rs"), "{renames}");
    let follow = git_text(
        &history,
        &["log", "--follow", "--format=%s", "--", "src/new_name.rs"],
    );
    assert_eq!(follow.lines().count(), 2, "{follow}");

    // merge 2개(evil merge, 충돌 해결). merge에서만 생긴 줄(11번째)은 blame이 merge 커밋을 가리킨다.
    assert_eq!(
        git_text(&history, &["rev-list", "--merges", "--count", "HEAD"]),
        "2"
    );
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

    // 파일 사이 블록 이동은 blame -C로 원래 파일을 가리킨다
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

    // 미커밋 변경
    assert_eq!(
        git_text(&history, &["status", "--porcelain"]),
        " M src/lib.rs\n?? untracked.txt"
    );
}

#[test]
fn encoding_scenarios() {
    let encoding = encoding();

    // 줄 끝이 blob에 그대로 남는다
    assert_eq!(
        git(&encoding, &["cat-file", "-p", "HEAD:crlf.txt"]),
        b"first\r\nsecond\r\nthird\r\n"
    );
    assert_eq!(
        git(&encoding, &["cat-file", "-p", "HEAD~4:lf-to-crlf.txt"]),
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

    // 한글·공백·이모지 경로가 quotepath 없이 그대로
    let files = git(&encoding, &["ls-files", "-z"]);
    let files: Vec<&str> = files
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| std::str::from_utf8(s).expect("UTF-8 경로"))
        .collect();
    for expected in ["한글/파일 이름.txt", "문서/설명.md", "이모지-😀.txt"] {
        assert!(files.contains(&expected), "{expected} 없음: {files:?}");
    }

    // 한글 메시지·작성자, 한글 경로 rename
    assert_eq!(
        git_text(&encoding, &["log", "-1", "--format=%an", "HEAD~2"]),
        "김개발"
    );
    assert_eq!(
        git_text(&encoding, &["log", "-1", "--format=%s", "HEAD~1"]),
        "설명.md를 문서/로 옮김"
    );
    let renames = git_text(
        &encoding,
        &["log", "--format=", "--name-status", "-M", "--diff-filter=R"],
    );
    assert!(renames.contains("한글/설명.md\t문서/설명.md"), "{renames}");

    // EUC-KR로 인코딩된 커밋 메시지: 객체에는 encoding 헤더와 EUC-KR 바이트, git log는 UTF-8로 바꿔 보여 준다
    let raw = git(&encoding, &["cat-file", "-p", "HEAD"]);
    assert!(raw.windows(16).any(|w| w == b"encoding euc-kr\n"));
    assert_eq!(
        git_text(&encoding, &["log", "-1", "--format=%s", "HEAD"]),
        "인코딩 테스트: EUC-KR 커밋 메시지"
    );

    // UTF-8이 아닌 내용(EUC-KR "안녕하세요", "EUC-KR 줄 추가")
    assert_eq!(
        git(&encoding, &["cat-file", "-p", "HEAD:euc-kr.txt"]),
        b"\xbe\xc8\xb3\xe7\xc7\xcf\xbc\xbc\xbf\xe4\nEUC-KR \xc1\xd9 \xc3\xdf\xb0\xa1\n"
    );
}
