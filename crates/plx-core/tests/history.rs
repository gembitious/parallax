//! 파일 히스토리를 fixtures에서 git 결과와 대조한다.
//! - 파서: `-z` 없이 다른 형식으로 받은 `git log` 출력과 커밋·필드 단위로 같아야 한다.
//! - 구간 잇기: rename으로 생긴 파일은 옛 이름의 이력이 뒤에 붙는다.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{encoding, git, git_text, history};
use plx_core::cache::DiskCache;
use plx_core::commit::FileStatus;
use plx_core::history::{HistoryEvent, HistoryMeta, HistorySessions, HistoryStats};
use plx_core::log::{LogEntry, LogStream};
use plx_core::repo::RepoCache;

// ---------------------------------------------------------------- 파서

/// git이 다른 형식(`-z` 없음, 줄 단위)으로 낸 같은 이력. 경로의 상태는 그 경로 줄에서 읽는다.
fn git_log(repo: &Path, starts: &[&str], path: &str) -> Vec<LogEntry> {
    let mut args = vec![
        "log",
        "--format=%x02%H%n%P%n%aN%n%aE%n%ad%n%s",
        "--date=raw",
        "--name-status",
        "--no-follow",
        "--encoding=UTF-8",
    ];
    args.extend_from_slice(starts);
    args.extend(["--", path]);
    let out = String::from_utf8(git(repo, &args)).expect("UTF-8");
    out.split('\x02')
        .filter(|commit| !commit.is_empty())
        .map(|commit| {
            let mut lines = commit.lines();
            let mut next = || lines.next().unwrap_or("").to_owned();
            let (sha, parents, author, mail, date, summary) =
                (next(), next(), next(), next(), next(), next());
            let (time, tz) = date.split_once(' ').unwrap();
            let mut entry = LogEntry {
                sha,
                parents: parents.split_whitespace().map(str::to_owned).collect(),
                author,
                author_mail: mail,
                author_time: time.parse().unwrap(),
                author_tz: tz.to_owned(),
                summary,
                status: None,
                path: path.to_owned(),
                old_path: None,
            };
            for line in lines {
                let mut fields = line.split('\t');
                let (Some(status), Some(first)) = (fields.next(), fields.next()) else {
                    continue;
                };
                let second = fields.next();
                if second.unwrap_or(first) == path && entry.status.is_none() {
                    entry.status = Some(match &status[..1] {
                        "A" => FileStatus::Added,
                        "D" => FileStatus::Deleted,
                        "M" => FileStatus::Modified,
                        "T" => FileStatus::TypeChanged,
                        "R" => FileStatus::Renamed,
                        "C" => FileStatus::Copied,
                        other => panic!("모르는 상태 {other}"),
                    });
                    entry.old_path = second.map(|_| first.to_owned());
                }
            }
            entry
        })
        .collect()
}

fn stream_all(repo: &Path, starts: &[&str], path: &str) -> Vec<LogEntry> {
    let starts: Vec<String> = starts.iter().map(|s| (*s).to_owned()).collect();
    let mut stream = LogStream::spawn(repo, &starts, path).expect("git log");
    let mut out = Vec::new();
    while stream.read(&mut out).expect("읽기") {}
    stream.finish().expect("git log 성공");
    out
}

/// 추적하는 모든 파일과 지운 파일, 디렉터리, 저장소 전체.
fn paths(repo: &Path) -> Vec<String> {
    let mut paths: Vec<String> = git_text(repo, &["ls-files", "-z"])
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    let deleted = git_text(
        repo,
        &[
            "log",
            "--diff-filter=D",
            "--name-only",
            "--format=",
            "-z",
            "--no-renames",
        ],
    );
    paths.extend(
        deleted
            .split('\0')
            .map(|p| p.trim_start_matches('\n'))
            .filter(|p| !p.is_empty())
            .map(str::to_owned),
    );
    for path in paths.clone() {
        if let Some((dir, _)) = path.rsplit_once('/') {
            paths.push(dir.to_owned());
        }
    }
    paths.push(".".to_owned());
    paths.sort();
    paths.dedup();
    paths
}

