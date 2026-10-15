//! 라인 히스토리(`git log -L`)를 fixtures에서 git 결과와 대조한다.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{encoding, git, git_text, history};
use plx_core::cache::DiskCache;
use plx_core::commit::FileStatus;
use plx_core::history::{HistoryEvent, HistoryMeta, HistorySessions, HistoryStats, LineRange};
use plx_core::line_log::LineLogStream;
use plx_core::log::LogEntry;
use plx_core::repo::RepoCache;

/// git이 낸 `-L` 출력을 단순하게 나눈 것: 커밋마다 (sha, 새 경로, hunk(머리, 줄들)).
/// fixtures에는 `-- `·`++ `로 시작하는 내용 줄이 없어 줄 앞 글자만 보고 나눠도 된다.
type Reference = Vec<(String, String, Vec<(String, Vec<String>)>)>;

fn reference(repo: &Path, rev: &str, path: &str, start: u32, end: u32) -> Reference {
    let range = format!("-L{start},{end}:{path}");
    let out = git(
        repo,
        &[
            "log",
            "--format=%x02%H",
            "--no-color",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            &range,
            rev,
        ],
    );
    let out = String::from_utf8_lossy(&out).into_owned();
    out.split('\x02')
        .filter(|c| !c.is_empty())
        .map(|commit| {
            let mut lines = commit.lines();
            let sha = lines.next().unwrap().to_owned();
            let mut new_path = path.to_owned();
            let mut hunks: Vec<(String, Vec<String>)> = Vec::new();
            for line in lines {
                if let Some(p) = line.strip_prefix("+++ b/") {
                    new_path = p.to_owned();
                } else if line.starts_with("@@ ") {
                    hunks.push((normalize_header(line), Vec::new()));
                } else if let Some((_, body)) = hunks.last_mut()
                    && matches!(line.as_bytes().first(), Some(b' ' | b'-' | b'+' | b'\\'))
                {
                    body.push(line.trim_end_matches('\r').to_owned());
                }
            }
            (sha, new_path, hunks)
        })
        .collect()
}

fn stream_all(repo: &Path, rev: &str, path: &str, start: u32, end: u32) -> Vec<LogEntry> {
    let sha = git_text(repo, &["rev-parse", rev]);
    let mut stream = LineLogStream::spawn(repo, &sha, path, start, end).expect("git log -L");
    let mut out = Vec::new();
    while stream.read(&mut out).expect("읽기") {}
    stream.finish().expect("git log -L 성공");
    out
}

fn as_reference(entries: &[LogEntry]) -> Reference {
    entries
        .iter()
        .map(|e| {
            let hunks = e
                .hunks
                .as_ref()
                .expect("hunks")
                .iter()
                .map(|h| {
                    let header = format!(
                        "@@ -{} +{} @@",
                        span(h.old_start, h.old_lines),
                        span(h.new_start, h.new_lines)
                    );
                    (header, h.lines.clone())
                })
                .collect();
            (e.sha.clone(), e.path.clone(), hunks)
        })
        .collect()
}

fn span(start: u32, lines: u32) -> String {
    format!("{start},{lines}")
}

/// `@@ -a,b +c,d @@ …`를 `@@ -a,b +c,d @@`로(줄 수를 생략했으면 1, 뒤의 함수 이름은 뗀다).
/// git log -L은 `-1,1`처럼 쓰고 보통 diff는 `-1`처럼 생략한다.
fn normalize_header(line: &str) -> String {
    let mut parts = line.split_whitespace().skip(1);
    let mut side = |sign: char| {
        let part = parts.next().unwrap().strip_prefix(sign).unwrap();
        let (start, lines) = part.split_once(',').unwrap_or((part, "1"));
        format!("{start},{lines}")
    };
    let old = side('-');
    let new = side('+');
    format!("@@ -{old} +{new} @@")
}

