//! 메인 창 보이기·숨기기. 앱은 상주하므로 창을 닫아도 숨기기만 한다.

use tauri::{AppHandle, Manager, Window};

pub const MAIN: &str = "main";

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

pub fn set_title(app: &AppHandle, title: &str) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.set_title(&format!("{title} — parallax"));
    }
}
