//! 커밋 그래프: 행 순서와 부모를 `git log --topo-order`와 대조하고, 레이아웃이 끊기지 않는지 본다.

mod common;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use common::{encoding, git, git_text, history};
use plx_core::cache::DiskCache;
use plx_core::graph::{
    CHECKPOINT, GraphEvent, GraphMeta, GraphOptions, GraphRow, GraphScope, GraphSessions,
    GraphStats, LabelKind,
};
use plx_core::repo::RepoCache;

/// `git log --topo-order <revs>`가 보여 주는 (sha, 부모들)
fn reference(repo: &Path, revs: &[&str]) -> Vec<(String, Vec<String>)> {
    let mut args = vec!["log", "--topo-order", "--format=%H %P"];
    args.extend_from_slice(revs);
    git_text(repo, &args)
        .lines()
        .map(|line| {
            let mut parts = line.split(' ').filter(|p| !p.is_empty()).map(str::to_owned);
            (parts.next().unwrap(), parts.collect())
        })
        .collect()
}

struct Opened {
    meta: GraphMeta,
    events: Vec<GraphEvent>,
    rows: Vec<GraphRow>,
    stats: GraphStats,
}

/// 그래프를 끝까지 열어 모든 행과 이벤트를 받는다.
fn open_with(sessions: &GraphSessions, path: &Path, options: GraphOptions) -> (u64, Opened) {
    let repos = RepoCache::new();
    let (tx, rx) = mpsc::channel();
    let id = sessions
        .open(&repos, path, options, move |event| tx.send(event).is_ok())
        .expect("그래프 열기");
    let mut events = Vec::new();
    loop {
        let event = rx.recv_timeout(Duration::from_secs(60)).expect("이벤트");
        let end = event.is_end();
        events.push(event);
        if end {
            break;
        }
    }
    let GraphEvent::Meta(meta) = events[0].clone() else {
        panic!("{events:?}");
    };
    let GraphEvent::Done { stats } = events.last().unwrap().clone() else {
        panic!("{events:?}");
    };
    let window = sessions.rows(id, 0, usize::MAX).expect("행");
    assert!(window.done);
    assert_eq!(window.available, stats.rows);
    assert_eq!(window.total, stats.rows);
    let rows = window.rows;
    (
        id,
        Opened {
            meta,
            events,
            rows,
            stats,
        },
    )
}

fn open(path: &Path, options: GraphOptions) -> Opened {
    let sessions = GraphSessions::new();
    let (id, opened) = open_with(&sessions, path, options);
    assert!(sessions.close(id));
    assert!(!sessions.close(id));
    assert_eq!(sessions.open_count(), 0);
    opened
}

const ALL: GraphOptions = GraphOptions {
    scope: GraphScope::All,
};

/// 행 순서·부모가 git과 같고, 모든 부모 선이 부모 점까지 끊기지 않는다.
fn assert_matches_git(repo: &Path, opened: &Opened, revs: &[&str]) {
    let expected = reference(repo, revs);
    let ours: Vec<(String, Vec<String>)> = opened
        .rows
        .iter()
        .map(|r| (r.sha.clone(), r.parents.clone()))
        .collect();
    assert_eq!(ours, expected, "{} {revs:?}", repo.display());
    assert_eq!(opened.stats.rows, expected.len());
    check_edges(&opened.rows);
}

/// 모든 부모 선이 부모 점까지 끊기지 않고, 행 사이 가장자리의 선이 맞물린다.
fn check_edges(rows: &[GraphRow]) {
    let row_of: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.sha.as_str(), i))
        .collect();
    for (r, row) in rows.iter().enumerate() {
        let layout = &row.layout;
        assert_eq!(layout.down.len(), row.parents.len(), "{}", row.sha);
        for (k, parent) in row.parents.iter().enumerate() {
            let target = row_of[parent.as_str()];
            assert!(target > r);
            let mut lane = layout.down[k][0];
            for next in &rows[r + 1..target] {
                let line = next
                    .layout
                    .through
                    .iter()
                    .find(|t| t[0] == lane)
                    .unwrap_or_else(|| panic!("{} → {parent}: {}에서 끊긴다", row.sha, next.sha));
                lane = line[1];
            }
            assert!(
                rows[target].layout.up.iter().any(|u| u[0] == lane),
                "{} → {parent}",
                row.sha
            );
        }
        if let Some(next) = rows.get(r + 1) {
            let bottom: HashSet<u16> = layout
                .through
                .iter()
                .map(|t| t[1])
                .chain(layout.down.iter().map(|d| d[0]))
                .collect();
            let top: Vec<u16> = next
                .layout
                .through
                .iter()
                .map(|t| t[0])
                .chain(next.layout.up.iter().map(|u| u[0]))
                .collect();
            assert_eq!(top.len(), top.iter().collect::<HashSet<_>>().len());
            assert_eq!(
                bottom,
                top.into_iter().collect(),
                "{} → {}",
                row.sha,
                next.sha
            );
        }
    }
}