#[test]
fn parser_matches_git_for_every_path() {
    for repo in [history(), encoding()] {
        let mut checked = 0;
        for path in paths(&repo) {
            let expected = git_log(&repo, &["HEAD"], &path);
            assert!(!expected.is_empty(), "{path}: 이력이 있어야 한다");
            assert_eq!(stream_all(&repo, &["HEAD"], &path), expected, "{path}");
            checked += 1;
        }
        assert!(checked > 10, "{}: {checked}개만 확인했다", repo.display());
    }
}

#[test]
fn parser_matches_git_from_several_starts() {
    let repo = history();
    // 합치지 않은 브랜치와 main을 함께(합집합, 날짜 순)
    let expected = git_log(&repo, &["main", "topic"], ".");
    assert!(expected.iter().any(|e| e.summary == "topic: start"));
    assert_eq!(stream_all(&repo, &["main", "topic"], "."), expected);
}

#[test]
fn korean_messages_and_authors_are_utf8() {
    let repo = encoding();
    let entries = stream_all(&repo, &["HEAD"], "euc-kr.txt");
    let summaries: Vec<&str> = entries.iter().map(|e| e.summary.as_str()).collect();
    // EUC-KR로 인코딩된 커밋 메시지도 UTF-8로 받는다(encoding 헤더).
    assert!(
        summaries.contains(&"인코딩 테스트: EUC-KR 커밋 메시지"),
        "{summaries:?}"
    );
    let entries = stream_all(&repo, &["HEAD"], "한글/파일 이름.txt");
    assert!(entries.iter().any(|e| e.author == "김개발"), "{entries:?}");
}

#[test]
fn unknown_path_has_no_history_and_bad_start_fails() {
    let repo = history();
    assert_eq!(stream_all(&repo, &["HEAD"], "no/such/file.rs"), []);
    let starts = vec!["0123456789012345678901234567890123456789".to_owned()];
    let mut stream = LogStream::spawn(&repo, &starts, "src/lib.rs").unwrap();
    let mut out = Vec::new();
    while stream.read(&mut out).unwrap() {}
    let error = stream.finish().unwrap_err().to_string();
    assert!(error.starts_with("git log 실패: "), "{error}");
}

// ---------------------------------------------------------------- 요청

/// 요청 하나의 이벤트 전부. 순서 규칙(`meta` → `entries`* → 끝 이벤트 하나)도 확인한다.
fn events_with(
    sessions: &HistorySessions,
    path: &Path,
    rev: Option<&str>,
    mut on_event: impl FnMut(&HistoryEvent) -> bool,
) -> Vec<HistoryEvent> {
    let repos = RepoCache::new();
    let id = sessions.register();
    let mut events = Vec::new();
    sessions.run(id, &repos, path, rev, |event| {
        let keep = on_event(&event);
        events.push(event);
        keep
    });
    assert_eq!(sessions.active(), 0, "끝난 요청은 지운다");
    assert_eq!(
        events.iter().filter(|e| e.is_end()).count(),
        1,
        "{events:?}"
    );
    assert!(events.last().unwrap().is_end(), "{events:?}");
    let metas: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, HistoryEvent::Meta(_)))
        .map(|(i, _)| i)
        .collect();
    assert!(metas.is_empty() || metas == [0], "{events:?}");
    events
}

struct Run {
    meta: HistoryMeta,
    entries: Vec<LogEntry>,
    stats: HistoryStats,
}

