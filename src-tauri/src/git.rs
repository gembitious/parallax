//! 저장소 읽기 명령. 무거운 일은 블로킹 스레드에서 하고 결과는 Channel로 흘려보낸다.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use plx_core::ai_notes::{AiAttribution, AiItem, AiNotes};
use plx_core::blame_session::{SessionEvent, Sessions};
use plx_core::cache::DiskCache;
use plx_core::cli::Request;
use plx_core::commit::{self, CommitDetail, FileStat, FileVersions, StatRequest};
use plx_core::commit_graph::{self, GraphStatus, Progress};
use plx_core::files::{DirListing, FileLists, FileSearch};
use plx_core::graph::{GraphEvent, GraphOptions, GraphSessions, GraphWindow};
use plx_core::history::{HistoryEvent, HistorySessions, LineRange};
use plx_core::navigate::{self, Direction, Version};
use plx_core::repo::{CommitInfo, FilePath, RepoCache};
use plx_core::watch::{Change, Watchers};
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

/// 결과 캐시 한도. 넘으면 오래 안 쓴 것부터 지운다.
const CACHE_MAX_BYTES: u64 = 512 << 20;

/// 요청 대상의 저장소와 파일([`GitState::locate`]).
#[derive(Debug, Clone)]
pub struct Located {
    /// 작업트리 경로(blame 화면의 `meta.repo`와 같은 문자열)
    pub repo: String,
    /// 파일을 준 요청의 저장소 기준 경로(`/` 구분)
    pub path: Option<String>,
}

/// 앱 전체가 나눠 쓰는 저장소 캐시와 진행 중인 요청.
pub struct GitState {
    repos: RepoCache,
    blames: Sessions,
    histories: HistorySessions,
    graphs: GraphSessions,
    watchers: Watchers,
    files: FileLists,
    ai: AiNotes,
    cache: Option<Arc<DiskCache>>,
}

