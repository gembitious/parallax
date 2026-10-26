//! 외부에서 오는 입력을 읽는 파서는 어떤 바이트에도 패닉하지 않는다(오류로 돌려준다).
//!
//! 상주 앱이라 패닉 하나가 요청을 멈추고 캐시 잠금을 오염시킨다. git 출력에는 저장소가 정한 경로·메시지·이름이
//! 들고, AI 노트(`refs/notes/ai`)는 클론한 저장소에 누구나 넣을 수 있으며, CLI 인자와 검색어는 사람이 친다.
//! 실제 출력(fixtures 저장소, `fixtures/git-ai` 노트)을 씨앗으로 고정 시드 변이를 만든다: 바이트 뒤집기, 형식의
//! 구분자·경계 숫자·깨진 UTF-8 끼우기, 자르기, 다른 씨앗 조각 붙이기. 청크 경계도 아무 데나 끊는다.
//! 더 오래 돌리려면 `PLX_FUZZ_ITERS=1000000`(대상마다, 기본 3000). 패닉한 입력은 `PLX_FUZZ_OUT`(기본 임시
//! 디렉터리)에 남는다.

mod common;

use std::cell::{Cell, RefCell};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Once;

use common::git;

struct Rng(u64);

impl Rng {
    /// xorshift64*
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// 끼워 넣어 볼 조각: 각 형식의 구분자·키워드, 경계 숫자, 깨진 UTF-8
const PIECES: &[&[u8]] = &[
    b"\n",
    b"\0",
    b"\t",
    b" ",
    b"\"",
    b"\\",
    b"-",
    b",",
    b":",
    b"0",
    b"4294967295",
    b"4294967296",
    b"18446744073709551616",
    b"-1",
    b"---",
    b"\r\n",
    b"\xff",
    b"\xe1\x84",
    b"\xed\x95\x9c",
    b"\xf0\x9f\x98\x80",
    b"filename ",
    b"previous ",
    b"boundary\n",
    b"author-time ",
    b"author-tz ",
    b"\x01",
    b"s_",
    b"h_",
    b"::t_",
    b"{",
    b"}",
    b"@@ -1,2 +3,4 @@",
    b"@@ -0,0 +0,0 @@",
    b"diff --git a/x b/x\n",
    b"+++ b/x\n",
    b"--- a/x\n",
    b"\\ No newline at end of file\n",
    b"R100\0",
    b"C050\0",
    b"A\0",
    b"D\0",
    b"0000000000000000000000000000000000000000",
    b" 1 1 1\n",
    b" 0 0 0\n",
    b"\"\\303\\251\"",
    b"5-3",
    b" +0900",
    b" -9999",
];

fn mutate(rng: &mut Rng, seeds: &[Vec<u8>]) -> Vec<u8> {
    let mut v = seeds[rng.below(seeds.len())].clone();
    for _ in 0..1 + rng.below(6) {
        let len = v.len();
        match rng.below(8) {
            0 if len > 0 => {
                let i = rng.below(len);
                v[i] ^= 1 << rng.below(8);
            }
            1 => {
                let i = rng.below(len + 1);
                let piece = PIECES[rng.below(PIECES.len())];
                v.splice(i..i, piece.iter().copied());
            }
            2 if len > 0 => {
                let a = rng.below(len);
                v.drain(a..(a + 1 + rng.below(32)).min(len));
            }
            3 if len > 0 => {
                let a = rng.below(len);
                let part = v[a..(a + 1 + rng.below(64)).min(len)].to_vec();
                let i = rng.below(len + 1);
                v.splice(i..i, part);
            }
            4 if len > 0 => {
                let i = rng.below(len);
                v[i] = rng.next() as u8;
            }
            5 if len > 0 => v.truncate(rng.below(len)),
            6 => {
                let other = &seeds[rng.below(seeds.len())];
                if !other.is_empty() {
                    let a = rng.below(other.len());
                    let b = (a + 1 + rng.below(128)).min(other.len());
                    let i = rng.below(len + 1);
                    v.splice(i..i, other[a..b].iter().copied());
                }
            }
            _ => {
                // 숫자 하나를 경계 값으로
                let Some(i) = (0..32)
                    .map(|_| rng.below(len))
                    .find(|&i| v.get(i).is_some_and(u8::is_ascii_digit))
                else {
                    continue;
                };
                let end = (i..len).find(|&j| !v[j].is_ascii_digit()).unwrap_or(len);
                let value: &[u8] = [
                    &b"0"[..],
                    b"4294967295",
                    b"4294967296",
                    b"99999999999999999999",
                ][rng.below(4)];
                v.splice(i..end, value.iter().copied());
            }
        }
    }
    v
}

thread_local! {
    static FUZZING: Cell<bool> = const { Cell::new(false) };
    static LAST_PANIC: RefCell<String> = const { RefCell::new(String::new()) };
}

/// 퍼징 중인 스레드의 패닉은 문장만 남기고, 나머지(테스트의 assert 등)는 기본 훅으로 보낸다.
fn install_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if FUZZING.get() {
                LAST_PANIC.with(|p| *p.borrow_mut() = info.to_string());
            } else {
                default(info);
            }
        }));
    });
}

