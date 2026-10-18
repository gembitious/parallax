//! 메인 창 보이기·숨기기. 앱은 상주하므로 창을 닫아도 숨기기만 한다.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, Window};

pub const MAIN: &str = "main";

/// 콜드 스타트에서 프론트엔드가 첫 화면을 그리기 전에는 창을 숨겨 둔다(빈 화면 깜빡임 방지).
static READY: AtomicBool = AtomicBool::new(false);
/// 프론트엔드가 끝내 준비를 알리지 못해도 이 시간이 지나면 창을 보인다.
const READY_FALLBACK: Duration = Duration::from_secs(3);

/// 창을 앞으로 가져온다.
pub fn show_main(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 창 닫기 대신 부른다.
pub fn hide(window: &Window) {
    let _ = window.hide();
    // macOS는 앱도 숨겨 포커스를 직전 앱(보통 터미널)으로 돌려준다.
    #[cfg(target_os = "macos")]
    let _ = window.app_handle().hide();
}

/// 화면에서 `q`(맨 앞 화면에서 뒤로)를 누르면 창 닫기처럼 숨긴다.
#[tauri::command]
pub fn hide_window(window: Window) {
    hide(&window);
}

/// 탐색으로 화면이 바뀌면 창 제목을 바꾼다.
#[tauri::command]
pub fn set_window_title(app: AppHandle, title: String) {
    set_title(&app, &title);
}

pub fn set_title(app: &AppHandle, title: &str) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.set_title(&format!("{title} — parallax"));
    }
}

/// 프론트엔드가 첫 화면을 그린 뒤 부른다.
#[tauri::command]
pub fn app_ready(app: AppHandle) {
    READY.store(true, Ordering::SeqCst);
    show_main(&app);
}

pub fn is_ready() -> bool {
    READY.load(Ordering::SeqCst)
}

pub fn show_if_not_ready(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(READY_FALLBACK);
        if !is_ready() {
            eprintln!("plx: 프론트엔드가 준비를 알리지 않아 창을 그냥 보인다");
            show_main(&app);
        }
    });
}