impl GitState {
    /// `cache_dir`가 있으면 blame·히스토리 결과를 그 아래 `results/`에 캐시한다.
    /// 지켜보는 저장소의 HEAD·참조나 파일이 바뀌면 `on_change`를 부른다(다른 스레드에서).
    pub fn new(
        cache_dir: Option<PathBuf>,
        on_change: impl Fn(Change) + Send + 'static,
    ) -> GitState {
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
            watchers: Watchers::new(on_change),
            files: FileLists::new(),
            ai: AiNotes::new(),
            cache,
        }
    }

    /// `path`(파일·디렉터리)를 담은 저장소의 작업트리. 앱의 저장소 캐시로 연다(곧 화면이 같은 저장소를 연다).
    pub fn repo_root(&self, path: &Path) -> Result<PathBuf, plx_core::repo::Error> {
        Ok(self.repos.for_path(path)?.workdir().to_path_buf())
    }

    /// `plx` 요청의 저장소(작업트리)와, 파일을 준 요청이면 그 파일의 저장소 기준 경로(blame은 심볼릭 링크를 푼
    /// 경로: blame 화면이 보이는 파일이다). 화면이 같은 저장소의 작업 공간과 같은 파일의 탭을 이것으로 찾는다.
    /// 앱의 저장소 캐시로 연다(곧 화면이 같은 저장소를 연다).
    pub fn locate(&self, request: &Request, cwd: &Path) -> Result<Located, plx_core::repo::Error> {
        let repo = self.repos.for_path(request.repo_path(cwd))?;
        let path = match request {
            Request::Blame { path, .. } => repo
                .relative_path(path)
                .and_then(|rel| repo.resolve_link(&rel))
                .ok(),
            Request::Log { path } | Request::LineLog { path, .. } => repo.relative_path(path).ok(),
            Request::Graph | Request::Show { .. } => None,
        };
        Ok(Located {
            repo: repo.workdir().to_string_lossy().into_owned(),
            path,
        })
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
    // `repo`가 있으면 `path`는 그 저장소 기준 경로(git이 준 경로라 그대로 쓴다), 없으면 터미널에서 준 절대 경로다.
    let file = FilePath::from_request(repo.as_deref(), &path);
    tauri::async_runtime::spawn_blocking(move || {
        git.blames
            .run(id, &git.repos, file, rev.as_deref(), |event| {
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
    // `repo`가 있으면 `path`는 그 저장소 기준 경로(git이 준 경로라 그대로 쓴다), 없으면 터미널에서 준 절대 경로다.
    let file = FilePath::from_request(repo.as_deref(), &path);
    tauri::async_runtime::spawn_blocking(move || {
        git.histories
            .run(id, &git.repos, file, rev.as_deref(), |event| {
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
    // `repo`가 있으면 `path`는 그 저장소 기준 경로(git이 준 경로라 그대로 쓴다), 없으면 터미널에서 준 절대 경로다.
    let file = FilePath::from_request(repo.as_deref(), &path);
    tauri::async_runtime::spawn_blocking(move || {
        git.histories.run_lines(
            id,
            &git.repos,
            file,
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

/// 메시지·작성자·sha로 `from` 다음(`forward`가 아니면 이전) 행부터 찾는다. 없으면 `null`.
/// 같은 그래프에서 새 검색을 시작하면 이 검색은 "취소했다" 오류로 끝난다.
#[tauri::command]
pub async fn graph_search(
    git: State<'_, Arc<GitState>>,
    id: u64,
    query: String,
    from: usize,
    forward: bool,
) -> Result<Option<usize>, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || git.graphs.search(id, &query, from, forward))
        .await
        .map_err(|e| e.to_string())?
}

/// 경로(파일·디렉터리, git pathspec)를 바꾼 커밋의 행들(오름차순).
#[tauri::command]
pub async fn graph_search_path(
    git: State<'_, Arc<GitState>>,
    id: u64,
    path: String,
) -> Result<Vec<usize>, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || git.graphs.search_path(id, &path))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Watching {
    id: u64,
    /// 작업트리 경로(`repo-changed` 이벤트의 `repo`와 같다) 또는 파일 경로(`file-changed`의 `path`와 같다)
    target: String,
}

/// `repo`(저장소 안의 아무 경로)의 HEAD·참조 변화를 지켜본다. 바뀌면 `repo-changed` 이벤트가 온다.
/// 화면을 떠나면 `unwatch_repo`로 끝낸다.
#[tauri::command]
pub async fn watch_repo(git: State<'_, Arc<GitState>>, repo: String) -> Result<Watching, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        git.watchers
            .watch(&git.repos, Path::new(&repo))
            .map(|(id, target)| Watching { id, target })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 파일(작업트리 blame으로 연 파일)이 바뀌면 `file-changed` 이벤트가 온다. `unwatch_repo`로 끝낸다.
#[tauri::command]
pub async fn watch_file(git: State<'_, Arc<GitState>>, path: String) -> Result<Watching, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        git.watchers
            .watch_file(Path::new(&path))
            .map(|(id, target)| Watching { id, target })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 저장소의 워크트리들(주 작업트리 먼저, 링크된 워크트리는 이름 순).
#[tauri::command]
pub async fn worktrees(
    git: State<'_, Arc<GitState>>,
    repo: String,
) -> Result<Vec<plx_core::worktree::Worktree>, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repo = git
            .repos
            .for_path(Path::new(&repo))
            .map_err(|e| e.to_string())?;
        plx_core::worktree::list(&repo).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Quick Open: 추적 파일을 `query`로 찾아 점수 순 앞 `limit`개(기본 100). 같은 저장소에 새 검색이 오면
/// 이 검색은 "취소했다" 오류로 끝난다. 빈 검색어는 최근 연 파일(`files_touch`)부터 보인다.
#[tauri::command]
pub async fn files_search(
    git: State<'_, Arc<GitState>>,
    repo: String,
    query: String,
    limit: Option<usize>,
) -> Result<FileSearch, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repo = git
            .repos
            .for_path(Path::new(&repo))
            .map_err(|e| e.to_string())?;
        git.files.search(&repo, &query, limit.unwrap_or(100))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 파일(저장소 기준 경로)을 최근 연 파일로 기억한다. blame·히스토리 화면이 열 때 부른다.
#[tauri::command]
pub async fn files_touch(
    git: State<'_, Arc<GitState>>,
    repo: String,
    path: String,
) -> Result<(), String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        git.files.touch(&repo, &path);
        Ok(())
    })
    .await
}

/// 파일 트리: `dir`(저장소 기준, `/` 구분, 루트는 빈 문자열) 바로 아래의 추적 폴더·파일. 목록은 Quick Open과
/// 같은 인덱스이고 인덱스가 바뀌었으면 다시 읽는다.
#[tauri::command]
pub async fn files_dir(
    git: State<'_, Arc<GitState>>,
    repo: String,
    dir: String,
) -> Result<DirListing, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repo = git
            .repos
            .for_path(Path::new(&repo))
            .map_err(|e| e.to_string())?;
        git.files.list_dir(&repo, &dir)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoState {
    /// 작업트리 경로
    repo: String,
    /// 작업트리 폴더 이름
    name: String,
    /// 체크아웃한 브랜치. detached면 `null`
    branch: Option<String>,
    /// HEAD 커밋. 커밋이 하나도 없으면 `null`
    head: Option<String>,
}

/// 작업 공간의 머리줄·상태 막대: `path`를 담은 저장소의 작업트리와 HEAD(브랜치, 커밋).
#[tauri::command]
pub async fn repo_state(git: State<'_, Arc<GitState>>, path: String) -> Result<RepoState, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&path))?;
        let workdir = repo.workdir();
        Ok(RepoState {
            repo: workdir.display().to_string(),
            name: workdir.file_name().map_or_else(
                || workdir.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            branch: repo.head_branch()?,
            head: repo.head()?.map(|id| id.to_string()),
        })
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    /// 에디터 이름(VS Code, Zed …)
    editor: String,
    /// 연 줄(리비전의 줄을 작업트리 파일의 줄로 옮긴 것). 폴더면 `null`.
    line: Option<u32>,
}

/// 작업트리 파일을 에디터(`PLX_EDITOR`, PATH의 code·zed …)로 연다. `path`(저장소 기준)가 없으면 작업트리 폴더를
/// 연다. `rev`의 `line`은 작업트리 파일의 줄로 옮긴다. 환경 변수는 마지막 `plx` 호출의 셸 값을 먼저 본다.
/// 저장소는 바꾸지 않는다.
#[tauri::command]
pub async fn open_in_editor(
    git: State<'_, Arc<GitState>>,
    requests: State<'_, Arc<crate::instance::Requests>>,
    repo: String,
    path: Option<String>,
    rev: Option<String>,
    line: Option<u32>,
) -> Result<Opened, String> {
    let git = git.inner().clone();
    let env = requests.editor_env();
    tauri::async_runtime::spawn_blocking(move || {
        let repo = git
            .repos
            .for_path(Path::new(&repo))
            .map_err(|e| e.to_string())?;
        let target = plx_core::editor::locate(&repo, path.as_deref(), rev.as_deref(), line)
            .map_err(|e| e.to_string())?;
        let editor = plx_core::editor::Editor::find(&env)?;
        editor
            .open(&target)
            .map_err(|e| format!("{}를 띄우지 못했다: {e}", editor.name))?;
        Ok(Opened {
            editor: editor.name,
            line: target.line,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// blame 줄의 AI 출처(git-ai의 `refs/notes/ai`): (커밋, 그 커밋에서의 경로)마다 에이전트·사람이 쓴 줄 구간
/// (그 커밋 버전의 줄). 저장소에 노트가 없으면 `notes: false`.
#[tauri::command]
pub async fn ai_attribution(
    git: State<'_, Arc<GitState>>,
    repo: String,
    items: Vec<AiItem>,
) -> Result<AiAttribution, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        git.ai.attribution(&repo, &items)
    })
    .await
}

/// 감시 구독(저장소·파일)을 끝낸다. 이미 끝났으면 `false`. OS 감시를 버리는 일(macOS는 스레드를 기다린다)이
/// 메인 스레드를 막지 않게 비동기 명령이다.
#[tauri::command]
pub async fn unwatch_repo(git: State<'_, Arc<GitState>>, id: u64) -> Result<bool, String> {
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || git.watchers.unwatch(id))
        .await
        .map_err(|e| e.to_string())
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

/// `commit_detail`이 시간 안에 세지 못한 파일들의 줄 수(요청 순서대로). 큰 커밋은 목록을 먼저 보이고 이것을 받는다.
#[tauri::command]
pub async fn file_stats(
    git: State<'_, Arc<GitState>>,
    repo: String,
    files: Vec<StatRequest>,
) -> Result<Vec<FileStat>, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        commit::file_stats(&repo, &files)
    })
    .await
}

/// 파일 하나의 두 버전(diff 화면). blob id가 없으면 빈 쪽이다(추가·삭제). `path`는 `diff` 속성을 볼 경로,
/// `full`이면 큰 파일도 내용을 보낸다.
#[tauri::command]
pub async fn file_versions(
    git: State<'_, Arc<GitState>>,
    repo: String,
    old_id: Option<String>,
    new_id: Option<String>,
    path: Option<String>,
    full: Option<bool>,
) -> Result<FileVersions, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        commit::file_versions(
            &repo,
            old_id.as_deref(),
            new_id.as_deref(),
            path.as_deref(),
            full.unwrap_or(false),
        )
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
