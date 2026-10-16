//! blame 파서와 줄 매핑의 골든 테스트: fixtures 저장소에서 git 결과와 비교한다.

mod common;

use std::collections::HashMap;
use std::path::Path;

use common::{commit_all, encoding, git, git_text, history, scratch_repo, ten_lines};
use plx_core::blame::{Blame, BlameCommit, is_uncommitted};
use plx_core::linemap::LineMap;
use plx_core::repo::Repo;

/// 한 줄의 blame 결과(비교할 필드만).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    sha: String,
    orig: u32,
    path: String,
    previous: Option<(String, String)>,
    author: String,
    /// 미커밋 줄은 실행 시각이라 비교하지 않는다(None).
    author_time: Option<i64>,
    summary: String,
    boundary: bool,
}

fn ours(repo: &Path, path: &str, rev: Option<&str>) -> Vec<Line> {
    let mut commits: HashMap<String, BlameCommit> = HashMap::new();
    let mut blocks = Vec::new();
    let blame = Blame::spawn(repo, path, rev).expect("git blame 실행");
    blame
        .stream(|chunk| {
            for commit in chunk.commits {
                assert!(
                    commits.insert(commit.sha.clone(), commit).is_none(),
                    "같은 커밋이 두 번 나왔다"
                );
            }
            blocks.extend(chunk.blocks);
        })
        .unwrap_or_else(|e| panic!("{path}: {e}"));

    let total: u32 = blocks.iter().map(|b| b.num_lines).sum();
    let mut lines: Vec<Option<Line>> = vec![None; total as usize];
    for block in blocks {
        let commit = &commits[&block.sha];
        for i in 0..block.num_lines {
            let slot = &mut lines[(block.final_line - 1 + i) as usize];
            assert!(
                slot.is_none(),
                "{path}: 줄 {}이 두 블록에 들어갔다",
                block.final_line + i
            );
            *slot = Some(Line {
                sha: block.sha.clone(),
                orig: block.orig_line + i,
                path: block.path.clone(),
                previous: block
                    .previous
                    .as_ref()
                    .map(|p| (p.sha.clone(), p.path.clone())),
                author: commit.author.clone(),
                author_time: (!is_uncommitted(&block.sha)).then_some(commit.author_time),
                summary: commit.summary.clone(),
                boundary: commit.boundary,
            });
        }
    }
    lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| l.unwrap_or_else(|| panic!("{path}: 줄 {}이 비었다", i + 1)))
        .collect()
}

/// `git blame --line-porcelain`: 줄마다 헤더를 모두 반복하는 형식.
fn reference(repo: &Path, path: &str, rev: Option<&str>) -> Vec<Line> {
    let mut args = vec!["blame", "--line-porcelain"];
    args.extend(rev);
    args.extend(["--", path]);
    let out = git(repo, &args);
    let mut lines = Vec::new();
    let mut current: Option<Line> = None;
    for raw in out.split(|&b| b == b'\n') {
        if raw.first() == Some(&b'\t') {
            lines.push(current.take().expect("내용 줄 앞에 머리 줄이 없다"));
            continue;
        }
        if raw.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(raw);
        let Some(line) = current.as_mut() else {
            let mut fields = text.split(' ');
            let sha = fields.next().unwrap().to_owned();
            let orig = fields.next().unwrap().parse().unwrap();
            current = Some(Line {
                sha,
                orig,
                path: String::new(),
                previous: None,
                author: String::new(),
                author_time: None,
                summary: String::new(),
                boundary: false,
            });
            continue;
        };
        let (key, value) = text.split_once(' ').unwrap_or((&text, ""));
        match key {
            "author" => line.author = value.to_owned(),
            "author-time" if !is_uncommitted(&line.sha) => {
                line.author_time = Some(value.parse().unwrap())
            }
            "summary" => line.summary = value.to_owned(),
            "boundary" => line.boundary = true,
            "previous" => {
                let (sha, path) = value.split_once(' ').unwrap();
                line.previous = Some((sha.to_owned(), path.to_owned()));
            }
            "filename" => line.path = value.to_owned(),
            _ => {}
        }
    }
    lines
}

fn compare(repo: &Path, path: &str, rev: Option<&str>) -> Vec<Line> {
    let ours = ours(repo, path, rev);
    let expected = reference(repo, path, rev);
    assert_eq!(ours.len(), expected.len(), "{path} {rev:?}: 줄 수");
    for (i, (a, b)) in ours.iter().zip(&expected).enumerate() {
        assert_eq!(a, b, "{path} {rev:?}: {}번째 줄", i + 1);
    }
    ours
}

#[test]
fn history_blame_matches_git() {
    let repo = history();
    for path in [
        "src/lib.rs",
        "src/new_name.rs",
        "core/util.rs",
        "src/moves.rs",
        "src/target.rs",
        "src/conflict.rs",
        "src/feature.rs",
        "docs/notes.md",
        "src/no_eol.txt",
        "empty.txt",
    ] {
        compare(&repo, path, None);
        compare(&repo, path, Some("HEAD"));
    }
    compare(&repo, "src/lib.rs", Some("v0.1"));
    compare(&repo, "src/topic.rs", Some("topic"));
}