fn iters() -> usize {
    std::env::var("PLX_FUZZ_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000)
}

fn out_dir() -> PathBuf {
    std::env::var_os("PLX_FUZZ_OUT").map_or_else(std::env::temp_dir, PathBuf::from)
}

/// 씨앗 그대로 한 번씩, 그다음 변이를 `iters()`번 넣는다. 패닉하면 자리마다 입력 하나를 남기고 실패한다.
fn fuzz(name: &str, seeds: &[Vec<u8>], mut target: impl FnMut(&[u8], &mut Rng)) {
    assert!(!seeds.is_empty(), "{name}: 씨앗이 없다");
    install_hook();
    let mut rng = Rng(name.bytes().fold(0x9E37_79B9_7F4A_7C15, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01B3)
    }));
    let mut sites = Vec::new();
    for i in 0..seeds.len() + iters() {
        let input = seeds
            .get(i)
            .cloned()
            .unwrap_or_else(|| mutate(&mut rng, seeds));
        let mut sub = Rng(rng.next() | 1);
        FUZZING.set(true);
        let result = panic::catch_unwind(AssertUnwindSafe(|| target(&input, &mut sub)));
        FUZZING.set(false);
        if result.is_err() {
            let site = LAST_PANIC.with(|p| p.borrow().lines().next().unwrap_or("").to_owned());
            if !sites.iter().any(|(s, _)| *s == site) {
                let file = out_dir().join(format!("plx-fuzz-{name}-{}.bin", sites.len() + 1));
                std::fs::write(&file, &input).unwrap();
                sites.push((site, file));
            }
        }
    }
    assert!(sites.is_empty(), "{name}: 패닉한 입력이 있다 {sites:#?}");
}

/// 청크를 아무 데서나 끊는다(git 출력은 파이프에서 이렇게 온다).
fn chunks<'a>(input: &'a [u8], rng: &mut Rng) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        let (a, b) = rest.split_at(1 + rng.below(rest.len().min(4096)));
        out.push(a);
        rest = b;
    }
    out
}

