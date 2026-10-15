//! 저장소 읽기 명령. 무거운 일은 블로킹 스레드에서 하고 결과는 Channel로 흘려보낸다.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use plx_core::blame_session::{SessionEvent, Sessions};
use plx_core::cache::DiskCache;
use plx_core::commit::{self, CommitDetail, FileVersions};
use plx_core::commit_graph::{self, GraphStatus, Progress};
use plx_core::graph::{GraphEvent, GraphOptions, GraphSessions, GraphWindow};
use plx_core::history::{HistoryEvent, HistorySessions, LineRange};
use plx_core::navigate::{self, Direction, Version};
use plx_core::repo::{CommitInfo, RepoCache};
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

/// 결과 캐시 한도. 넘으면 오래 안 쓴 것부터 지운다.
const CACHE_MAX_BYTES: u64 = 512 << 20;

/// 앱 전체가 나눠 쓰는 저장소 캐시와 진행 중인 요청.
pub struct GitState {
    repos: RepoCache,
    blames: Sessions,
    histories: HistorySessions,
    graphs: GraphSessions,
    cache: Option<Arc<DiskCache>>,
}

impl GitState {
    /// `cache_dir`가 있으면 blame·히스토리 결과를 그 아래 `results/`에 캐시한다.
    pub fn new(cache_dir: Option<PathBuf>) -> GitState {
        let cache =
            cache_dir.map(|dir| Arc::new(DiskCache::new(dir.join("results"), CACHE_MAX_BYTES)));
        let (blames, histories, graphs) = match &cache {
            Some(cache) => (
                Sessions::with_cache(cache.clone()),
                HistorySessions::with_cache(cache.clone()),
                GraphSessions::with_cache(cache.clone()),
            ),
            None => (
                Sessions::new(),
                HistorySessions::new(),
                GraphSessions::new(),
            ),
        };
        GitState {
            repos: RepoCache::new(),
            blames,
            histories,
            graphs,
            cache,
        }
    }

    /// 캐시가 한도를 넘었으면 줄인다. 시작할 때 백그라운드에서 한 번 부른다.
    pub fn prune_cache(&self) {
        if let Some(cache) = &self.cache
            && let Err(e) = cache.prune()
        {
            eprintln!(
                "plx: 결과 캐시를 정리하지 못했다 ({}): {e}",
                cache.dir().display()
            );
        }
    }
}

/// `repo`가 있으면 `path`는 그 저장소 기준 경로이고, 없으면 절대 경로다.
fn resolve(repo: Option<&str>, path: &str) -> PathBuf {
    match repo {
        Some(repo) => Path::new(repo).join(path),
        None => PathBuf::from(path),
    }
}

/// 파일 blame을 시작하고 요청 id를 바로 돌려준다. 결과는 `on_event`로 온다
/// (`meta` → `chunk`* → `done`·`error`·`cancelled`). `rev`가 없으면 작업트리 파일이다.
#[tauri::command]
pub async fn blame_open(
    git: State<'_, Arc<GitState>>,
    repo: Option<String>,
    path: String,
    rev: Option<String>,
    on_event: Channel<SessionEvent>,
) -> Result<u64, String> {
    let git = git.inner().clone();
    let id = git.blames.register();
    let path = resolve(repo.as_deref(), &path);
    tauri::async_runtime::spawn_blocking(move || {
        git.blames
            .run(id, &git.repos, &path, rev.as_deref(), |event| {
                on_event.send(event).is_ok()
            });
    });
    Ok(id)
}

/// 진행 중인 blame을 멈춘다. 이미 끝났으면 `false`.
#[tauri::command]
pub fn blame_cancel(git: State<'_, Arc<GitState>>, id: u64) -> bool {
    git.blames.cancel(id)
}