#[test]
fn working_tree_blame_marks_uncommitted_lines() {
    let lines = compare(&history(), "src/lib.rs", None);
    // 미커밋 변경: 4번째 줄 수정, 끝에 한 줄 추가
    assert!(is_uncommitted(&lines[3].sha), "{:?}", lines[3]);
    assert!(is_uncommitted(&lines.last().unwrap().sha));
    assert!(!is_uncommitted(&lines[0].sha));
    // merge에서만 생긴 줄은 merge 커밋을 가리킨다
    assert_eq!(lines[10].summary, "Merge branch feature");
}

#[test]
fn rename_is_followed_with_previous_path() {
    let lines = compare(&history(), "src/new_name.rs", Some("HEAD"));
    // 2번째 줄은 rename 전 커밋(initial import)에서 왔고, 그때 경로는 old_name.rs다
    assert_eq!(lines[1].summary, "initial import");
    assert_eq!(lines[1].path, "src/old_name.rs");
    // 1번째 줄은 rename 커밋에서 바뀌었고, 부모 리비전의 경로는 old_name.rs다
    assert_eq!(lines[0].summary, "rename old_name.rs to new_name.rs");
    assert_eq!(
        lines[0].previous.as_ref().map(|p| p.1.as_str()),
        Some("src/old_name.rs")
    );
}

#[test]
fn encoding_blame_matches_git() {
    let repo = encoding();
    for path in [
        "한글/파일 이름.txt",
        "문서/설명.md",
        "이모지-😀.txt",
        "crlf.txt",
        "lf-to-crlf.txt",
        "mixed.txt",
        "euc-kr.txt",
    ] {
        compare(&repo, path, None);
    }
    let lines = compare(&repo, "euc-kr.txt", None);
    // git이 커밋 메시지를 UTF-8로 바꿔 내보낸다
    assert_eq!(lines[1].summary, "인코딩 테스트: EUC-KR 커밋 메시지");
    let lines = compare(&repo, "한글/파일 이름.txt", None);
    assert_eq!(lines[1].author, "김개발");
}

#[test]
fn missing_path_is_a_git_error() {
    let blame = Blame::spawn(&history(), "no/such/file.rs", None).unwrap();
    let error = blame.stream(|_| {}).unwrap_err().to_string();
    assert!(error.contains("git blame 실패"), "{error}");
}

#[test]
fn cancel_stops_the_stream() {
    let blame = Blame::spawn(&history(), "src/lib.rs", Some("HEAD")).unwrap();
    blame.canceller().cancel();
    let result = blame.stream(|_| {});
    assert!(
        matches!(result, Err(plx_core::blame::BlameError::Cancelled) | Ok(_)),
        "{result:?}"
    );
}

// ---------------------------------------------------------------- 줄 매핑

/// `git diff -U0`의 hunk 머리 줄로 만든 기준 매핑.
fn git_line_map(repo: &Path, before: &str, after: &str, before_lines: u32) -> LineMap {
    // 내용 줄은 UTF-8이 아닐 수 있다(EUC-KR 파일). hunk 머리 줄만 쓴다.
    let out = String::from_utf8_lossy(&git(
        repo,
        &[
            "diff",
            "-U0",
            "--no-color",
            "--no-ext-diff",
            "--diff-algorithm=myers",
            before,
            after,
        ],
    ))
    .into_owned();
    let range = |spec: &str| -> std::ops::Range<u32> {
        let (start, count) = match spec.split_once(',') {
            Some((s, c)) => (s.parse::<u32>().unwrap(), c.parse::<u32>().unwrap()),
            None => (spec.parse().unwrap(), 1),
        };
        // 개수가 0이면 시작 값은 "그 앞 줄"이다.
        if count == 0 {
            start..start
        } else {
            start - 1..start - 1 + count
        }
    };
    let hunks = out
        .lines()
        .filter_map(|l| l.strip_prefix("@@ -"))
        .map(|l| {
            let mut parts = l.split(' ');
            let old = parts.next().unwrap();
            let new = parts.next().unwrap().trim_start_matches('+');
            (range(old), range(new))
        })
        .collect();
    LineMap::from_hunks(hunks, before_lines)
}

fn line_count(data: &[u8]) -> u32 {
    let newlines = data.iter().filter(|&&b| b == b'\n').count() as u32;
    newlines + u32::from(!data.is_empty() && !data.ends_with(b"\n"))
}