#[test]
fn rows_follow_git_topo_order() {
    for repo in [history(), encoding()] {
        let opened = open(&repo, GraphOptions::default());
        assert_matches_git(&repo, &opened, &["HEAD", "--branches"]);
        assert!(!opened.meta.commit_graph);
        assert_eq!(opened.stats.from_objects, opened.rows.len());
        assert_eq!(opened.stats.from_graph, 0);
    }
}

#[test]
fn events_arrive_in_order() {
    let opened = open(&history(), GraphOptions::default());
    let kinds: Vec<&str> = opened
        .events
        .iter()
        .map(|e| match e {
            GraphEvent::Meta(_) => "meta",
            GraphEvent::Loading { .. } => "loading",
            GraphEvent::Page { .. } => "page",
            GraphEvent::Progress { .. } => "progress",
            GraphEvent::Done { .. } => "done",
            GraphEvent::Error { .. } => "error",
            GraphEvent::Cancelled => "cancelled",
        })
        .filter(|k| *k != "loading" && *k != "progress")
        .collect();
    assert_eq!(kinds, ["meta", "page", "done"]);
    let GraphEvent::Page { rows, total } = &opened.events[1] else {
        panic!();
    };
    assert_eq!(*total, opened.rows.len());
    assert_eq!(rows, &opened.rows, "fixtures는 첫 화면에 다 들어간다");
    assert!(opened.stats.first_page_ms <= opened.stats.total_ms);
}

#[test]
fn labels_branches_tags_and_head() {
    let repo = history();
    let opened = open(&repo, GraphOptions::default());
    let meta = &opened.meta;
    assert_eq!(meta.branch.as_deref(), Some("main"));
    assert_eq!(
        meta.head.as_deref(),
        Some(git_text(&repo, &["rev-parse", "HEAD"]).as_str())
    );
    assert_eq!(meta.tips, 4, "HEAD(main), conflict, feature, topic");
    assert_eq!(meta.hidden_remotes, 0);

    let labels_at = |rev: &str| {
        let sha = git_text(&repo, &["rev-parse", &format!("{rev}^{{commit}}")]);
        let row = opened.rows.iter().find(|r| r.sha == sha).expect(rev);
        row.labels
            .iter()
            .map(|l| (l.name.as_str(), l.kind, l.current))
            .collect::<Vec<_>>()
    };
    // 현재 브랜치가 먼저, annotated 태그(v0.2)는 가리키는 커밋에
    assert_eq!(
        labels_at("main"),
        [
            ("main", LabelKind::Branch, true),
            ("v0.2", LabelKind::Tag, false)
        ]
    );
    assert_eq!(labels_at("v0.1"), [("v0.1", LabelKind::Tag, false)]);
    assert_eq!(labels_at("topic"), [("topic", LabelKind::Branch, false)]);

    // 요약은 커밋 인코딩대로(EUC-KR 커밋 메시지)
    let opened = open(&encoding(), GraphOptions::default());
    assert_eq!(opened.rows[0].summary, "인코딩 테스트: EUC-KR 커밋 메시지");
    assert!(
        opened
            .rows
            .iter()
            .any(|r| r.summary == "설명.md를 문서/로 옮김")
    );
}

/// fixtures 저장소를 복제한다(원격 브랜치가 생긴다). 사용자 설정과 떼어 놓는다.
fn clone(source: &Path, dir: &Path, extra: &[&str]) {
    let mut args = vec!["clone", "--quiet", "--no-local"];
    args.extend_from_slice(extra);
    // file:// 전송을 써야 --depth가 먹는다. Windows 경로(C:/…)는 file:///C:/…로 쓴다.
    let path = source.to_string_lossy().replace('\\', "/");
    let url = if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    };
    let target = dir.to_string_lossy().into_owned();
    args.push(&url);
    args.push(&target);
    git(dir.parent().unwrap(), &args);
}

