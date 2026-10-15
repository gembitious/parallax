//! 저장소 읽기 명령. 무거운 일은 블로킹 스레드에서 하고 결과는 Channel로 흘려보낸다.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use plx_core::blame_session::{SessionEvent, Sessions};
use plx_core::cache::BlameCache;
use plx_core::commit::{self, CommitDetail, FileVersions};
use plx_core::navigate::{self, Direction, Version};
use plx_core::repo::{CommitInfo, RepoCache};
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

/// blame 캐시 한도. 넘으면 오래 안 쓴 것부터 지운다.
const CACHE_MAX_BYTES: u64 = 512 << 20;

/// 앱 전체가 나눠 쓰는 저장소 캐시와 진행 중인 요청.
pub struct GitState {
    repos: RepoCache,
    blames: Sessions,
}

impl GitState {
    /// `cache_dir`가 있으면 blame 결과를 그 아래 `blame/`에 캐시한다.
    pub fn new(cache_dir: Option<PathBuf>) -> GitState {
        let blames = match cache_dir {
            Some(dir) => {
                let cache = BlameCache::new(dir.join("blame"), CACHE_MAX_BYTES);
                Sessions::with_cache(cache)
            }
            None => Sessions::new(),
        };
        GitState {
            repos: RepoCache::new(),
            blames,
        }
    }

    /// 캐시가 한도를 넘었으면 줄인다. 시작할 때 백그라운드에서 한 번 부른다.
    pub fn prune_cache(&self) {
        if let Some(cache) = self.blames.cache()
            && let Err(e) = cache.prune()
        {
            eprintln!(
                "plx: blame 캐시를 정리하지 못했다 ({}): {e}",
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
#[tauri::command]
pub async fn commit_detail(
    git: State<'_, Arc<GitState>>,
    repo: String,
    rev: String,
) -> Result<CommitDetail, String> {
    blocking(git, move |git| {
        let repo = git.repos.for_path(Path::new(&repo))?;
        let id = repo.resolve_commit(&rev)?;
        commit::commit_detail(&repo, id)
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