fn check_map(repo_dir: &Path, repo: &Repo, before: (&str, &str), after: (&str, &str)) -> usize {
    let blob = |(rev, path): (&str, &str)| {
        let id = repo.resolve_commit(rev).unwrap();
        repo.blob_at(id, path).unwrap()
    };
    let (Some(before_data), Some(after_data)) = (blob(before), blob(after)) else {
        return 0;
    };
    if before_data == after_data {
        return 0;
    }
    let ours = LineMap::new(&before_data, &after_data);
    let reference = git_line_map(
        repo_dir,
        &format!("{}:{}", before.0, before.1),
        &format!("{}:{}", after.0, after.1),
        line_count(&before_data),
    );
    for line in 1..=line_count(&after_data) {
        assert_eq!(
            ours.to_before(line),
            reference.to_before(line),
            "{before:?} → {after:?}, {line}번째 줄"
        );
    }
    1
}

#[test]
fn line_map_matches_git_diff_for_every_parent() {
    let dir = history();
    let repo = Repo::discover(&dir).unwrap();
    let commits = git_text(&dir, &["rev-list", "--all", "--parents"]);
    let mut compared = 0;
    for line in commits.lines() {
        let mut ids = line.split(' ');
        let child = ids.next().unwrap();
        for parent in ids {
            for path in [
                "src/lib.rs",
                "src/moves.rs",
                "src/conflict.rs",
                "src/new_name.rs",
                "docs/notes.md",
                "src/topic.rs",
            ] {
                compared += check_map(&dir, &repo, (parent, path), (child, path));
            }
        }
    }
    assert!(compared >= 10, "비교한 쌍이 너무 적다: {compared}");
}

#[test]
fn line_map_matches_git_diff_for_blame_previous() {
    // `,`(부모 리비전 blame)가 실제로 쓰는 쌍: 블록의 (커밋, 경로) → (previous 커밋, previous 경로)
    let mut compared = 0;
    for (dir, path) in [
        (history(), "src/lib.rs"),
        (history(), "src/new_name.rs"),
        (history(), "core/util.rs"),
        (history(), "src/conflict.rs"),
        (encoding(), "lf-to-crlf.txt"),
        (encoding(), "문서/설명.md"),
        (encoding(), "euc-kr.txt"),
    ] {
        let repo = Repo::discover(&dir).unwrap();
        for line in ours(&dir, path, Some("HEAD")) {
            if let Some((prev_sha, prev_path)) = &line.previous {
                compared += check_map(&dir, &repo, (prev_sha, prev_path), (&line.sha, &line.path));
            }
        }
    }
    // 문서/설명.md·core/util.rs처럼 이름만 바뀐 파일은 previous가 없다. 나머지에서 충분히 비교했는지 본다.
    assert!(compared >= 4, "비교한 쌍이 너무 적다: {compared}");
}

// ---------------------------------------------------------------- 세션(앱의 blame 요청)

use plx_core::blame_session::{BlameMeta, SessionEvent, Sessions};
use plx_core::repo::RepoCache;
use plx_core::text::Encoding;

/// 요청 하나의 이벤트 전부. 이벤트 순서 규칙(`meta` → `chunk`* → 끝 이벤트 하나)도 확인한다.
fn events_with(
    sessions: &Sessions,
    id: u64,
    path: &Path,
    rev: Option<&str>,
    mut on_event: impl FnMut(&SessionEvent) -> bool,
) -> Vec<SessionEvent> {
    let repos = RepoCache::new();
    let mut events = Vec::new();
    sessions.run(id, &repos, path, rev, |event| {
        let keep = on_event(&event);
        events.push(event);
        keep
    });
    assert_eq!(sessions.active(), 0, "끝난 요청은 지운다");
    let ends = events.iter().filter(|e| e.is_end()).count();
    assert_eq!(ends, 1, "끝 이벤트는 하나: {events:?}");
    assert!(
        events.last().unwrap().is_end(),
        "끝 이벤트가 마지막: {events:?}"
    );
    let metas: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, SessionEvent::Meta(_)))
        .map(|(i, _)| i)
        .collect();
    assert!(
        metas.is_empty() || metas == [0],
        "meta는 많아야 하나, 맨 앞: {events:?}"
    );
    events
}

fn events(path: &Path, rev: Option<&str>) -> Vec<SessionEvent> {
    let sessions = Sessions::new();
    let id = sessions.register();
    events_with(&sessions, id, path, rev, |_| true)
}

/// 성공한 요청의 내용과 blame 줄 수.
fn session(path: &Path, rev: Option<&str>) -> (BlameMeta, u32) {
    let events = events(path, rev);
    let SessionEvent::Meta(meta) = &events[0] else {
        panic!("{}: {events:?}", path.display());
    };
    let SessionEvent::Done { stats, .. } = events.last().unwrap() else {
        panic!("{}: {events:?}", path.display());
    };
    let lines = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Chunk(chunk) => {
                Some(chunk.blocks.iter().map(|b| b.num_lines).sum::<u32>())
            }
            _ => None,
        })
        .sum();
    let blocks: usize = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Chunk(chunk) => Some(chunk.blocks.len()),
            _ => None,
        })
        .sum();
    assert_eq!(stats.blocks, blocks);
    (meta.clone(), lines)
}

fn error_of(events: &[SessionEvent]) -> &str {
    match events.last().unwrap() {
        SessionEvent::Error { message } => message,
        other => panic!("실패여야 한다: {other:?}"),
    }
}