/// 사용자 이름이 없는 환경(CI)에서도 커밋을 만든다.
fn git_as_user(repo: &Path, args: &[&str]) {
    let mut all = vec!["-c", "user.name=plx", "-c", "user.email=plx@example.com"];
    all.extend_from_slice(args);
    git(repo, &all);
}

fn commit(repo: &Path, message: &str) {
    git_as_user(repo, &["commit", "--quiet", "--allow-empty", "-m", message]);
}

#[test]
fn remotes_default_to_upstreams_only() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("clone");
    clone(&history(), &repo, &[]);
    // 로컬 main이 origin/main보다 앞서고, upstream이 없는 로컬 브랜치도 있다.
    commit(&repo, "local work");
    git(&repo, &["branch", "local-only", "HEAD~1"]);

    let opened = open(&repo, GraphOptions::default());
    assert_matches_git(
        &repo,
        &opened,
        &["HEAD", "--branches", "refs/remotes/origin/main"],
    );
    assert_eq!(opened.meta.hidden_remotes, 3, "conflict, feature, topic");
    // 시작점이 아닌 원격 브랜치도 그래프 안의 커밋이면 이름표를 단다.
    let feature = git_text(&repo, &["rev-parse", "origin/feature"]);
    let row = opened.rows.iter().find(|r| r.sha == feature).unwrap();
    assert_eq!(row.labels[0].name, "origin/feature");
    assert_eq!(row.labels[0].kind, LabelKind::Remote);
    // origin/HEAD는 이름표를 따로 달지 않는다.
    assert!(
        opened
            .rows
            .iter()
            .flat_map(|r| &r.labels)
            .all(|l| l.name != "origin/HEAD")
    );

    let all = open(&repo, ALL);
    assert_matches_git(&repo, &all, &["HEAD", "--branches", "--remotes"]);
    assert_eq!(all.meta.hidden_remotes, 0);
    assert!(all.rows.len() > opened.rows.len());

    let head = open(&repo, GraphOptions::scope(GraphScope::Head));
    assert_matches_git(&repo, &head, &["HEAD"]);
    assert_eq!(
        (head.meta.tips, head.meta.hidden_branches),
        (1, 1),
        "local-only (main은 HEAD)"
    );
    assert_eq!(head.meta.hidden_remotes, 4);
}

#[test]
fn commit_graph_gives_the_same_graph() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("clone");
    clone(&history(), &repo, &[]);
    let without = open(&repo, ALL);
    // 테스트용 복제본에만 쓴다.
    git(&repo, &["commit-graph", "write", "--reachable"]);
    let with = open(&repo, ALL);
    assert!(with.meta.commit_graph);
    assert_eq!(with.stats.from_objects, 0);
    assert_eq!(with.stats.from_graph, with.rows.len());
    assert_eq!(with.rows, without.rows);

    // commit-graph를 쓴 뒤에 생긴 커밋은 객체에서 읽고 나머지는 그래프에서 읽는다.
    commit(&repo, "after graph");
    git_as_user(&repo, &["merge", "--quiet", "--no-edit", "origin/topic"]);
    let mixed = open(&repo, ALL);
    assert!(mixed.stats.from_objects >= 1, "{:?}", mixed.stats);
    assert!(mixed.stats.from_graph >= with.rows.len() - 1);
    assert_matches_git(&repo, &mixed, &["HEAD", "--branches", "--remotes"]);
}