fn tracked(repo: &Path) -> Vec<String> {
    String::from_utf8(git(repo, &["ls-files", "-z"]))
        .unwrap()
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn repos() -> [PathBuf; 2] {
    [common::history(), common::encoding()]
}

fn blame(input: &[u8], rng: &mut Rng) -> Result<(), plx_core::blame::ParseError> {
    let mut parser = plx_core::blame::IncrementalParser::new();
    let mut out = Vec::new();
    for chunk in chunks(input, rng) {
        parser.push(chunk, &mut out)?;
    }
    parser.finish()
}

#[test]
fn blame_incremental() {
    let mut seeds = Vec::new();
    for repo in repos() {
        for path in tracked(&repo) {
            seeds.push(git(&repo, &["blame", "--incremental", "HEAD", "--", &path]));
        }
    }
    for seed in &seeds {
        blame(seed, &mut Rng(1)).expect("씨앗은 git의 실제 출력이다");
    }
    fuzz("blame", &seeds, |input, rng| {
        let _ = blame(input, rng);
    });
}

/// `plx_core::log`의 FORMAT과 같다(씨앗이 파서와 맞는지 아래에서 확인한다).
const LOG_FORMAT: &str = "--format=%x01%H%x00%P%x00%aN%x00%aE%x00%ad%x00%s";

fn log(input: &[u8], path: &str, rng: &mut Rng) -> Result<usize, plx_core::log::ParseError> {
    let mut parser = plx_core::log::LogParser::new(path);
    let mut out = Vec::new();
    for chunk in chunks(input, rng) {
        parser.push(chunk, &mut out)?;
    }
    parser.finish(&mut out)?;
    Ok(out.len())
}

#[test]
fn file_history_log() {
    let mut seeds = Vec::new();
    for repo in repos() {
        // 저장소 전체(`plx log .`)는 경로도 바뀐 파일 목록도 없다.
        let whole = git(&repo, &["log", "-z", LOG_FORMAT, "--date=raw", "HEAD"]);
        assert!(log(&whole, "", &mut Rng(1)).unwrap() > 0);
        seeds.push(whole);
        for path in tracked(&repo) {
            let args = [
                "log",
                "-z",
                LOG_FORMAT,
                "--date=raw",
                "--name-status",
                "HEAD",
                "--",
                &path,
            ];
            let seed = git(&repo, &args);
            log(&seed, &path, &mut Rng(1)).expect("씨앗은 git의 실제 출력이다");
            seeds.push(seed);
        }
    }
    fuzz("log", &seeds, |input, rng| {
        let path = ["", "src/lib.rs", "한글/파일 이름.txt"][rng.below(3)];
        let _ = log(input, path, rng);
    });
}

/// `plx_core::line_log`의 FORMAT과 같다.
const LINE_FORMAT: &str = "--format=%x01%H%x00%P%x00%aN%x00%aE%x00%ad%x00%s%x00";

fn line_log(input: &[u8], rng: &mut Rng) -> Result<usize, plx_core::log::ParseError> {
    let mut parser = plx_core::line_log::LineLogParser::new("src/lib.rs");
    let mut out = Vec::new();
    for chunk in chunks(input, rng) {
        parser.push(chunk, &mut out)?;
    }
    parser.finish(&mut out)?;
    Ok(out.len())
}

#[test]
fn line_history_log() {
    let repo = common::history();
    let mut seeds = Vec::new();
    for range in ["1,3", "2,9", "5,5"] {
        let spec = format!("-L{range}:src/lib.rs");
        let args = [
            "-c",
            "diff.suppressBlankEmpty=false",
            "log",
            LINE_FORMAT,
            "--date=raw",
            "--no-color",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            &spec,
            "HEAD",
        ];
        let seed = git(&repo, &args);
        assert!(line_log(&seed, &mut Rng(1)).expect("씨앗은 git의 실제 출력이다") > 0);
        seeds.push(seed);
    }
    fuzz("line-log", &seeds, |input, rng| {
        let _ = line_log(input, rng);
    });
}

#[test]
fn ai_notes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/git-ai");
    let mut seeds = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let bundle = entry.unwrap().path();
        if bundle.extension().is_none_or(|e| e != "bundle") {
            continue;
        }
        let name = bundle.file_stem().unwrap().to_string_lossy();
        let repo = common::scratch_repo(&format!("fuzz-{name}"));
        let source = bundle.to_str().expect("UTF-8 경로");
        git(
            &repo,
            &["fetch", "-q", "--update-head-ok", source, "+refs/*:refs/*"],
        );
        for line in String::from_utf8(git(&repo, &["notes", "--ref=ai", "list"]))
            .unwrap()
            .lines()
        {
            let blob = line.split_whitespace().next().unwrap();
            seeds.push(git(&repo, &["cat-file", "-p", blob]));
        }
    }
    for seed in &seeds {
        plx_core::ai_notes::parse(std::str::from_utf8(seed).unwrap()).expect("git-ai가 쓴 노트다");
    }
    fuzz("ai-notes", &seeds, |input, _| {
        if let Ok(log) = plx_core::ai_notes::parse(&String::from_utf8_lossy(input)) {
            for file in &log.files {
                let _ = log.spans(&file.path, &mut |_| None);
            }
        }
    });
}

