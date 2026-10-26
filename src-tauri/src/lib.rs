mod git;
mod instance;
mod menu;
mod open;
mod perf;
mod tray;
mod window;

use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: &'static str,
    core_version: &'static str,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        core_version: plx_core::VERSION,
    }
}

pub fn run() {
    let t_main_ms = plx_core::time::epoch_ms();
    let requests = Arc::new(instance::Requests::default());
    // 실행 중인 인스턴스가 있으면 여기서 넘기고 끝난다. Tauri 초기화는 상주 인스턴스만 한다.
    let server = instance::start(t_main_ms, requests.clone());
    // WebView를 만드는 동안 git을 한 번 띄워 둔다: 첫 blame의 git이 실행 파일을 처음 여는 값을 치르지 않는다.
    std::thread::spawn(plx_core::git::warm_up);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(requests.clone())
        .manage(perf::Perf::from_env(t_main_ms))
        .invoke_handler(tauri::generate_handler![
            app_info,
            git::blame_open,
            git::blame_cancel,
            git::blame_map_line,
            git::blame_neighbor,
            git::history_open,
            git::history_cancel,
            git::line_history_open,
            git::graph_open,
            git::graph_rows,
            git::graph_find,
            git::graph_search,
            git::graph_search_path,
            git::graph_close,
            git::watch_repo,
            git::watch_file,
            git::watch_tree,
            git::worktrees,
            git::files_search,
            git::files_touch,
            git::files_dir,
            git::repo_state,
            git::open_in_editor,
            git::ai_attribution,
            git::unwatch_repo,
            git::repo_acceleration,
            git::accelerate,
            git::commit_info,
            git::commit_detail,
            git::compare,
            git::ref_names,
            git::file_versions,
            git::file_stats,
            instance::current_request,
            open::open_path,
            open::open_dialog,
            open::recent_repos,
            open::forget_recent,
            open::cli_status,
            open::install_cli,
            window::app_ready,
            window::hide_window,
            window::quit_app,
            window::set_window_title,
            menu::set_app_menu,
            perf::perf_info,
            perf::perf_record,
            perf::perf_ping,
            perf::perf_stream,
        ])
        .on_menu_event(|app, event| menu::on_menu_event(app, event.id().as_ref()))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                window::hide(window);
            }
        })
        .setup(move |app| {
            // blame 캐시는 OS 캐시 위치(Linux ~/.cache, macOS ~/Library/Caches, Windows %LOCALAPPDATA%)에 둔다.
            // PLX_CACHE_DIR로 바꿀 수 있다(측정·자동 확인이 사용자 캐시와 섞이지 않게).
            let cache_dir = std::env::var_os("PLX_CACHE_DIR")
                .map(std::path::PathBuf::from)
                .or_else(|| app.path().app_cache_dir().ok());
            // 지켜보는 저장소·파일이 바뀌면(외부 커밋, 에이전트가 고친 파일) 화면에 알린다.
            let handle = app.handle().clone();
            let git = Arc::new(git::GitState::new(cache_dir, move |change| {
                let _ = match change {
                    plx_core::watch::Change::Repo(change) => handle.emit("repo-changed", change),
                    plx_core::watch::Change::File(change) => handle.emit("file-changed", change),
                    plx_core::watch::Change::Tree(change) => handle.emit("tree-changed", change),
                };
            }));
            app.manage(git.clone());
            std::thread::spawn(move || git.prune_cache());
            // 최근 저장소는 앱 데이터 위치(macOS ~/Library/Application Support, Windows %APPDATA%, Linux
            // ~/.local/share)에 둔다. 요청을 보낼 때마다 고치므로 요청을 붙이기(attach) 전에 둔다.
            app.manage(Arc::new(plx_core::recent::Recent::load(
                open::data_dir(app).as_deref(),
            )));
            tray::create(app)?;
            window::show_if_not_ready(app.handle().clone());
            requests.attach(app.handle().clone());
            #[cfg(target_os = "macos")]
            open::open_pending(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("tauri 앱 초기화 실패");

    app.run(move |_app, event| match event {
        // Dock 아이콘을 눌렀는데 보이는 창이 없으면 다시 보여 준다.
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => window::show_main(_app),
        // Dock 아이콘에 놓은 폴더, `open -a parallax <폴더>`(Info.plist의 폴더 문서 형식)
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Opened { urls } => open::opened(_app, urls),
        tauri::RunEvent::Exit => server.close(),
        _ => {}
    });
}