#[test]
fn session_working_tree_and_revision() {
    let dir = history();
    let (meta, lines) = session(&dir.join("src/lib.rs"), None);
    assert_eq!(meta.path, "src/lib.rs");
    assert_eq!(meta.rev, None);
    let content = meta.content.unwrap();
    assert_eq!(
        content.line_count, lines,
        "blame 줄 수와 내용 줄 수가 같아야 한다"
    );
    assert!(
        content
            .text
            .contains("uncommitted edit in the working tree")
    );

    let (meta, lines) = session(&dir.join("src/lib.rs"), Some("v0.1"));
    assert_eq!(
        meta.rev.as_deref(),
        Some(git_text(&dir, &["rev-parse", "v0.1^{commit}"]).as_str())
    );
    assert_eq!(meta.content.unwrap().line_count, lines);
}

#[test]
fn session_encodings_and_binary() {
    let (meta, _) = session(&encoding().join("euc-kr.txt"), None);
    let content = meta.content.unwrap();
    assert_eq!(content.encoding, Encoding::EucKr);
    assert!(content.text.starts_with("안녕하세요\n"));

    let (meta, _) = session(&encoding().join("한글/파일 이름.txt"), None);
    assert_eq!(meta.path, "한글/파일 이름.txt");
    assert_eq!(meta.content.unwrap().encoding, Encoding::Utf8);

    let (meta, lines) = session(&history().join("assets/logo.bin"), None);
    assert!(meta.binary && meta.content.is_none());
    assert_eq!(lines, 0);
}

#[test]
fn session_errors() {
    let missing = events(&history().join("no/such.rs"), None);
    let error = error_of(&missing);
    assert!(
        error.contains("no/such.rs") || error.contains("파일"),
        "{error}"
    );
    assert_eq!(missing.len(), 1, "내용을 못 읽으면 meta도 없다");

    let bad_rev = events(&history().join("src/lib.rs"), Some("no-such-rev"));
    assert!(error_of(&bad_rev).contains("no-such-rev"));

    // 그 리비전에 없는 파일
    let not_there = events(&history().join("src/new_name.rs"), Some("v0.1"));
    assert!(error_of(&not_there).contains("src/new_name.rs"));
}

#[test]
fn session_file_not_in_head_is_all_uncommitted() {
    // 추적하지 않는 파일(git blame은 "no such path in HEAD"로 실패한다): 모든 줄이 커밋 안 됨
    let (meta, lines) = session(&history().join("untracked.txt"), None);
    let content = meta.content.unwrap();
    assert!(content.line_count > 0);
    assert_eq!(lines, content.line_count);
    let events = events(&history().join("untracked.txt"), None);
    let SessionEvent::Chunk(chunk) = &events[1] else {
        panic!("{events:?}");
    };
    assert!(chunk.blocks.iter().all(|b| is_uncommitted(&b.sha)));
    assert!(chunk.commits.iter().all(|c| is_uncommitted(&c.sha)));

    // 커밋이 없는 저장소의 새 파일, 빈 새 파일
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("unborn-repo");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("새 파일.txt"), "하나\n둘\n").unwrap();
    std::fs::write(dir.join("empty.txt"), "").unwrap();
    let (meta, lines) = session(&dir.join("새 파일.txt"), None);
    assert_eq!((meta.path.as_str(), lines), ("새 파일.txt", 2));
    let (_, lines) = session(&dir.join("empty.txt"), None);
    assert_eq!(lines, 0);
}

#[test]
fn session_cancel_before_start() {
    let sessions = Sessions::new();
    let id = sessions.register();
    assert!(sessions.cancel(id));
    let events = events_with(&sessions, id, &history().join("src/lib.rs"), None, |_| true);
    assert!(
        matches!(events[..], [SessionEvent::Cancelled]),
        "{events:?}"
    );
    assert!(!sessions.cancel(id), "끝난 요청은 취소할 것이 없다");
    assert!(!sessions.cancel(9999), "모르는 id");
}

#[test]
fn session_cancel_while_streaming() {
    let sessions = Sessions::new();
    let id = sessions.register();
    let path = history().join("src/lib.rs");
    let events = events_with(&sessions, id, &path, Some("HEAD"), |event| {
        if matches!(event, SessionEvent::Meta(_)) {
            assert!(sessions.cancel(id));
        }
        true
    });
    assert!(matches!(events[0], SessionEvent::Meta(_)));
    assert!(
        matches!(events.last(), Some(SessionEvent::Cancelled)),
        "{events:?}"
    );
}

#[test]
fn session_stops_when_the_receiver_is_gone() {
    // meta를 받을 곳이 없다
    let sessions = Sessions::new();
    let id = sessions.register();
    let path = history().join("src/lib.rs");
    let events = events_with(&sessions, id, &path, None, |_| false);
    assert!(
        matches!(events.last(), Some(SessionEvent::Cancelled)),
        "{events:?}"
    );

    // 첫 묶음을 받은 뒤 사라졌다
    let id = sessions.register();
    let events = events_with(&sessions, id, &path, None, |event| {
        !matches!(event, SessionEvent::Chunk(_))
    });
    assert!(
        matches!(events.last(), Some(SessionEvent::Cancelled)),
        "{events:?}"
    );
}