#[test]
fn commit_table_bytes() {
    use plx_core::commit_table::CommitTable;
    let id = |n: u8| gix::ObjectId::from_bytes_or_panic(&[n; 20]);
    let table = CommitTable::from_commits(vec![
        (id(1), vec![], 10),
        (id(2), vec![id(1)], 20),
        (id(3), vec![id(1), id(2)], 30),
        (id(4), vec![id(3), id(9)], 40),
    ]);
    let seeds = vec![table.to_bytes()];
    fuzz("commit-table", &seeds, |input, _| {
        // 디스크 캐시 파일이 망가져도 읽기는 None이고, 읽은 표는 어느 번호로 물어도 패닉하지 않는다.
        if let Some(table) = CommitTable::from_bytes(input) {
            for i in 0..table.len() as u32 {
                let _ = (table.id(i), table.time(i), table.lookup(&table.id(i)));
                for &p in table.parents(i) {
                    let _ = table.id(p);
                }
            }
        }
    });
}

fn grep(
    input: &[u8],
    rev: Option<&str>,
    rng: &mut Rng,
) -> Result<usize, plx_core::grep::ParseError> {
    let mut parser = plx_core::grep::GrepParser::new(rev);
    let mut out = Vec::new();
    for chunk in chunks(input, rng) {
        parser.push(chunk, &mut out)?;
    }
    parser.finish(&mut out)?;
    // 맞은 자리는 내용 안에 있고 앞에서 뒤로 온다(화면이 그대로 자른다).
    for record in &out {
        let len = record.line.text.encode_utf16().count() as u32;
        let mut at = 0;
        for &[start, end] in &record.line.ranges {
            assert!(
                at <= start && start < end && end <= len,
                "{:?}",
                record.line
            );
            at = end;
        }
    }
    Ok(out.len())
}

#[test]
fn grep_output() {
    use plx_core::grep::{GrepQuery, grep_args};
    let mut seeds = Vec::new();
    for repo in repos() {
        for (pattern, regex) in [("e", false), ("[^ -~]+|t.", true)] {
            let query = GrepQuery {
                pattern: pattern.to_owned(),
                regex,
                case_sensitive: false,
                word: false,
                rev: None,
                paths: Vec::new(),
            };
            let args = grep_args(&query, Some("HEAD"));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            seeds.push(git(&repo, &args));
        }
    }
    // 내용 안의 표시 비슷한 바이트, 닫지 않은 표시, 깨진 UTF-8, 아주 긴 줄
    seeds.push(b"HEAD:a\x001\x00\x1b[7m\x1b[7mx\x1b[m\x1b[m\x1b[\xff\x1b[7m\n".to_vec());
    seeds.push(
        [
            b"HEAD:b\x002\x00".as_slice(),
            &"가".repeat(900).into_bytes(),
            b"\x1b[7m\xed\x95\x1b[m\n",
        ]
        .concat(),
    );
    for seed in &seeds {
        grep(seed, Some("HEAD"), &mut Rng(1)).expect("씨앗은 git의 실제 출력이다");
    }
    fuzz("grep", &seeds, |input, rng| {
        let _ = grep(input, Some("HEAD"), rng);
    });
}

