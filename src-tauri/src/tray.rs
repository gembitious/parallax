//! 트레이(메뉴 막대) 아이콘. 상주 앱의 종료는 여기서만 한다. 창이 숨어 있어도 저장소를 열고 터미널 명령을 설치한다.

use tauri::App;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::{open, window};

pub fn create(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "창 보이기", true, None::<&str>)?;
    let open_repo = MenuItem::with_id(app, "open", "저장소 열기…", true, None::<&str>)?;
    let install = MenuItem::with_id(app, "install-cli", "터미널 명령 설치…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &open_repo, &install, &separator, &quit])?;

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("parallax")
        .menu(&menu)
        // macOS 메뉴 막대 아이콘은 클릭하면 메뉴를 여는 게 관례다. Windows는 왼쪽 클릭으로 창을 연다.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => window::show_main(app),
            "open" => open::open_from_tray(app),
            "install-cli" => open::install_from_tray(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if cfg!(not(target_os = "macos"))
                && let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
            {
                window::show_main(tray.app_handle());
            }
        });
    // macOS 메뉴 막대는 단색 템플릿 이미지가 관례다(시스템이 밝기·강조에 맞춰 칠한다).
    // 나머지는 앱 아이콘(Windows는 실행 파일의 아이콘 리소스, Linux는 icons/32x32.png)을 쓴다.
    if cfg!(target_os = "macos") {
        tray = tray
            .icon(tauri::include_image!("icons/tray-template.png"))
            .icon_as_template(true);
    } else if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}