#[test]
fn shallow_clone_stops_at_the_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("shallow");
    clone(&history(), &repo, &["--depth", "3", "--no-single-branch"]);
    let opened = open(&repo, ALL);
    assert_matches_git(&repo, &opened, &["HEAD", "--branches", "--remotes"]);
    let shallow: HashSet<String> = std::fs::read_to_string(repo.join(".git/shallow"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    for row in &opened.rows {
        if shallow.contains(&row.sha) {
            assert!(row.parents.is_empty(), "경계 커밋은 부모가 없다");
            assert!(row.layout.down.is_empty());
        }
    }
}

#[test]
fn windows_replay_from_checkpoints() {
    // CHECKPOINT보다 긴 이력: merge가 섞인 커밋을 여러 개 만든다.
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("long");
    clone(&history(), &repo, &[]);
    for i in 0..(CHECKPOINT + 40) {
        commit(&repo, &format!("c{i}"));
        if i % 50 == 25 {
            git(&repo, &["branch", &format!("b{i}")]);
        }
    }
    let sessions = GraphSessions::new();
    let (id, opened) = open_with(&sessions, &repo, GraphOptions::default());
    assert!(opened.rows.len() > CHECKPOINT + 40);
    assert_matches_git(
        &repo,
        &opened,
        &["HEAD", "--branches", "refs/remotes/origin/main"],
    );
    for (start, end) in [
        (0, 10),
        (CHECKPOINT - 3, CHECKPOINT + 3),
        (CHECKPOINT + 5, CHECKPOINT + 30),
        (opened.rows.len() - 2, opened.rows.len() + 100),
    ] {
        let window = sessions.rows(id, start, end).unwrap();
        assert_eq!(window.start, start);
        assert_eq!(
            window.rows,
            opened.rows[start..end.min(opened.rows.len())],
            "{start}..{end}"
        );
    }
    let past = sessions.rows(id, 10_000, 10_010).unwrap();
    assert!(past.rows.is_empty());
    assert_eq!(past.start, opened.rows.len());

    // 리비전으로 행 찾기
    let head = sessions.find(id, "HEAD~3").unwrap();
    assert_eq!(
        opened.rows[head.unwrap()].sha,
        git_text(&repo, &["rev-parse", "HEAD~3"])
    );
    assert_eq!(
        sessions.find(id, "v0.1").unwrap().unwrap(),
        opened.rows.len() - 1
    );
    assert!(sessions.find(id, "nope").is_err());
    assert!(sessions.close(id));
    assert!(sessions.rows(id, 0, 10).is_err(), "닫은 그래프");
}

#[test]
fn empty_and_detached_repositories() {
    let temp = tempfile::tempdir().unwrap();
    let empty = temp.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    git(&empty, &["init", "--quiet"]);
    let opened = open(&empty, GraphOptions::default());
    assert!(opened.rows.is_empty());
    assert_eq!(opened.meta.head, None);

    let detached = temp.path().join("detached");
    clone(&history(), &detached, &[]);
    git(&detached, &["checkout", "--quiet", "--detach", "HEAD~2"]);
    let opened = open(&detached, GraphOptions::default());
    assert_eq!(opened.meta.branch, None);
    let head = git_text(&detached, &["rev-parse", "HEAD"]);
    let row = opened.rows.iter().find(|r| r.sha == head).unwrap();
    assert_eq!(row.labels[0].name, "HEAD");
    assert_eq!(row.labels[0].kind, LabelKind::Head);
    assert!(row.labels[0].current);
}

#[test]
fn closing_stops_the_walk() {
    let sessions = GraphSessions::new();
    let repos = RepoCache::new();
    let (tx, rx) = mpsc::channel();
    // 받는 쪽이 첫 이벤트에서 멈추면 워크도 멈춘다.
    let id = sessions
        .open(&repos, &history(), GraphOptions::default(), move |event| {
            let _ = tx.send(event);
            false
        })
        .unwrap();
    let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(first, GraphEvent::Meta(_)));
    let last = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(last, GraphEvent::Cancelled), "{last:?}");
    assert!(sessions.close(id));
}

#[test]
fn commit_table_replaces_object_reads_without_commit_graph() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("clone");
    clone(&history(), &repo, &[]);
    let cache_dir = temp.path().join("cache");
    let cache = || Arc::new(DiskCache::new(cache_dir.clone(), 1 << 30));
    let opened_with = |sessions: &GraphSessions| {
        let (id, opened) = open_with(sessions, &repo, ALL);
        sessions.close(id);
        opened
    };

    // 처음에는 모두 객체에서 읽고 표를 쓴다.
    let first = opened_with(&GraphSessions::with_cache_min(cache(), 1));
    assert_eq!(first.stats.from_objects, first.rows.len());
    assert!(
        std::fs::read_dir(&cache_dir)
            .unwrap()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "bin")),
        "표를 캐시에 쓴다"
    );

    // 앱을 다시 띄운 것처럼 새 세션 모음: 표에서 읽는다.
    let second = opened_with(&GraphSessions::with_cache_min(cache(), 1));
    assert_eq!(second.stats.from_objects, 0, "{:?}", second.stats);
    assert_eq!(second.stats.from_table, second.rows.len());
    assert_eq!(second.rows, first.rows);

    // 그 뒤에 생긴 커밋만 객체에서 읽는다.
    commit(&repo, "new 1");
    commit(&repo, "new 2");
    let sessions = GraphSessions::with_cache_min(cache(), 1000);
    let third = opened_with(&sessions);
    assert_eq!(third.stats.from_objects, 2);
    assert_eq!(third.stats.from_table, third.rows.len() - 2);
    assert_matches_git(&repo, &third, &["HEAD", "--branches", "--remotes"]);
    // 새 커밋이 한도보다 적으면 표를 다시 쓰지 않는다.
    assert_eq!(opened_with(&sessions).stats.from_objects, 2);

    // commit-graph가 생기면 표는 쓰지 않는다.
    git(&repo, &["commit-graph", "write", "--reachable"]);
    let with_graph = opened_with(&GraphSessions::with_cache_min(cache(), 1));
    assert_eq!(with_graph.stats.from_table, 0);
    assert_eq!(with_graph.stats.from_graph, with_graph.rows.len());
}

