mod instance;
mod tray;
mod window;

use std::sync::Arc;

use serde::Serialize;

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

    let app = tauri::Builder::default()
        .manage(requests.clone())
        .invoke_handler(tauri::generate_handler![
            app_info,
            instance::current_request
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                window::hide(window);
            }
        })
        .setup(move |app| {
            tray::create(app)?;
            requests.attach(app.handle().clone());
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
        tauri::RunEvent::Exit => server.close(),
        _ => {}
    });
}