#[test]
fn parser_matches_git_for_ranges_in_fixtures() {
    let cases: &[(&Path, &str, &str, u32, u32)] = &[
        (&history(), "HEAD", "src/lib.rs", 1, 3),
        (&history(), "HEAD", "src/lib.rs", 2, 9),
        (&history(), "HEAD", "src/lib.rs", 9, 11),
        (&history(), "HEAD", "src/new_name.rs", 1, 3),
        (&history(), "HEAD", "core/util.rs", 2, 5),
        (&history(), "HEAD", "src/moves.rs", 1, 10),
        (&history(), "HEAD", "src/conflict.rs", 1, 1),
        (&history(), "HEAD", "src/no_eol.txt", 1, 1),
        (&history(), "HEAD~3", "src/lib.rs", 1, 11),
        (&encoding(), "HEAD", "한글/파일 이름.txt", 1, 3),
        (&encoding(), "HEAD", "문서/설명.md", 1, 3),
        (&encoding(), "HEAD", "crlf.txt", 1, 3),
        (&encoding(), "HEAD", "lf-to-crlf.txt", 1, 3),
    ];
    for &(repo, rev, path, start, end) in cases {
        let ours = stream_all(repo, rev, path, start, end);
        assert!(!ours.is_empty(), "{path} {start},{end}");
        assert_eq!(
            as_reference(&ours),
            reference(repo, rev, path, start, end),
            "{path} {start},{end} @{rev}"
        );
    }
}