fn run_with(sessions: &HistorySessions, path: &Path, rev: Option<&str>) -> Run {
    let events = events_with(sessions, path, rev, |_| true);
    let HistoryEvent::Meta(meta) = &events[0] else {
        panic!("{}: {events:?}", path.display());
    };
    let HistoryEvent::Done { stats, .. } = events.last().unwrap() else {
        panic!("{}: {events:?}", path.display());
    };
    let entries = events
        .iter()
        .filter_map(|e| match e {
            HistoryEvent::Entries { entries } => Some(entries.clone()),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(stats.entries, entries.len());
    Run {
        meta: meta.clone(),
        entries,
        stats: stats.clone(),
    }
}

fn run(path: &Path, rev: Option<&str>) -> Run {
    run_with(&HistorySessions::new(), path, rev)
}

fn shas(entries: &[LogEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.sha.as_str()).collect()
}

fn log_shas(repo: &Path, args: &[&str]) -> Vec<String> {
    let mut all = vec!["log", "--format=%H"];
    all.extend_from_slice(args);
    git_text(repo, &all).lines().map(str::to_owned).collect()
}

#[test]
fn rename_continues_with_the_old_name() {
    let dir = history();
    let result = run(&dir.join("src/new_name.rs"), None);
    assert_eq!(result.meta.path, "src/new_name.rs");
    assert_eq!(result.meta.rev, None);
    assert_eq!(
        result.meta.start.as_deref(),
        Some(git_text(&dir, &["rev-parse", "HEAD"]).as_str())
    );
    let entries = &result.entries;
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(entries[0].summary, "rename old_name.rs to new_name.rs");
    assert_eq!(entries[0].status, Some(FileStatus::Renamed));
    assert_eq!(entries[0].path, "src/new_name.rs");
    assert_eq!(entries[0].old_path.as_deref(), Some("src/old_name.rs"));
    assert_eq!(entries[1].summary, "initial import");
    assert_eq!(entries[1].status, Some(FileStatus::Added));
    assert_eq!(entries[1].path, "src/old_name.rs");
    assert_eq!(result.stats.segments, 2);
    // 직선 이력이라 --follow와 같다.
    assert_eq!(
        shas(entries),
        log_shas(&dir, &["--follow", "--", "src/new_name.rs"])
    );

    // 디렉터리 이동, 한글 경로 이동
    let moved = run(&dir.join("core/util.rs"), None).entries;
    assert_eq!(
        moved
            .iter()
            .map(|e| (e.status, e.path.as_str()))
            .collect::<Vec<_>>(),
        [
            (Some(FileStatus::Renamed), "core/util.rs"),
            (Some(FileStatus::Added), "lib/util.rs")
        ]
    );
    let korean = run(&encoding().join("문서/설명.md"), None).entries;
    assert_eq!(
        korean
            .iter()
            .map(|e| (e.status, e.path.as_str(), e.old_path.as_deref()))
            .collect::<Vec<_>>(),
        [
            (
                Some(FileStatus::Renamed),
                "문서/설명.md",
                Some("한글/설명.md")
            ),
            (Some(FileStatus::Added), "한글/설명.md", None)
        ]
    );
}

#[test]
fn history_without_renames_is_git_log() {
    let dir = history();
    // merge(merge에서만 생긴 줄이 있다)와 양쪽 브랜치의 커밋, 삭제 후 다시 추가
    for path in ["src/lib.rs", "docs/notes.md", "src/conflict.rs", "src"] {
        let result = run(&dir.join(path), None);
        assert_eq!(
            shas(&result.entries),
            log_shas(&dir, &["--", path]),
            "{path}"
        );
        assert_eq!(result.stats.segments, 1, "{path}");
    }
    let lib = run(&dir.join("src/lib.rs"), None).entries;
    let merge = lib
        .iter()
        .find(|e| e.summary == "Merge branch feature")
        .unwrap();
    assert_eq!(merge.parents.len(), 2);
    assert_eq!(merge.status, None, "merge는 상태가 없다");
    let notes = run(&dir.join("docs/notes.md"), None).entries;
    assert_eq!(
        notes.iter().map(|e| e.status).collect::<Vec<_>>(),
        [
            Some(FileStatus::Added),
            Some(FileStatus::Deleted),
            Some(FileStatus::Added)
        ]
    );
}

#[test]
fn deleted_paths_pinned_revisions_and_the_whole_repo() {
    let dir = history();
    // 이름을 바꿔 HEAD에 없는 옛 경로: 그 이름으로 바뀐 커밋까지
    let old = run(&dir.join("src/old_name.rs"), None).entries;
    assert_eq!(
        old.iter()
            .map(|e| (e.summary.as_str(), e.status))
            .collect::<Vec<_>>(),
        [
            (
                "rename old_name.rs to new_name.rs",
                Some(FileStatus::Deleted)
            ),
            ("initial import", Some(FileStatus::Added))
        ]
    );
    // 리비전을 정하면 그 커밋부터
    let pinned = run(&dir.join("src/lib.rs"), Some("HEAD~3"));
    let sha = git_text(&dir, &["rev-parse", "HEAD~3"]);
    assert_eq!(pinned.meta.rev.as_deref(), Some(sha.as_str()));
    assert_eq!(pinned.meta.start.as_deref(), Some(sha.as_str()));
    assert_eq!(
        shas(&pinned.entries),
        log_shas(&dir, &["HEAD~3", "--", "src/lib.rs"])
    );
    // 저장소 전체(plx log .)
    let all = run(&dir, None);
    assert_eq!(all.meta.path, ".");
    assert_eq!(shas(&all.entries), log_shas(&dir, &[]));
    assert!(all.stats.first_page_ms.is_none(), "50개보다 적다");
    assert!(all.stats.first_entry_ms.is_some());
}

#[test]
fn errors_and_empty_repositories() {
    let dir = history();
    let events = events_with(
        &HistorySessions::new(),
        &dir.join("src/lib.rs"),
        Some("no-such-rev"),
        |_| true,
    );
    assert!(
        matches!(&events[..], [HistoryEvent::Error { message }] if message.contains("no-such-rev")),
        "{events:?}"
    );

    let empty = Path::new(env!("CARGO_TARGET_TMPDIR")).join("history-empty");
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    git(&empty, &["init", "-q"]);
    let result = run(&empty.join("a.txt"), None);
    assert_eq!(result.meta.start, None);
    assert!(result.entries.is_empty());
}

#[test]
fn cancel_before_and_during_a_request() {
    let dir = history();
    let sessions = HistorySessions::new();
    let repos = RepoCache::new();
    let id = sessions.register();
    assert!(sessions.cancel(id));
    let mut events = Vec::new();
    sessions.run(id, &repos, &dir.join("src/lib.rs"), None, |e| {
        events.push(e);
        true
    });
    assert!(
        matches!(&events[..], [HistoryEvent::Cancelled]),
        "{events:?}"
    );
    assert!(!sessions.cancel(id), "끝난 요청");

    // 받는 쪽이 첫 묶음 뒤에 사라졌다
    let events = events_with(&sessions, &dir, None, |e| {
        !matches!(e, HistoryEvent::Entries { .. })
    });
    assert!(
        matches!(events.last(), Some(HistoryEvent::Cancelled)),
        "{events:?}"
    );
}

#[test]
fn finished_history_is_cached() {
    let dir = history();
    let cache_dir = tempfile::tempdir().unwrap();
    let sessions = HistorySessions::with_cache(Arc::new(DiskCache::new(
        cache_dir.path().to_path_buf(),
        1 << 30,
    )));
    let first = run_with(&sessions, &dir.join("src/new_name.rs"), None);
    assert!(!first.stats.cached);
    let second = run_with(&sessions, &dir.join("src/new_name.rs"), None);
    assert!(second.stats.cached, "두 번째는 캐시에서");
    assert_eq!(second.entries, first.entries);
    assert_eq!(second.stats.segments, 2);
    assert_eq!(second.meta, first.meta);
    // 시작 커밋이 다르면 다른 결과
    let pinned = run_with(&sessions, &dir.join("src/new_name.rs"), Some("HEAD~1"));
    assert!(!pinned.stats.cached);

    // 50개가 넘으면 첫 50개를 먼저 보낸다(저장소 전체: 커밋 수와 관계없이 첫 묶음 크기를 본다).
    let all = run_with(&sessions, &dir, None);
    let again = events_with(&sessions, &dir, None, |_| true);
    let sizes: Vec<usize> = again
        .iter()
        .filter_map(|e| match e {
            HistoryEvent::Entries { entries } => Some(entries.len()),
            _ => None,
        })
        .collect();
    assert_eq!(sizes.iter().sum::<usize>(), all.entries.len());
    assert!(sizes[0] <= 50, "{sizes:?}");
}

/// 같은 이름이 두 브랜치에서 각각 rename으로 생기고 merge됐다: 옛 이름 구간은 두 경계의 부모에서 함께 시작한다.
#[test]
fn renames_on_two_branches_join_in_one_older_segment() {
    let repo = Path::new(env!("CARGO_TARGET_TMPDIR")).join("history-two-renames");
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    let mut tick = 0;
    let mut commit = |message: &str| {
        tick += 1;
        let date = format!("{} +0000", 1_700_000_000 + tick * 60);
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@e",
                "commit",
                "-q",
                "-a",
                "-m",
                message,
            ])
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .env("GIT_CONFIG_GLOBAL", common::NULL_DEVICE)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let lines = |tag: &str| -> String {
        (1..=8)
            .map(|i| format!("{tag} line {i}: the quick brown fox jumps over the lazy dog\n"))
            .collect()
    };
    let write = |name: &str, text: &str| std::fs::write(repo.join(name), text).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    write("a.txt", &lines("base"));
    git(&repo, &["add", "a.txt"]);
    commit("add a");
    git(&repo, &["checkout", "-q", "-b", "side"]);
    git(&repo, &["mv", "a.txt", "b.txt"]);
    commit("side: rename a to b");
    git(&repo, &["checkout", "-q", "main"]);
    write("a.txt", &format!("{}main edit\n", lines("base")));
    commit("main: edit a");
    git(&repo, &["mv", "a.txt", "b.txt"]);
    commit("main: rename a to b");
    // 양쪽이 같은 rename을 했다. 합친 결과를 양쪽과 다르게 만들어 merge 커밋도 이력에 나오게 한다.
    let merged = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "merge",
            "-q",
            "--no-ff",
            "--no-commit",
            "side",
        ])
        .env("GIT_CONFIG_GLOBAL", common::NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    // 충돌(1)이어도 손으로 푼다. 그 밖의 실패는 테스트 준비가 잘못된 것이다.
    assert!(
        matches!(merged.status.code(), Some(0 | 1)),
        "{}",
        String::from_utf8_lossy(&merged.stderr)
    );
    assert!(repo.join(".git/MERGE_HEAD").exists(), "merge 중이어야 한다");
    write("b.txt", &format!("{}merged\n", lines("base")));
    git(&repo, &["add", "b.txt"]);
    commit("merge side");

    let result = run(&repo.join("b.txt"), None);
    let summary: Vec<(&str, Option<FileStatus>, &str)> = result
        .entries
        .iter()
        .map(|e| (e.summary.as_str(), e.status, e.path.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            ("merge side", None, "b.txt"),
            ("main: rename a to b", Some(FileStatus::Renamed), "b.txt"),
            ("side: rename a to b", Some(FileStatus::Renamed), "b.txt"),
            ("main: edit a", Some(FileStatus::Modified), "a.txt"),
            ("add a", Some(FileStatus::Added), "a.txt"),
        ]
    );
    assert_eq!(result.stats.segments, 2);
}
