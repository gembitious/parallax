//! macOS 메뉴 막대. 프런트엔드가 메뉴 모형(src/menu/model.ts의 `toNative`)을 보내면 시스템 메뉴로 만들어 바꿔 끼운다.
//! 항목을 누르면 "menu-action" 이벤트로 그 id를 돌려주고 프런트엔드가 실행한다(src/menu/native.ts). 단축키(⌘B 등)는
//! 페이지가 먼저 받는다: WebKit은 ⌘ 키를 페이지에 먼저 주고 처리하지 않은 것만 메뉴로 넘긴다. Windows·Linux는 도구
//! 막대의 HTML 메뉴를 쓰므로 여기서는 아무것도 하지 않는다.

use serde::Deserialize;
use tauri::AppHandle;

/// 메뉴 막대 항목 id의 머리. 트레이 메뉴(show, quit …)와 섞이지 않게 붙인다.
const ID_PREFIX: &str = "plx:";

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct MenuDto {
    title: String,
    role: Option<Role>,
    items: Vec<ItemDto>,
}

/// 창 목록이 붙는 윈도우 메뉴, 검색 칸이 붙는 도움말 메뉴
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum Role {
    Window,
    Help,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum ItemDto {
    Item {
        id: String,
        label: String,
        accelerator: Option<String>,
        enabled: bool,
    },
    Separator,
    Submenu {
        label: String,
        items: Vec<ItemDto>,
    },
    Predefined {
        name: Predefined,
        label: String,
    },
}

/// 시스템이 하는 항목(이름은 src/menu/model.ts의 Predefined)
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum Predefined {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Zoom,
    Fullscreen,
    BringAllToFront,
}

/// 메뉴 막대를 바꿔 끼운다. 동기 명령이라 메인 스레드에서 돈다(메뉴 항목은 메인 스레드에서 만든다).
#[tauri::command]
pub fn set_app_menu(app: AppHandle, menus: Vec<MenuDto>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let menu = macos::build(&app, &menus).map_err(|e| e.to_string())?;
        app.set_menu(menu).map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, menus);
    Ok(())
}

