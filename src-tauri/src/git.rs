//! 저장소 읽기 명령. 무거운 일은 블로킹 스레드에서 하고 결과는 Channel로 흘려보낸다.

use std::path::Path;
use std::sync::Arc;

use plx_core::blame_session::{SessionEvent, Sessions};
use plx_core::repo::{CommitInfo, RepoCache};
use tauri::State;
use tauri::ipc::Channel;

/// 앱 전체가 나눠 쓰는 저장소 캐시와 진행 중인 요청.
#[derive(Default)]
pub struct GitState {
    repos: RepoCache,
    blames: Sessions,
}

/// 파일 blame을 시작하고 요청 id를 바로 돌려준다. 결과는 `on_event`로 온다
/// (`meta` → `chunk`* → `done`·`error`·`cancelled`). `rev`가 없으면 작업트리 파일이다.
#[tauri::command]
pub async fn blame_open(
    git: State<'_, Arc<GitState>>,
    path: String,
    rev: Option<String>,
    on_event: Channel<SessionEvent>,
) -> Result<u64, String> {
    let git = git.inner().clone();
    let id = git.blames.register();
    tauri::async_runtime::spawn_blocking(move || {
        git.blames
            .run(id, &git.repos, Path::new(&path), rev.as_deref(), |event| {
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
    let git = git.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let repo = git
            .repos
            .for_path(Path::new(&repo))
            .map_err(|e| e.to_string())?;
        let id = repo.resolve_commit(&rev).map_err(|e| e.to_string())?;
        repo.commit_info(id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