#[test]
fn renames_merges_and_encodings() {
    let entries = stream_all(&history(), "HEAD", "src/new_name.rs", 1, 3);
    assert_eq!(
        entries
            .iter()
            .map(|e| (
                e.summary.as_str(),
                e.status,
                e.path.as_str(),
                e.old_path.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            (
                "rename old_name.rs to new_name.rs",
                Some(FileStatus::Renamed),
                "src/new_name.rs",
                Some("src/old_name.rs")
            ),
            (
                "initial import",
                Some(FileStatus::Added),
                "src/old_name.rs",
                None
            ),
        ]
    );
    // merge에서만 생긴 줄(11)은 patch 없이 merge 커밋만 나온다.
    let merge = stream_all(&history(), "HEAD", "src/lib.rs", 11, 11);
    assert_eq!(merge[0].summary, "Merge branch feature");
    assert_eq!(merge[0].status, None);
    assert_eq!(merge[0].hunks.as_deref(), Some(&[][..]));

    // EUC-KR 파일의 내용 줄도 한글로 읽는다.
    let euc = stream_all(&encoding(), "HEAD", "euc-kr.txt", 1, 2);
    let lines: Vec<&str> = euc
        .iter()
        .flat_map(|e| e.hunks.iter().flatten())
        .flat_map(|h| h.lines.iter().map(String::as_str))
        .collect();
    assert!(lines.contains(&"+안녕하세요"), "{lines:?}");
    assert!(lines.contains(&"+EUC-KR 줄 추가"), "{lines:?}");
}

// ---------------------------------------------------------------- 요청

fn run_with(
    sessions: &HistorySessions,
    path: &Path,
    rev: Option<&str>,
    start: u32,
    end: u32,
) -> Result<(HistoryMeta, Vec<LogEntry>, HistoryStats), String> {
    let repos = RepoCache::new();
    let id = sessions.register();
    let mut events = Vec::new();
    sessions.run_lines(id, &repos, path, rev, LineRange { start, end }, |event| {
        events.push(event);
        true
    });
    assert_eq!(sessions.active(), 0);
    assert_eq!(
        events.iter().filter(|e| e.is_end()).count(),
        1,
        "{events:?}"
    );
    match events.last().unwrap() {
        HistoryEvent::Done { stats, .. } => {
            let HistoryEvent::Meta(meta) = &events[0] else {
                panic!("{events:?}");
            };
            let entries = events
                .iter()
                .filter_map(|e| match e {
                    HistoryEvent::Entries { entries } => Some(entries.clone()),
                    _ => None,
                })
                .flatten()
                .collect();
            Ok((meta.clone(), entries, stats.clone()))
        }
        HistoryEvent::Error { message } => Err(message.clone()),
        other => panic!("{other:?}"),
    }
}

fn run(
    path: &Path,
    rev: Option<&str>,
    start: u32,
    end: u32,
) -> Result<(HistoryMeta, Vec<LogEntry>, HistoryStats), String> {
    run_with(&HistorySessions::new(), path, rev, start, end)
}

fn shas(entries: &[LogEntry]) -> Vec<String> {
    entries.iter().map(|e| e.sha.clone()).collect()
}

fn line_log_shas(repo: &Path, rev: &str, path: &str, start: u32, end: u32) -> Vec<String> {
    reference(repo, rev, path, start, end)
        .into_iter()
        .map(|(sha, ..)| sha)
        .collect()
}

#[test]
fn pinned_revision_ranges() {
    let dir = history();
    let (meta, entries, stats) = run(&dir.join("src/lib.rs"), Some("HEAD~3"), 2, 5).unwrap();
    assert_eq!(meta.lines, Some(LineRange { start: 2, end: 5 }));
    assert_eq!(
        meta.rev.as_deref(),
        Some(git_text(&dir, &["rev-parse", "HEAD~3"]).as_str())
    );
    assert_eq!(
        shas(&entries),
        line_log_shas(&dir, "HEAD~3", "src/lib.rs", 2, 5)
    );
    assert_eq!(stats.entries, entries.len());
    // 범위를 거꾸로 주거나 파일 끝을 넘기면 맞춘다.
    let (meta, ..) = run(&dir.join("src/lib.rs"), Some("HEAD"), 50, 9).unwrap();
    assert_eq!(meta.lines, Some(LineRange { start: 9, end: 11 }));
}

#[test]
fn worktree_lines_are_moved_to_head() {
    // fixtures의 작업트리 src/lib.rs: 4번 줄을 고쳤고 12번 줄을 더했다(HEAD는 11줄).
    let dir = history();
    let (meta, entries, _) = run(&dir.join("src/lib.rs"), None, 12, 12).unwrap();
    assert_eq!(meta.rev, None);
    assert_eq!(
        meta.lines,
        Some(LineRange { start: 11, end: 11 }),
        "더한 줄은 그 자리의 HEAD 줄로"
    );
    assert_eq!(entries[0].summary, "Merge branch feature");
    let (meta, entries, _) = run(&dir.join("src/lib.rs"), None, 5, 9).unwrap();
    assert_eq!(meta.lines, Some(LineRange { start: 5, end: 9 }));
    assert_eq!(
        shas(&entries),
        line_log_shas(&dir, "HEAD", "src/lib.rs", 5, 9)
    );
}

#[test]
fn files_without_line_history_are_errors() {
    let dir = history();
    for (path, what) in [
        ("untracked.txt", "커밋하지 않은 파일"),
        ("assets/logo.bin", "바이너리"),
        ("empty.txt", "빈 파일"),
    ] {
        let error = run(&dir.join(path), None, 1, 1).unwrap_err();
        assert!(error.contains(what), "{path}: {error}");
    }
}

#[test]
fn finished_line_history_is_cached() {
    let dir = history();
    let cache_dir = tempfile::tempdir().unwrap();
    let sessions = HistorySessions::with_cache(Arc::new(DiskCache::new(
        cache_dir.path().to_path_buf(),
        1 << 30,
    )));
    let path = dir.join("src/new_name.rs");
    let (_, first, stats) = run_with(&sessions, &path, Some("HEAD"), 1, 3).unwrap();
    assert!(!stats.cached);
    let (_, second, stats) = run_with(&sessions, &path, Some("HEAD"), 1, 3).unwrap();
    assert!(stats.cached);
    assert_eq!(first, second);
    let (_, _, stats) = run_with(&sessions, &path, Some("HEAD"), 1, 2).unwrap();
    assert!(!stats.cached, "범위가 다르면 다른 결과");
}