/// 한 번에 다 있는 결과(캐시, 모두 미커밋)도 받는 쪽이 meta에서 사라졌으면 취소로 끝난다. git 결과가 묶음 하나로
/// 다 온 뒤에 사라진 경우(바쁜 머신에서 첫 묶음 전에 git이 끝났다)와 같은 길이다.
#[test]
fn whole_results_end_cancelled_when_the_receiver_is_gone() {
    let repo = scratch_repo("session-gone");
    std::fs::write(repo.join("a.txt"), ten_lines(&[])).unwrap();
    commit_all(&repo, "first");
    std::fs::write(repo.join("new.txt"), "untracked\n").unwrap();
    let sessions = Sessions::new();
    let id = sessions.register();
    let events = events_with(&sessions, id, &repo.join("new.txt"), None, |_| false);
    assert!(
        matches!(events[..], [SessionEvent::Meta(_), SessionEvent::Cancelled]),
        "{events:?}"
    );

    let dir = tempfile::tempdir().unwrap();
    let sessions =
        Sessions::with_cache(Arc::new(DiskCache::new(dir.path().to_path_buf(), 1 << 30)));
    let file = repo.join("a.txt");
    let (_, cached) = cached_session(&sessions, &file, Some("HEAD"));
    assert!(!cached);
    let id = sessions.register();
    let events = events_with(&sessions, id, &file, Some("HEAD"), |_| false);
    assert!(
        matches!(events[..], [SessionEvent::Meta(_), SessionEvent::Cancelled]),
        "{events:?}"
    );
}

#[test]
fn session_events_serialize_for_the_ui() {
    let events = events(&history().join("src/lib.rs"), None);
    let json: Vec<serde_json::Value> = events
        .iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
    assert_eq!(json[0]["kind"], "meta");
    assert_eq!(json[0]["path"], "src/lib.rs");
    assert_eq!(json[0]["content"]["encoding"], "UTF-8");
    assert!(json[0]["content"]["lineCount"].is_u64());
    assert_eq!(json[1]["kind"], "chunk");
    assert!(json[1]["blocks"][0]["finalLine"].is_u64());
    assert!(json[1]["commits"][0]["authorTime"].is_i64());
    let done = json.last().unwrap();
    assert_eq!(done["kind"], "done");
    assert!(done["metaMs"].is_f64());
    assert!(done["stats"]["totalMs"].is_f64());
    assert_eq!(
        serde_json::to_value(SessionEvent::Error {
            message: "x".into()
        })
        .unwrap(),
        serde_json::json!({"kind": "error", "message": "x"})
    );
    assert_eq!(
        serde_json::to_value(SessionEvent::Cancelled).unwrap(),
        serde_json::json!({"kind": "cancelled"})
    );
}

// ---------------------------------------------------------------- 캐시

use plx_core::blame::Chunk;
use plx_core::cache::DiskCache;
use std::sync::Arc;

/// 요청 하나의 (줄별 결과, 캐시에서 읽었는가).
fn cached_session(
    sessions: &Sessions,
    path: &Path,
    rev: Option<&str>,
) -> (Vec<(String, u32)>, bool) {
    let id = sessions.register();
    let events = events_with(sessions, id, path, rev, |_| true);
    let mut all = Chunk::default();
    let mut cached = None;
    for event in events {
        match event {
            SessionEvent::Chunk(chunk) => all.blocks.extend(chunk.blocks),
            SessionEvent::Done { stats, .. } => cached = Some(stats.cached),
            SessionEvent::Error { message } => panic!("{}: {message}", path.display()),
            _ => {}
        }
    }
    let mut lines: Vec<(u32, String, u32)> = all
        .blocks
        .iter()
        .flat_map(|b| {
            (0..b.num_lines).map(move |i| (b.final_line + i, b.sha.clone(), b.orig_line + i))
        })
        .collect();
    lines.sort();
    (
        lines
            .into_iter()
            .map(|(_, sha, orig)| (sha, orig))
            .collect(),
        cached.expect("done"),
    )
}

#[test]
fn commit_blame_is_cached() {
    let dir = tempfile::tempdir().unwrap();
    let sessions =
        Sessions::with_cache(Arc::new(DiskCache::new(dir.path().to_path_buf(), 1 << 30)));
    let file = history().join("src/lib.rs");
    let (first, cached) = cached_session(&sessions, &file, Some("HEAD~1"));
    assert!(!cached);
    let (second, cached) = cached_session(&sessions, &file, Some("HEAD~1"));
    assert!(cached, "두 번째는 캐시에서");
    assert_eq!(first, second);
    // 다른 리비전은 다른 결과
    let (_, cached) = cached_session(&sessions, &file, Some("HEAD~2"));
    assert!(!cached);
    // 바이너리·없는 파일은 넣지 않는다
    let (_, cached) = cached_session(&sessions, &history().join("assets/logo.bin"), Some("HEAD"));
    assert!(!cached);
}

