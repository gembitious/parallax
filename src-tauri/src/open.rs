//! 앱 안에서 저장소 열기(첫 화면, Ctrl+O·⌘O, 끌어 놓기, macOS Dock, 트레이), 최근 저장소, 터미널 명령 설치.
//! 연 대상은 `plx`를 그 폴더에서 부른 것과 같은 요청으로 창에 보낸다(`Requests::open`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use plx_core::cli::{self, PathKind, Request};
use plx_core::cli_install;
use plx_core::recent::Recent;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::git::GitState;
use crate::instance::{PlxRequest, Requests};
use crate::window;

/// 최근 저장소가 바뀌었다(화면이 목록을 다시 읽는다).
const RECENT_CHANGED: &str = "recent-changed";
/// 결과를 돌려받을 호출이 없는 일(Dock에 놓기, 트레이 메뉴)의 결과를 화면 아래에 알린다.
const NOTICE: &str = "notice";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Notice {
    text: String,
    error: bool,
}

fn notify(app: &AppHandle, text: String, error: bool) {
    let _ = app.emit_to(window::MAIN, NOTICE, Notice { text, error });
}

/// 폴더면 그 저장소의 그래프, 파일이면 작업트리 blame(`plx <파일>`과 같다). 저장소가 아니면 그 까닭을 돌려준다.
fn open(app: &AppHandle, path: &Path) -> Result<(), String> {
    let (request, cwd) = match cli::stat(path) {
        PathKind::Dir => (Request::Graph, path.to_path_buf()),
        PathKind::File => (
            Request::Blame {
                path: path.to_path_buf(),
                line: None,
            },
            path.parent().unwrap_or(path).to_path_buf(),
        ),
        PathKind::Missing => return Err(format!("없는 경로다: {}", path.display())),
        PathKind::Unreadable(kind) => {
            return Err(format!("볼 수 없는 경로다: {} ({kind})", path.display()));
        }
    };
    // 화면이 곧 같은 저장소를 열므로 앱의 저장소 캐시로 열어 본다.
    app.state::<Arc<GitState>>()
        .repo_root(request.repo_path(&cwd))
        .map_err(|e| e.to_string())?;
    let cwd = cwd.into_os_string().into_string().map_err(|cwd| {
        format!(
            "경로가 유효한 유니코드가 아니라 열 수 없다: {}",
            cwd.to_string_lossy()
        )
    })?;
    app.state::<Arc<Requests>>().open(request, cwd);
    Ok(())
}

/// 창에 보낸 요청의 저장소를 최근 저장소 맨 앞에 둔다. 저장소 찾기와 쓰기는 파일을 읽고 쓰므로 따로 스레드에서 한다.
pub fn remember(app: &AppHandle, request: &PlxRequest) {
    let (Some(git), Some(recent)) = (
        app.try_state::<Arc<GitState>>(),
        app.try_state::<Arc<Recent>>(),
    ) else {
        return;
    };
    let (git, recent) = (git.inner().clone(), recent.inner().clone());
    let path = request
        .request
        .repo_path(Path::new(&request.cwd))
        .to_path_buf();
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(root) = git
            .repo_root(&path)
            .ok()
            .and_then(|root| root.into_os_string().into_string().ok())
        else {
            return;
        };
        match recent.add(&root) {
            Ok(true) => {
                let _ = app.emit_to(window::MAIN, RECENT_CHANGED, ());
            }
            Ok(false) => {}
            Err(e) => eprintln!("plx: 최근 저장소를 쓰지 못했다: {e}"),
        }
    });
}