/// 메뉴 막대의 항목을 눌렀다: 그 id를 프런트엔드에 알린다. 트레이 메뉴의 항목은 트레이가 처리한다.
pub fn on_menu_event(app: &AppHandle, id: &str) {
    if let Some(id) = id.strip_prefix(ID_PREFIX) {
        use tauri::Emitter;
        let _ = app.emit("menu-action", id);
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use tauri::menu::{
        AboutMetadata, HELP_SUBMENU_ID, Menu, MenuItem, PredefinedMenuItem, Submenu,
        WINDOW_SUBMENU_ID,
    };
    use tauri::{AppHandle, Wry};

    use super::{ID_PREFIX, ItemDto, MenuDto, Predefined, Role};

    pub fn build(app: &AppHandle, menus: &[MenuDto]) -> tauri::Result<Menu<Wry>> {
        let menu = Menu::new(app)?;
        for (i, dto) in menus.iter().enumerate() {
            // 윈도우·도움말 메뉴는 Tauri가 정한 id로 만들어야 시스템이 창 목록·검색 칸을 붙인다(set_menu).
            let id = match dto.role {
                Some(Role::Window) => WINDOW_SUBMENU_ID.to_owned(),
                Some(Role::Help) => HELP_SUBMENU_ID.to_owned(),
                None => format!("{ID_PREFIX}menu{i}"),
            };
            let submenu = Submenu::with_id(app, id, &dto.title, true)?;
            append(app, &submenu, &dto.items)?;
            menu.append(&submenu)?;
        }
        Ok(menu)
    }

    fn append(app: &AppHandle, submenu: &Submenu<Wry>, items: &[ItemDto]) -> tauri::Result<()> {
        for item in items {
            match item {
                ItemDto::Item {
                    id,
                    label,
                    accelerator,
                    enabled,
                } => submenu.append(&MenuItem::with_id(
                    app,
                    format!("{ID_PREFIX}{id}"),
                    label,
                    *enabled,
                    accelerator.as_deref(),
                )?)?,
                ItemDto::Separator => submenu.append(&PredefinedMenuItem::separator(app)?)?,
                ItemDto::Submenu { label, items } => {
                    let nested = Submenu::new(app, label, true)?;
                    append(app, &nested, items)?;
                    submenu.append(&nested)?;
                }
                ItemDto::Predefined { name, label } => {
                    submenu.append(&predefined(app, *name, label)?)?
                }
            }
        }
        Ok(())
    }

    fn predefined(
        app: &AppHandle,
        name: Predefined,
        label: &str,
    ) -> tauri::Result<PredefinedMenuItem<Wry>> {
        let text = Some(label);
        match name {
            Predefined::About => {
                let info = app.package_info();
                let metadata = AboutMetadata {
                    name: Some(info.name.clone()),
                    version: Some(info.version.to_string()),
                    copyright: Some("Copyright (c) 2026 gembitious · MIT OR Apache-2.0".to_owned()),
                    ..Default::default()
                };
                PredefinedMenuItem::about(app, text, Some(metadata))
            }
            Predefined::Services => PredefinedMenuItem::services(app, text),
            Predefined::Hide => PredefinedMenuItem::hide(app, text),
            Predefined::HideOthers => PredefinedMenuItem::hide_others(app, text),
            Predefined::ShowAll => PredefinedMenuItem::show_all(app, text),
            Predefined::Quit => PredefinedMenuItem::quit(app, text),
            Predefined::Undo => PredefinedMenuItem::undo(app, text),
            Predefined::Redo => PredefinedMenuItem::redo(app, text),
            Predefined::Cut => PredefinedMenuItem::cut(app, text),
            Predefined::Copy => PredefinedMenuItem::copy(app, text),
            Predefined::Paste => PredefinedMenuItem::paste(app, text),
            Predefined::SelectAll => PredefinedMenuItem::select_all(app, text),
            Predefined::Minimize => PredefinedMenuItem::minimize(app, text),
            Predefined::Zoom => PredefinedMenuItem::maximize(app, text),
            Predefined::Fullscreen => PredefinedMenuItem::fullscreen(app, text),
            Predefined::BringAllToFront => PredefinedMenuItem::bring_all_to_front(app, text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// src/menu/model.ts의 toNative가 보내는 모양을 읽는다.
    #[test]
    fn reads_native_menu_model() {
        let json = r#"[
          {"title":"파일","items":[
            {"kind":"item","id":"파일/저장소 열기…","label":"저장소 열기…","accelerator":"CmdOrCtrl+O","enabled":true},
            {"kind":"submenu","label":"최근 저장소","items":[{"kind":"item","id":"파일/최근 저장소/없음","label":"없음","enabled":false}]},
            {"kind":"separator"}
          ]},
          {"title":"편집","items":[{"kind":"predefined","name":"selectAll","label":"모두 선택"}]},
          {"title":"윈도우","role":"window","items":[{"kind":"predefined","name":"bringAllToFront","label":"모두 앞으로 가져오기"}]},
          {"title":"도움말","role":"help","items":[]}
        ]"#;
        let menus: Vec<MenuDto> = serde_json::from_str(json).expect("메뉴 모형");
        assert_eq!(menus.len(), 4);
        assert!(matches!(menus[2].role, Some(Role::Window)));
        assert!(matches!(menus[3].role, Some(Role::Help)));
        match &menus[0].items[0] {
            ItemDto::Item {
                id,
                accelerator,
                enabled,
                ..
            } => {
                assert_eq!(id, "파일/저장소 열기…");
                assert_eq!(accelerator.as_deref(), Some("CmdOrCtrl+O"));
                assert!(*enabled);
            }
            _ => panic!("항목이어야 한다"),
        }
        assert!(matches!(&menus[0].items[1], ItemDto::Submenu { items, .. } if items.len() == 1));
        assert!(matches!(
            menus[1].items[0],
            ItemDto::Predefined {
                name: Predefined::SelectAll,
                ..
            }
        ));
    }
}