#[test]
fn worktree_blame_cache_follows_the_file() {
    // 수정 시각을 직접 정하므로 공유 fixtures 대신 저장소를 따로 만든다.
    let repo = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cache-worktree");
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    let file = repo.join("파일.txt");
    std::fs::write(&file, "하나\n둘\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "commit",
            "-q",
            "-m",
            "first",
        ],
    );
    let modified = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let set_mtime = || {
        let f = std::fs::File::options().write(true).open(&file).unwrap();
        f.set_modified(modified).unwrap();
    };
    std::fs::write(&file, "하나\n둘\n셋\n").unwrap();
    set_mtime();

    let cache_dir = tempfile::tempdir().unwrap();
    let sessions = Sessions::with_cache(Arc::new(DiskCache::new(
        cache_dir.path().to_path_buf(),
        1 << 30,
    )));
    let (first, cached) = cached_session(&sessions, &file, None);
    assert!(!cached);
    let uncommitted =
        |lines: &[(String, u32)]| lines.iter().filter(|(sha, _)| is_uncommitted(sha)).count();
    assert_eq!(uncommitted(&first), 1);
    let (second, cached) = cached_session(&sessions, &file, None);
    assert!(cached);
    assert_eq!(first, second);

    // 열쇠는 내용이다: 크기와 수정 시각이 그대로인 채 내용이 바뀌어도(같은 초 안에 다시 고침) 다시 계산한다.
    std::fs::write(&file, "하나\n넷\n셋\n").unwrap();
    set_mtime();
    let (third, cached) = cached_session(&sessions, &file, None);
    assert!(!cached, "크기·수정 시각이 같아도 내용이 다르면 다른 열쇠");
    assert_eq!(uncommitted(&third), 2);
    // 막 고친 파일도 넣는다(git에 준 내용이 열쇠라 낡은 결과가 될 수 없다).
    std::fs::write(&file, "하나\n둘\n셋\n넷\n").unwrap();
    let (fourth, cached) = cached_session(&sessions, &file, None);
    assert!(!cached);
    assert_eq!(fourth.len(), 4);
    let (fifth, cached) = cached_session(&sessions, &file, None);
    assert!(cached);
    assert_eq!(fourth, fifth);
}

// ---------------------------------------------------------------- 리뷰에서 찾은 경우(회귀 테스트)

use plx_core::repo::FilePath;

/// 줄마다 (sha, 원래 줄)과 캐시에서 왔는지, 세션이 blame한 경로. `repos`를 나눠 쓰면 앱처럼 저장소를 열어 둔다.
fn lines_of(
    sessions: &Sessions,
    repos: &RepoCache,
    file: FilePath,
    rev: Option<&str>,
) -> (Vec<(String, u32)>, bool, String) {
    let id = sessions.register();
    let mut blocks = Vec::new();
    let mut cached = false;
    let mut path = String::new();
    sessions.run(id, repos, file, rev, |event| {
        match event {
            SessionEvent::Meta(meta) => path = meta.path,
            SessionEvent::Chunk(chunk) => blocks.extend(chunk.blocks),
            SessionEvent::Done { stats, .. } => cached = stats.cached,
            SessionEvent::Error { message } => panic!("{message}"),
            SessionEvent::Cancelled => panic!("취소됐다"),
        }
        true
    });
    let mut lines: Vec<(u32, String, u32)> = blocks
        .iter()
        .flat_map(|b| {
            (0..b.num_lines).map(move |i| (b.final_line + i, b.sha.clone(), b.orig_line + i))
        })
        .collect();
    lines.sort();
    (
        lines
            .into_iter()
            .map(|(_, sha, orig)| (sha, orig))
            .collect(),
        cached,
        path,
    )
}

fn in_repo(dir: &Path, path: &str) -> FilePath {
    FilePath::InRepo {
        workdir: dir.to_path_buf(),
        path: path.to_owned(),
    }
}

fn cache_in(dir: &Path) -> Sessions {
    Sessions::with_cache(Arc::new(DiskCache::new(dir.to_path_buf(), 1 << 30)))
}