/// 파일 히스토리를 시작하고 요청 id를 바로 돌려준다. 결과는 `on_event`로 온다
/// (`meta` → `entries`* → `done`·`error`·`cancelled`). `rev`가 없으면 HEAD부터다.
#[tauri::command]
pub async fn history_open(
    git: State<'_, Arc<GitState>>,
    repo: Option<String>,
    path: String,
    rev: Option<String>,
    on_event: Channel<HistoryEvent>,
) -> Result<u64, String> {
    let git = git.inner().clone();
    let id = git.histories.register();
    let path = resolve(repo.as_deref(), &path);
    tauri::async_runtime::spawn_blocking(move || {
        git.histories
            .run(id, &git.repos, &path, rev.as_deref(), |event| {
                on_event.send(event).is_ok()
            });
    });
    Ok(id)
}

/// 라인 히스토리(`git log -L`)를 시작하고 요청 id를 바로 돌려준다. 이벤트는 파일 히스토리와 같고 커밋마다
/// 그 줄 범위의 hunk가 붙는다. `rev`가 없으면 `start`..=`end`는 작업트리 파일의 줄이다(HEAD 버전의 줄로 옮긴다).
/// 멈출 때는 `history_cancel`을 쓴다.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn line_history_open(
    git: State<'_, Arc<GitState>>,
    repo: Option<String>,
    path: String,
    rev: Option<String>,
    start: u32,
    end: u32,
    on_event: Channel<HistoryEvent>,
) -> Result<u64, String> {
    let git = git.inner().clone();
    let id = git.histories.register();
    let path = resolve(repo.as_deref(), &path);
    tauri::async_runtime::spawn_blocking(move || {
        git.histories.run_lines(
            id,
            &git.repos,
            &path,
            rev.as_deref(),
            LineRange { start, end },
            |event| on_event.send(event).is_ok(),
        );
    });
    Ok(id)
}

/// 진행 중인 히스토리(파일·라인)를 멈춘다. 이미 끝났으면 `false`.
#[tauri::command]
pub fn history_cancel(git: State<'_, Arc<GitState>>, id: u64) -> bool {
    git.histories.cancel(id)
}

/// 커밋 그래프를 열고 id를 바로 돌려준다. 이벤트는 `on_event`로 온다
/// (`meta` → `loading`* → `page` → `progress`* → `done`·`error`·`cancelled`). 첫 화면 뒤의 행은 `graph_rows`로 읽는다.
/// 화면을 떠나면 `graph_close`로 닫는다(행 레이아웃을 메모리에 둔다).
#[tauri::command]
pub async fn graph_open(
    git: State<'_, Arc<GitState>>,
    repo: String,
    options: Option<GraphOptions>,
    on_event: Channel<GraphEvent>,
) -> Result<u64, String> {
    let git = git.inner().clone();
    // 참조 읽기도 파일을 읽으므로 블로킹 스레드에서 한다.
    tauri::async_runtime::spawn_blocking(move || {
        git.graphs.open(
            &git.repos,
            Path::new(&repo),
            options.unwrap_or_default(),
            move |event| on_event.send(event).is_ok(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 그래프의 `start`..`end` 행(요약·작성자·이름표·레이아웃).
#[tauri::command]
pub async fn graph_rows(
    git: State<'_, Arc<GitState>>,
    id: u64,
    start: usize,
    end: usize,
) -> Result<GraphWindow, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || git.graphs.rows(id, start, end))
        .await
        .map_err(|e| e.to_string())?
}

/// 리비전이 놓인 행. 그래프에 없으면 `null`.
#[tauri::command]
pub async fn graph_find(
    git: State<'_, Arc<GitState>>,
    id: u64,
    rev: String,
) -> Result<Option<usize>, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || git.graphs.find(id, &rev))
        .await
        .map_err(|e| e.to_string())?
}

/// 그래프를 닫는다. 이미 닫혔으면 `false`.
#[tauri::command]
pub fn graph_close(git: State<'_, Arc<GitState>>, id: u64) -> bool {
    git.graphs.close(id)
}

/// 커밋 상세(hover 카드). `repo`는 blame `meta`의 작업트리 경로다.
#[tauri::command]
pub async fn commit_info(
    git: State<'_, Arc<GitState>>,
    repo: String,
    rev: String,
) -> Result<CommitInfo, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        let id = repo.resolve_commit(&rev)?;
        repo.commit_info(id)
    })
    .await
}