#[test]
fn recent_repos_file() {
    use plx_core::recent;
    let seeds = vec![
        recent::serialize(&[
            "/code/parallax".to_owned(),
            "/Users/나/코드/한글 저장소".to_owned(),
            r"C:\code\b".to_owned(),
        ]),
        br#"{"version":2,"repos":[{"path":"/x","pinned":true},{"path":7},{},"/y",{"path":"rel"}]}"#
            .to_vec(),
    ];
    fuzz("recent", &seeds, |input, _| {
        // 사람이나 다른 버전이 고친 recent.json: 읽을 수 있는 항목만, 많아야 LIMIT개, 같은 경로는 한 번.
        let paths = recent::parse(input);
        assert!(paths.len() <= recent::LIMIT);
        for (i, path) in paths.iter().enumerate() {
            assert!(!paths[..i].contains(path));
        }
        assert_eq!(recent::parse(&recent::serialize(&paths)), paths);
    });
}

#[test]
fn cli_install_files() {
    use plx_core::cli_install::{
        first_in_path, path_has_dir, path_with_dir, path_with_dir_before, shim_script, shim_target,
    };
    let seeds = vec![
        shim_script("/Applications/parallax.app/Contents/MacOS/plx")
            .unwrap()
            .into_bytes(),
        b"#!/bin/sh\n# parallax plx shim (scripts/install-macos-shim.sh)\nexec '/x y/plx' \"$@\"\n"
            .to_vec(),
        br#"C:\bin;%LOCALAPPDATA%\parallax\;"D:\x y";;"#.to_vec(),
    ];
    let dir = r"C:\Users\나\AppData\Local\parallax";
    let expand = |text: &str| text.replace("%LOCALAPPDATA%", r"C:\Users\나\AppData\Local");
    fuzz("cli-install", &seeds, |input, _| {
        // `/usr/local/bin/plx`에 있던 파일: 우리 shim으로 읽은 경로는 같은 shim으로 다시 쓸 수 있다.
        if let Some(bin) = shim_target(input) {
            let again = shim_script(&bin).expect("읽은 경로로 shim을 만든다");
            assert_eq!(shim_target(again.as_bytes()).as_deref(), Some(bin.as_str()));
        }
        // 레지스트리의 PATH 값: 더한 뒤에는 그 폴더가 들어 있다.
        let path = String::from_utf8_lossy(input);
        if let Some(added) = path_with_dir(&path, dir, expand) {
            assert!(path_has_dir(&added, dir, expand));
        }
        // 먼저 불리는 다른 plx 앞으로 옮긴 값: 그 폴더는 한 번만 있고, 다른 (빈 것이 아닌) 항목은 차례 그대로다.
        let before = input.len() % (path.split(';').count() + 1);
        let moved = path_with_dir_before(&path, dir, before, expand);
        let ours = |entry: &&str| path_has_dir(entry, dir, expand);
        assert_eq!(moved.split(';').filter(ours).count(), 1, "{moved:?}");
        let others = |value: &str| {
            value
                .split(';')
                .filter(|entry| !entry.trim().is_empty() && !ours(entry))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(others(&moved), others(&path));
        // 새 터미널은 시스템 PATH부터 찾는다: 같은 값이 둘에 있으면 시스템 쪽에서 찾는다.
        let has = |found: &str| found.len().is_multiple_of(3);
        if let Some(hit) = first_in_path(&path, &path, expand, has) {
            assert!(!hit.user && has(&hit.dir), "{hit:?}");
        }
    });
}

#[test]
fn cli_args() {
    const TOKENS: &[&str] = &[
        "log",
        "show",
        "-L",
        "-L1,2:a",
        "1,2:a",
        "--",
        "-h",
        "-V",
        "--help",
        "-",
        "a",
        "a:1",
        "a:1:2",
        "a:0",
        "a:-1",
        ":",
        "::",
        "a:99999999999",
        ",:",
        "-L,",
        "-L:",
        "-Lx",
        "-L1,:a",
        "-L,2:a",
        "-L2,1:a",
        "-L0,0:a",
        "-L4294967296,1:a",
        "HEAD",
        "HEAD~1",
        "src/lib.rs",
        "src/lib.rs:3:9",
        "한글/파일 이름.txt",
        "a b",
        "",
        "..",
        "/",
        "a:한",
    ];
    let cwd = common::history();
    fuzz("cli", &[Vec::new()], |_, rng| {
        let args: Vec<String> = (0..rng.below(5))
            .map(|_| TOKENS[rng.below(TOKENS.len())].to_owned())
            .collect();
        let _ = plx_core::cli::parse(&args, &cwd, plx_core::cli::stat);
    });
}

/// 경로·검색어에 쓸 글자: ASCII, 완성형 한글, 호환 자모(초성 검색), 조합형 자모(NFD 경로), 결합 문자,
/// 이모지와 ZWJ, 대소문자 바꾸기가 길이를 바꾸는 글자
fn random_text(rng: &mut Rng, max: usize) -> String {
    const UNITS: &[&str] = &[
        "a",
        "b",
        "C",
        "/",
        ".",
        " ",
        "_",
        "-",
        ":",
        "1",
        "\\",
        "\t",
        "파",
        "일",
        "팡",
        "ㄱ",
        "ㅍ",
        "ㅇ",
        "\u{1111}",
        "\u{1161}",
        "\u{11bc}",
        "\u{1100}\u{1161}",
        "\u{0301}",
        "\u{1f600}",
        "\u{200d}",
        "\u{ff21}",
        "\u{0130}",
        "\u{00df}",
        "\u{fb01}",
        "\u{d7a3}",
        "\u{ac00}",
        "\u{2028}",
    ];
    (0..rng.below(max + 1))
        .map(|_| UNITS[rng.below(UNITS.len())])
        .collect()
}

#[test]
fn fuzzy_match() {
    use plx_core::fuzzy::{Matcher, Query, base_of, compose_jamo};
    let mut matcher = Matcher::new();
    fuzz("fuzzy", &[Vec::new()], |_, rng| {
        let query = Query::parse(&random_text(rng, 8));
        let path: Vec<char> = random_text(rng, 24).chars().collect();
        let mut positions = Vec::new();
        let _ = matcher.score(&query, &path[..], base_of(&path[..]), Some(&mut positions));
        let _ = matcher.score(&query, &path[..], base_of(&path[..]), None);
        if let Some((composed, starts)) = compose_jamo(&path) {
            assert_eq!(composed.len() + 1, starts.len());
            let _ = matcher.score(
                &query,
                &composed[..],
                base_of(&composed[..]),
                Some(&mut positions),
            );
        }
        let ascii: String = path.iter().filter(|c| c.is_ascii()).collect();
        let _ = matcher.score(
            &query,
            ascii.as_bytes(),
            base_of(ascii.as_bytes()),
            Some(&mut positions),
        );
    });
}

fn random_lines(rng: &mut Rng) -> Vec<u8> {
    const LINES: &[&[u8]] = &[
        b"a\n", b"b\n", b"\n", b"}\n", b"a\r\n", b"c", b"\r", b"x\ry\n", b"\xff\n", b"\0\n",
    ];
    (0..rng.below(30))
        .flat_map(|_| LINES[rng.below(LINES.len())].iter().copied())
        .collect()
}

#[test]
fn line_map_text_similarity() {
    use plx_core::similarity::{Index, Spans, score, score_of};
    fuzz("linemap", &[Vec::new()], |_, rng| {
        let before = random_lines(rng);
        let after = random_lines(rng);
        let map = plx_core::linemap::LineMap::new(&before, &after);
        for line in [0, 1, 2, 29, 30, 31, 1000, u32::MAX] {
            let _ = map.to_before(line);
        }
        let _ = plx_core::text::decode(&before);
        let _ = plx_core::text::line_count(&after);
        let text = rng.below(2) == 0;
        let (a, b) = (Spans::new(&before, text), Spans::new(&after, text));
        let _ = score(&a, &b);
        let mut copied = Vec::new();
        Index::new(&[Some(&a), None, Some(&b)]).copied(&b, &mut copied);
        let _ = score_of(copied.first().copied().unwrap_or(0), a.size(), b.size());
    });
}