/// `git mv`만 하고 커밋하지 않은 파일: git blame은 옮기기 전 이력을 찾는다. 모두 미커밋으로 만들면 안 된다.
#[test]
fn staged_rename_keeps_history() {
    let dir = scratch_repo("staged-rename");
    std::fs::write(dir.join("a.txt"), ten_lines(&[])).unwrap();
    let c1 = commit_all(&dir, "c1");
    std::fs::write(dir.join("a.txt"), ten_lines(&[2])).unwrap();
    let c2 = commit_all(&dir, "c2");
    git(&dir, &["mv", "a.txt", "b.txt"]);
    let (lines, _, _) = lines_of(
        &Sessions::new(),
        &RepoCache::new(),
        dir.join("b.txt").into(),
        None,
    );
    assert_eq!(lines.len(), 10);
    assert!(
        lines.iter().all(|(sha, _)| !is_uncommitted(sha)),
        "{lines:?}"
    );
    assert_eq!(lines[1].0, c2);
    assert_eq!(lines[0].0, c1);

    // 새로 add만 한 파일은 git도 모두 미커밋이다.
    std::fs::write(dir.join("new.txt"), "x\ny\n").unwrap();
    git(&dir, &["add", "new.txt"]);
    let (lines, _, _) = lines_of(
        &Sessions::new(),
        &RepoCache::new(),
        dir.join("new.txt").into(),
        None,
    );
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|(sha, _)| is_uncommitted(sha)));
}

/// 사용자의 `i18n.logOutputEncoding`(예: cp949)과 관계없이 작성자·요약을 UTF-8로 받는다.
#[test]
fn log_output_encoding_does_not_garble_blame() {
    let dir = scratch_repo("output-encoding");
    git(&dir, &["config", "user.name", "김개발"]);
    std::fs::write(dir.join("f.txt"), "x\n").unwrap();
    std::fs::write(dir.join(".msg"), "한글 요약\n").unwrap();
    git(&dir, &["add", "f.txt"]);
    git(&dir, &["commit", "-q", "-F", ".msg"]);
    git(&dir, &["config", "i18n.logOutputEncoding", "euc-kr"]);
    let mut commits = Vec::new();
    Blame::spawn(&dir, "f.txt", None)
        .unwrap()
        .stream(|chunk| commits.extend(chunk.commits))
        .unwrap();
    assert_eq!(commits[0].author, "김개발");
    assert_eq!(commits[0].summary, "한글 요약");
}

/// 얕은 클론은 경계 너머 이력을 모른다. 이력을 더 받으면(`fetch --unshallow`) 결과가 바뀌므로 캐시에 넣지 않는다.
#[test]
fn shallow_clone_results_are_not_cached() {
    let src = scratch_repo("shallow-source");
    std::fs::write(src.join("f.txt"), ten_lines(&[])).unwrap();
    commit_all(&src, "c1");
    std::fs::write(src.join("f.txt"), ten_lines(&[3])).unwrap();
    commit_all(&src, "c2");
    std::fs::write(src.join("f.txt"), ten_lines(&[3, 5])).unwrap();
    let c3 = commit_all(&src, "c3");
    let shallow = Path::new(env!("CARGO_TARGET_TMPDIR")).join("scratch-shallow-clone");
    let _ = std::fs::remove_dir_all(&shallow);
    let url = format!("file://{}", src.to_string_lossy().replace('\\', "/"));
    git(
        &src,
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            &url,
            &shallow.to_string_lossy(),
        ],
    );

    let cache = tempfile::tempdir().unwrap();
    let sessions = cache_in(cache.path());
    let repos = RepoCache::new();
    let distinct = |lines: &[(String, u32)]| {
        let mut shas: Vec<&String> = lines.iter().map(|(sha, _)| sha).collect();
        shas.sort();
        shas.dedup();
        shas.len()
    };
    for rev in [Some(c3.as_str()), None] {
        let (before, cached, _) = lines_of(&sessions, &repos, shallow.join("f.txt").into(), rev);
        assert!(!cached);
        assert_eq!(distinct(&before), 1, "경계 커밋 하나");
        let (_, cached, _) = lines_of(&sessions, &repos, shallow.join("f.txt").into(), rev);
        assert!(!cached, "얕은 클론은 넣지 않는다");
    }
    git(&shallow, &["fetch", "-q", "--unshallow"]);
    for rev in [Some(c3.as_str()), None] {
        let (after, _, _) = lines_of(&sessions, &repos, shallow.join("f.txt").into(), rev);
        assert_eq!(distinct(&after), 3, "이력을 다 받으면 커밋 셋");
    }
}