/// 커밋 상세: 첫 부모와 비교한 파일 목록과 줄 수. `repo`는 저장소 안의 아무 경로(plx 호출의 cwd 등)다.
/// `focus`는 화면이 처음 고를 파일(저장소 기준)이다. 그 파일의 rename은 파일이 많은 커밋에서도 찾는다.
#[tauri::command]
pub async fn commit_detail(
    git: State<'_, Arc<GitState>>,
    repo: String,
    rev: String,
    focus: Option<String>,
) -> Result<CommitDetail, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        let id = repo.resolve_commit(&rev)?;
        commit::commit_detail(&repo, id, focus.as_deref())
    })
    .await
}

/// 파일 하나의 두 버전(diff 화면). blob id가 없으면 빈 쪽이다(추가·삭제).
#[tauri::command]
pub async fn file_versions(
    git: State<'_, Arc<GitState>>,
    repo: String,
    old_id: Option<String>,
    new_id: Option<String>,
) -> Result<FileVersions, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        commit::file_versions(&repo, old_id.as_deref(), new_id.as_deref())
    })
    .await
}

/// `from`의 `line`번째 줄이 `to`에서 몇 번째 줄인가(`,` 부모 리비전 blame).
#[tauri::command]
pub async fn blame_map_line(
    git: State<'_, Arc<GitState>>,
    repo: String,
    from: Version,
    to: Version,
    line: u32,
) -> Result<u32, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        navigate::map_line(&repo, &from, &to, line)
    })
    .await
}

#[derive(Serialize)]
pub struct Neighbor {
    version: Version,
    /// `current`의 커서 줄에 해당하는 줄
    line: u32,
}

/// 이전(`<`)·다음(`>`) 리비전과 커서 줄의 새 위치. 없으면 `null`.
#[tauri::command]
pub async fn blame_neighbor(
    git: State<'_, Arc<GitState>>,
    repo: String,
    current: Version,
    direction: Direction,
    line: u32,
) -> Result<Option<Neighbor>, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        let Some(version) = navigate::neighbor(&repo, &current, direction)? else {
            return Ok(None);
        };
        let line = navigate::map_line(&repo, &current, &version, line)?;
        Ok(Some(Neighbor { version, line }))
    })
    .await
}

/// 저장소의 가속 상태(commit-graph·Bloom 필터). 파일 머리만 읽는다.
#[tauri::command]
pub async fn repo_acceleration(
    git: State<'_, Arc<GitState>>,
    repo: String,
) -> Result<GraphStatus, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        commit_graph::status(&repo)
    })
    .await
}

/// 가속 설정: 사용자가 확인한 뒤 commit-graph와 Bloom 필터를 쓴다(저장소를 바꾸는 유일한 명령).
/// 진행은 `on_progress`로 오고, 끝나면 새 상태를 돌려준다.
#[tauri::command]
pub async fn accelerate(
    git: State<'_, Arc<GitState>>,
    repo: String,
    on_progress: Channel<Progress>,
) -> Result<GraphStatus, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        commit_graph::write(&repo, |progress| {
            let _ = on_progress.send(progress);
        })
    })
    .await
}

/// 블로킹 스레드에서 저장소 작업을 하고 오류는 화면에 보여 줄 문장으로 바꾼다.
async fn blocking<T: Send + 'static>(
    git: State<'_, Arc<GitState>>,
    f: impl FnOnce(&GitState) -> plx_core::repo::Result<T> + Send + 'static,
) -> Result<T, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || f(&git).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}