#[test]
fn shallow_clones_are_not_put_in_the_table() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("shallow");
    clone(&history(), &repo, &["--depth", "3"]);
    let cache_dir = temp.path().join("cache");
    let sessions =
        GraphSessions::with_cache_min(Arc::new(DiskCache::new(cache_dir.clone(), 1 << 30)), 1);
    let (id, _) = open_with(&sessions, &repo, ALL);
    sessions.close(id);
    // 경계 커밋을 부모 없는 커밋으로 적어 두면 나중에 이력을 더 받았을 때 틀린다.
    assert!(!cache_dir.exists() || std::fs::read_dir(&cache_dir).unwrap().next().is_none());
}

/// 행마다 git이 보는 (메시지, 작성자, 메일)로 기대 결과를 만든다.
fn text_matches(repo: &Path, rows: &[GraphRow], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| {
            let text = git_text(repo, &["log", "-1", "--format=%B%n%an%n%ae", &row.sha]);
            row.sha.starts_with(&query) || text.to_lowercase().contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn search_messages_authors_and_shas() {
    let repo = history();
    let sessions = GraphSessions::new();
    let (id, opened) = open_with(&sessions, &repo, GraphOptions::default());
    let n = opened.rows.len();
    for query in ["feature", "LINE", "fixture author", "notes"] {
        let expected = text_matches(&repo, &opened.rows, query);
        assert!(!expected.is_empty(), "{query}");
        // 앞으로: 0번 행 다음부터 차례로, 끝에 닿으면 처음으로
        let mut found = Vec::new();
        let mut from = n - 1;
        for _ in 0..expected.len() {
            let row = sessions
                .search(id, query, from, true)
                .unwrap()
                .expect(query);
            found.push(row);
            from = row;
        }
        assert_eq!(found, expected, "{query}");
        // 뒤로
        let back = sessions.search(id, query, expected[0], false).unwrap();
        assert_eq!(
            back,
            expected.last().copied(),
            "{query}: 처음에서 뒤로 가면 마지막"
        );
    }
    // sha 앞부분
    let sha = &opened.rows[5].sha;
    assert_eq!(sessions.search(id, &sha[..7], 0, true).unwrap(), Some(5));
    assert_eq!(sessions.search(id, "없는 말", 0, true).unwrap(), None);
    assert_eq!(sessions.search(id, "   ", 0, true).unwrap(), None);
    sessions.close(id);
    assert!(
        sessions.search(id, "feature", 0, true).is_err(),
        "닫은 그래프"
    );

    // 커밋 인코딩(EUC-KR) 메시지도 찾는다.
    let sessions = GraphSessions::new();
    let (id, opened) = open_with(&sessions, &encoding(), GraphOptions::default());
    let row = sessions
        .search(id, "인코딩 테스트", 0, true)
        .unwrap()
        .unwrap();
    assert!(opened.rows[row].summary.starts_with("인코딩 테스트"));
}

#[test]
fn search_paths_like_git_log() {
    for (repo, paths) in [
        (
            history(),
            vec![
                "src/lib.rs",
                "src",
                "src/new_name.rs",
                "core/*.rs",
                "없는/경로",
            ],
        ),
        (encoding(), vec!["한글/파일 이름.txt", "문서"]),
    ] {
        let sessions = GraphSessions::new();
        let (id, opened) = open_with(&sessions, &repo, GraphOptions::default());
        let row_of: HashMap<&str, usize> = opened
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.sha.as_str(), i))
            .collect();
        for path in paths {
            let shas = git_text(
                &repo,
                &[
                    "log",
                    "--format=%H",
                    "--full-history",
                    "--no-merges",
                    "HEAD",
                    "--branches",
                    "--",
                    path,
                ],
            );
            let mut expected: Vec<usize> = shas.lines().map(|sha| row_of[sha]).collect();
            expected.sort_unstable();
            assert_eq!(sessions.search_path(id, path).unwrap(), expected, "{path}");
        }
        sessions.close(id);
    }
}