/// 앱 데이터 디렉터리(최근 저장소). `PLX_DATA_DIR`로 바꿀 수 있다(자동 확인이 사용자 목록과 섞이지 않게).
pub fn data_dir(app: &tauri::App) -> Option<PathBuf> {
    std::env::var_os("PLX_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| app.path().app_data_dir().ok())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentRepo {
    path: String,
    /// 폴더 이름
    name: String,
    /// 화면에 보일 경로(홈 디렉터리는 `~`)
    display: String,
    /// 폴더가 없다(지웠거나 옮겼거나 꺼낸 볼륨)
    missing: bool,
}

#[tauri::command]
pub async fn recent_repos(recent: State<'_, Arc<Recent>>) -> Result<Vec<RecentRepo>, String> {
    let recent = recent.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let home = home_dir();
        recent
            .list()
            .into_iter()
            .map(|path| {
                let p = Path::new(&path);
                RecentRepo {
                    name: p
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.clone()),
                    display: display_path(p, home.as_deref()),
                    missing: !p.is_dir(),
                    path,
                }
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn forget_recent(
    app: AppHandle,
    recent: State<'_, Arc<Recent>>,
    path: String,
) -> Result<(), String> {
    let recent = recent.inner().clone();
    let changed = tauri::async_runtime::spawn_blocking(move || recent.remove(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("최근 저장소를 쓰지 못했다: {e}"))?;
    if changed {
        let _ = app.emit_to(window::MAIN, RECENT_CHANGED, ());
    }
    Ok(())
}

/// 끌어 놓은 폴더·파일이나 최근 저장소를 연다.
#[tauri::command]
pub async fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || open(&app, Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

/// OS 폴더 창으로 저장소를 골라 연다. 고르지 않고 닫으면 false.
#[tauri::command]
pub async fn open_dialog(app: AppHandle) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || pick_and_open(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// 메인 스레드가 아닌 곳에서 부른다(창이 닫힐 때까지 기다린다).
fn pick_and_open(app: &AppHandle) -> Result<bool, String> {
    let mut dialog = app.dialog().file().set_title("저장소 열기");
    if let Some(window) = app.get_webview_window(window::MAIN) {
        dialog = dialog.set_parent(&window);
    }
    // 저장소는 한 폴더에 모아 두는 일이 많아 가장 최근 저장소의 상위 폴더에서 시작한다.
    if let Some(parent) = app
        .try_state::<Arc<Recent>>()
        .and_then(|recent| recent.list().into_iter().next())
        .and_then(|path| Path::new(&path).parent().map(Path::to_path_buf))
        .filter(|parent| parent.is_dir())
    {
        dialog = dialog.set_directory(parent);
    }
    let Some(folder) = dialog.blocking_pick_folder() else {
        return Ok(false);
    };
    let path = folder.into_path().map_err(|e| e.to_string())?;
    open(app, &path)?;
    Ok(true)
}

/// 트레이의 "저장소 열기…": 창을 보이고(폴더 창이 그 창에 붙는다) 폴더 창을 띄운다.
pub fn open_from_tray(app: &AppHandle) {
    window::show_main(app);
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = pick_and_open(&app) {
            notify(&app, e, true);
        }
    });
}

/// 앱을 띄운 Apple 이벤트(꺼진 앱의 Dock 아이콘에 놓은 폴더)는 setup 전에 온다: AppKit은
/// applicationDidFinishLaunching 전에 문서를 열고, tao는 그 이벤트를 바로 넘긴다. 그때는 저장소 캐시가 아직 없어
/// setup 끝([`open_pending`])에서 연다. 둘 다 메인 스레드에서 돈다.
#[cfg(target_os = "macos")]
static PENDING: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// macOS: Dock 아이콘에 놓은 폴더, `open -a parallax <폴더>`(`RunEvent::Opened`). 여럿이면 첫 것만 연다.
#[cfg(target_os = "macos")]
pub fn opened(app: &AppHandle, urls: Vec<tauri::Url>) {
    let Some(path) = urls.into_iter().find_map(|url| url.to_file_path().ok()) else {
        return;
    };
    let mut pending = PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if app.try_state::<Arc<GitState>>().is_none() {
        *pending = Some(path);
        return;
    }
    drop(pending);
    open_in_background(app, path);
}

/// setup 끝에서 부른다: 그 전에 온 Apple 이벤트의 폴더를 연다.
#[cfg(target_os = "macos")]
pub fn open_pending(app: &AppHandle) {
    let path = PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(path) = path {
        open_in_background(app, path);
    }
}

#[cfg(target_os = "macos")]
fn open_in_background(app: &AppHandle, path: PathBuf) {
    use std::time::{Duration, Instant};
    let app = app.clone();
    std::thread::spawn(move || {
        let Err(e) = open(&app, &path) else {
            return;
        };
        // 꺼져 있던 앱의 Dock 아이콘에 놓았으면 첫 화면을 그린 뒤에 알린다. 그 전에는 알림을 들을 화면이 없고, 창은
        // 그린 뒤에 보인다(빈 창 깜빡임).
        if !window::is_ready() {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !window::is_ready() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            // 준비를 알린 바로 뒤라 화면이 알림을 막 듣기 시작했을 수 있다.
            std::thread::sleep(Duration::from_millis(200));
        }
        window::show_main(&app);
        notify(&app, e, true);
    });
}

#[tauri::command]
pub async fn cli_status() -> Result<cli_install::Status, String> {
    tauri::async_runtime::spawn_blocking(cli_install::status)
        .await
        .map_err(|e| e.to_string())
}

/// 터미널 명령을 설치한다. macOS는 관리자 암호 창을 띄울 수 있어 창이 닫힐 때까지 기다린다.
#[tauri::command]
pub async fn install_cli() -> Result<cli_install::Status, String> {
    tauri::async_runtime::spawn_blocking(cli_install::install)
        .await
        .map_err(|e| e.to_string())?
}

/// 트레이의 "터미널 명령 설치…": 결과는 창 아래에 알린다.
pub fn install_from_tray(app: &AppHandle) {
    window::show_main(app);
    let app = app.clone();
    std::thread::spawn(move || match cli_install::install() {
        Ok(status) => {
            let mut text = format!("터미널 명령 plx: {}", status.location);
            if let Some(note) = status.note {
                text = format!("{text} · {note}");
            }
            notify(&app, text, false);
        }
        Err(e) => notify(&app, format!("터미널 명령을 설치하지 못했다: {e}"), true),
    });
}

fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// 홈 디렉터리 아래면 `~/…`로 줄인다(macOS·Linux).
fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_path_shortens_home() {
        let home = Path::new("/Users/나");
        assert_eq!(
            display_path(Path::new("/Users/나/code/parallax"), Some(home)),
            "~/code/parallax"
        );
        assert_eq!(display_path(Path::new("/Users/나"), Some(home)), "~");
        assert_eq!(
            display_path(Path::new("/Users/나2/x"), Some(home)),
            "/Users/나2/x"
        );
        assert_eq!(display_path(Path::new("/opt/x"), None), "/opt/x");
    }
}
