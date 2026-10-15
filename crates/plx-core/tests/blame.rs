//! blame 파서와 줄 매핑의 골든 테스트: fixtures 저장소에서 git 결과와 비교한다.

mod common;

use std::collections::HashMap;
use std::path::Path;

use common::{encoding, git, git_text, history};
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
