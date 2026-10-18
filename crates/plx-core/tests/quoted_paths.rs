//! git이 C 문자열로 감싸 내는 경로(탭·따옴표·백슬래시, `core.quotepath=false`여도 감싼다)와 앞뒤 공백·`-`로
//! 시작하는 이름도 blame·히스토리·라인 히스토리·커밋 상세·Quick Open이 이름 그대로 다룬다. 그런 이름으로의
//! rename도 따라간다. Windows는 이런 이름을 만들 수 없어 Unix에서만 본다(CI의 macOS·Linux).
#![cfg(unix)]

mod common;

use std::collections::BTreeSet;

use common::{git, scratch_repo};
use plx_core::blame_session::{SessionEvent, Sessions};
use plx_core::commit::FileStatus;
use plx_core::history::{HistoryEvent, HistorySessions, LineRange};
use plx_core::repo::{FilePath, Repo, RepoCache};

const OLD: &str = "tab\there.txt";
const MOVED: &str = "tab\tmoved \"x\".txt";
const EDITED: &str = "back\\slash \"q\".txt";
/// 첫 커밋에만 있고 그대로인 이름들
const PLAIN: &[&str] = &[
    "quote\"d.txt",
    " lead space.txt",
    "trail space.txt ",
    "-dash.txt",
    "dir with space/inner \"q\".txt",
];

fn setup() -> std::path::PathBuf {
    let repo = scratch_repo("quoted-paths");
    for name in PLAIN.iter().chain([&OLD, &EDITED]) {
        let path = repo.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    }
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    git(&repo, &["mv", OLD, MOVED]);
    std::fs::write(repo.join(EDITED), "one\nTWO\nthree\n").unwrap();
    git(&repo, &["commit", "-q", "-am", "second"]);
    repo
}

/// blame 블록의 (원래 경로들, `previous` 경로들). 오류면 실패한다.
fn blame(
    repos: &RepoCache,
    file: FilePath,
    rev: Option<&str>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let sessions = Sessions::new();
    let id = sessions.register();
    let (mut paths, mut previous) = (BTreeSet::new(), BTreeSet::new());
    let mut end = None;
    sessions.run(id, repos, file, rev, |event| {
        match event {
            SessionEvent::Chunk(chunk) => {
                for block in chunk.blocks {
                    paths.insert(block.path);
                    previous.extend(block.previous.map(|p| p.path));
                }
            }
            other if other.is_end() => end = Some(other),
            _ => {}
        }
        true
    });
    assert!(matches!(end, Some(SessionEvent::Done { .. })), "{end:?}");
    (paths, previous)
}

/// 히스토리 항목의 (상태, 경로, 원래 경로). 오류면 실패한다.
fn history(
    repos: &RepoCache,
    file: FilePath,
    lines: Option<LineRange>,
) -> Vec<(Option<FileStatus>, String, Option<String>)> {
    let sessions = HistorySessions::new();
    let id = sessions.register();
    let mut entries = Vec::new();
    let mut end = None;
    let on_event = |event: HistoryEvent| {
        match event {
            HistoryEvent::Entries { entries: batch } => {
                entries.extend(batch.into_iter().map(|e| (e.status, e.path, e.old_path)));
            }
            other if other.is_end() => end = Some(other),
            _ => {}
        }
        true
    };
    match lines {
        Some(lines) => sessions.run_lines(id, repos, file, None, lines, on_event),
        None => sessions.run(id, repos, file, None, on_event),
    }
    assert!(matches!(end, Some(HistoryEvent::Done { .. })), "{end:?}");
    entries
}

#[test]
fn quoted_names_round_trip() {
    let repo = setup();
    let repos = RepoCache::new();
    let root = repo.to_str().unwrap();
    let in_repo = |path: &str| FilePath::from_request(Some(root), path);
    let from_shell = |path: &str| FilePath::from_request(None, repo.join(path).to_str().unwrap());

    for name in PLAIN {
        let one = BTreeSet::from([name.to_string()]);
        assert_eq!(blame(&repos, from_shell(name), None).0, one, "{name:?}");
        assert_eq!(
            blame(&repos, in_repo(name), Some("HEAD")).0,
            one,
            "{name:?}"
        );
        let added = vec![(Some(FileStatus::Added), name.to_string(), None)];
        assert_eq!(history(&repos, in_repo(name), None), added, "{name:?}");
        let lines = Some(LineRange { start: 1, end: 2 });
        assert_eq!(history(&repos, in_repo(name), lines), added, "{name:?}");
    }

    // 바꾼 파일: 바뀐 줄의 previous(`,` 부모 리비전 blame)가 이름 그대로다.
    let (paths, previous) = blame(&repos, from_shell(EDITED), None);
    assert_eq!(paths, BTreeSet::from([EDITED.to_string()]));
    assert_eq!(previous, BTreeSet::from([EDITED.to_string()]));
    let lines = Some(LineRange { start: 2, end: 2 });
    let edited: Vec<String> = history(&repos, in_repo(EDITED), lines)
        .into_iter()
        .map(|e| e.1)
        .collect();
    assert_eq!(edited, [EDITED, EDITED]);

    // 탭이 든 이름에서 탭·따옴표가 든 이름으로 옮겼다: blame은 옛 이름으로, 히스토리는 rename으로 잇는다.
    assert_eq!(
        blame(&repos, from_shell(MOVED), None).0,
        BTreeSet::from([OLD.to_string()])
    );
    assert_eq!(
        history(&repos, in_repo(MOVED), None),
        [
            (
                Some(FileStatus::Renamed),
                MOVED.to_string(),
                Some(OLD.to_string())
            ),
            (Some(FileStatus::Added), OLD.to_string(), None),
        ]
    );

    let opened = Repo::discover(&repo).unwrap();
    let head = opened.resolve_commit("HEAD").unwrap();
    let detail = plx_core::commit::commit_detail(&opened, head, None).unwrap();
    let files: Vec<_> = detail
        .files
        .iter()
        .map(|f| {
            (
                f.status,
                f.path.as_str(),
                f.old_path.as_deref(),
                f.path_lossy,
            )
        })
        .collect();
    assert_eq!(
        files,
        [
            (FileStatus::Modified, EDITED, None, false),
            (FileStatus::Renamed, MOVED, Some(OLD), false),
        ]
    );

    let lists = plx_core::files::FileLists::new();
    for (query, expected) in [
        ("moved", MOVED),
        ("slash", EDITED),
        ("inner", PLAIN[4]),
        ("trail", PLAIN[2]),
    ] {
        let found = lists.search(&opened, query, 10).unwrap();
        let paths: Vec<&str> = found.items.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, [expected], "{query}");
    }
}