/// `blame.ignoreRevsFile`을 켜거나 그 파일에 커밋을 더하면 캐시에 있던 결과를 쓰지 않는다.
#[test]
fn ignore_revs_change_the_cache_key() {
    let dir = scratch_repo("ignore-revs");
    std::fs::write(dir.join("f.txt"), ten_lines(&[])).unwrap();
    let c1 = commit_all(&dir, "c1");
    std::fs::write(dir.join("f.txt"), ten_lines(&[3])).unwrap();
    let c2 = commit_all(&dir, "c2: 포맷만 바꿈");
    let cache = tempfile::tempdir().unwrap();
    let sessions = cache_in(cache.path());
    let repos = RepoCache::new();
    let file = || in_repo(&dir, "f.txt");

    let (plain, _, _) = lines_of(&sessions, &repos, file(), Some(&c2));
    assert_eq!(plain[2].0, c2);
    let (_, cached, _) = lines_of(&sessions, &repos, file(), Some(&c2));
    assert!(cached);

    // 무시할 커밋 목록을 켠다: 저장소를 열어 둔 채로(앱처럼) 설정이 바뀐다.
    std::fs::write(dir.join(".git-blame-ignore-revs"), "").unwrap();
    git(
        &dir,
        &["config", "blame.ignoreRevsFile", ".git-blame-ignore-revs"],
    );
    let (_, cached, _) = lines_of(&sessions, &repos, file(), Some(&c2));
    assert!(!cached, "설정이 바뀌면 다른 열쇠");
    // 목록에 커밋을 더한다(설정은 그대로)
    std::fs::write(dir.join(".git-blame-ignore-revs"), format!("{c2}\n")).unwrap();
    let (ignored, cached, _) = lines_of(&sessions, &repos, file(), Some(&c2));
    assert!(!cached, "목록 파일이 바뀌면 다른 열쇠");
    assert_eq!(ignored[2].0, c1, "무시한 커밋 대신 그 앞 커밋");
    let (_, cached, _) = lines_of(&sessions, &repos, file(), Some(&c2));
    assert!(cached);
}

/// 작업트리 결과는 줄 끝 설정도 열쇠에 든다: `core.autocrlf`를 바꾸면 git이 보는 내용이 달라진다.
#[test]
fn autocrlf_change_changes_the_worktree_key() {
    let dir = scratch_repo("autocrlf-cache");
    git(&dir, &["config", "core.autocrlf", "true"]);
    std::fs::write(dir.join("f.txt"), ten_lines(&[])).unwrap();
    commit_all(&dir, "c1");
    std::fs::remove_file(dir.join("f.txt")).unwrap();
    git(&dir, &["checkout", "--", "f.txt"]);
    git(&dir, &["config", "core.autocrlf", "false"]);
    let cache = tempfile::tempdir().unwrap();
    let sessions = cache_in(cache.path());
    let repos = RepoCache::new();
    let uncommitted =
        |lines: &[(String, u32)]| lines.iter().filter(|(sha, _)| is_uncommitted(sha)).count();

    let (crlf, _, _) = lines_of(&sessions, &repos, dir.join("f.txt").into(), None);
    assert_eq!(
        uncommitted(&crlf),
        10,
        "autocrlf=false면 CRLF 줄은 모두 바뀐 줄"
    );
    git(&dir, &["config", "core.autocrlf", "true"]);
    let (converted, cached, _) = lines_of(&sessions, &repos, dir.join("f.txt").into(), None);
    assert!(!cached);
    assert_eq!(
        uncommitted(&converted),
        0,
        "autocrlf=true면 바뀐 줄이 없다(git과 같다)"
    );
}

/// 리비전의 경로는 그 리비전의 경로다: 오늘의 작업트리(심볼릭 링크)로 풀지 않는다.
#[cfg(unix)]
#[test]
fn revision_path_is_not_resolved_through_the_worktree() {
    let dir = scratch_repo("revision-symlink");
    std::fs::write(dir.join("alias.txt"), "alias at c1\n").unwrap();
    std::fs::write(dir.join("real.txt"), "real at c1\n").unwrap();
    let c1 = commit_all(&dir, "c1");
    std::fs::remove_file(dir.join("alias.txt")).unwrap();
    std::os::unix::fs::symlink("real.txt", dir.join("alias.txt")).unwrap();
    commit_all(&dir, "c2: alias는 심볼릭 링크");
    let (_, _, path) = lines_of(
        &Sessions::new(),
        &RepoCache::new(),
        in_repo(&dir, "alias.txt"),
        Some(&c1),
    );
    assert_eq!(path, "alias.txt");
    // 작업트리의 심볼릭 링크는 가리키는 파일을 blame한다(터미널에서 준 경로와 같다).
    let (_, _, path) = lines_of(
        &Sessions::new(),
        &RepoCache::new(),
        in_repo(&dir, "alias.txt"),
        None,
    );
    assert_eq!(path, "real.txt");
    let (_, _, path) = lines_of(
        &Sessions::new(),
        &RepoCache::new(),
        dir.join("alias.txt").into(),
        None,
    );
    assert_eq!(path, "real.txt");
}

/// 화면이 준 저장소 기준 경로는 저장소 밖을 가리킬 수 없다.
#[test]
fn repo_relative_paths_stay_inside() {
    let dir = scratch_repo("relative-inside");
    std::fs::write(dir.join("f.txt"), "x\n").unwrap();
    commit_all(&dir, "c1");
    for bad in ["../f.txt", "a/../../f.txt", "/etc/passwd", "a//b"] {
        let events = {
            let sessions = Sessions::new();
            let id = sessions.register();
            let mut events = Vec::new();
            sessions.run(id, &RepoCache::new(), in_repo(&dir, bad), None, |e| {
                events.push(e);
                true
            });
            events
        };
        assert!(
            matches!(events.last(), Some(SessionEvent::Error { message }) if message.contains("저장소 기준 경로가 아니다")),
            "{bad}: {events:?}"
        );
    }
}
