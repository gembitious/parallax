//! `plx` 호출을 받아 앱에 전달한다. 프로세스 사이 전송은 `plx_core::instance`가 맡는다.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use plx_core::cli::{self, Parsed, Request};
use plx_core::instance::{self, Claim, Endpoint, Invocation, PROTOCOL_VERSION, Reply, Server};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::window;

pub const APP_ID: &str = "com.gembitious.parallax";

/// 분리 실행된 자식 프로세스 표시. 자식은 다시 분리하지 않는다.
const ENV_DETACHED: &str = "PLX_DETACHED";
/// 분리 실행 강제: `1`이면 항상, `0`이면 하지 않는다 (테스트·스크립트용).
const ENV_DETACH: &str = "PLX_DETACH";
/// 호출 시작 시각(epoch ms). 분리 실행 전 프로세스나 벤치 스크립트가 넘긴다.
const ENV_T0: &str = "PLX_T0";

/// 상주 인스턴스가 막 뜨는 중일 때 기다리는 한도. 콜드 스타트보다 넉넉해야 한다.
const CLAIM_TIMEOUT: Duration = Duration::from_secs(10);

/// 프론트엔드로 보내는 요청. `plx` 호출 하나에 해당한다.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlxRequest {
    pub id: u64,
    /// 파싱한 뷰 요청.
    pub request: Request,
    /// 창 제목·화면 표시용 요약.
    pub label: String,
    pub args: Vec<String>,
    pub cwd: String,
    /// 호출 시작 시각(epoch ms).
    pub t0_ms: f64,
    /// 이 인스턴스가 받은 시각(epoch ms).
    pub recv_ms: f64,
    /// 콜드 스타트(이 프로세스를 띄운 호출)인지.
    pub cold: bool,
}

/// 받은 요청을 앱이 준비되면 창에 전달한다. 준비 전에 온 요청은 모아 뒀다가 넘긴다.
#[derive(Default)]
pub struct Requests {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    app: Option<AppHandle>,
    pending: bool,
    latest: Option<PlxRequest>,
    next_id: u64,
}

impl Requests {
    /// 받은 호출을 파싱해 창에 보낸다. 인자가 잘못됐으면 호출한 쪽에 보여 줄 오류 문장을 돌려준다.
    pub fn submit(&self, invocation: Invocation, cold: bool) -> Result<(), String> {
        let recv_ms = plx_core::time::epoch_ms();
        let cwd = Path::new(&invocation.cwd);
        let request = match cli::parse(&invocation.args, cwd, cli::stat) {
            Ok(Parsed::Request(request)) => request,
            // 도움말·버전은 호출한 프로세스가 직접 출력하고 넘기지 않는다.
            Ok(Parsed::Help | Parsed::Version) => return Err("넘길 요청이 없다".to_owned()),
            Err(e) => return Err(e.to_string()),
        };
        let label = request.label(cwd);
        let mut inner = self.inner.lock().unwrap();
        inner.next_id += 1;
        let request = PlxRequest {
            id: inner.next_id,
            request,
            label,
            args: invocation.args,
            cwd: invocation.cwd,
            t0_ms: invocation.t0_ms,
            recv_ms,
            cold,
        };
        inner.latest = Some(request.clone());
        match inner.app.clone() {
            Some(app) => {
                drop(inner);
                deliver(&app, &request, true);
            }
            None => inner.pending = true,
        }
        Ok(())
    }

    /// 앱 초기화(setup)에서 부른다. 그 전에 온 요청 중 마지막 것만 보여 주면 된다.
    pub fn attach(&self, app: AppHandle) {
        let mut inner = self.inner.lock().unwrap();
        inner.app = Some(app.clone());
        let pending = std::mem::take(&mut inner.pending)
            .then(|| inner.latest.clone())
            .flatten();
        drop(inner);
        if let Some(request) = pending {
            // 콜드 스타트: 창은 프론트엔드가 첫 화면을 그린 뒤(app_ready) 보인다.
            deliver(&app, &request, window::is_ready());
        }
    }

    pub fn latest(&self) -> Option<PlxRequest> {
        self.inner.lock().unwrap().latest.clone()
    }
}

fn deliver(app: &AppHandle, request: &PlxRequest, show: bool) {
    window::set_title(app, &request.label);
    // 프론트엔드가 아직 듣기 전이면 이벤트는 사라지지만, 뜨자마자 current_request로 가져간다.
    let _ = app.emit_to(window::MAIN, "plx-request", request);
    // 이벤트를 먼저 보내 숨겨진 창이 보이기 전에 새 화면을 그릴 기회를 준다.
    if show {
        window::show_main(app);
    }
}

/// 프론트엔드가 뜨면서 놓친 요청을 가져간다.
#[tauri::command]
pub fn current_request(requests: tauri::State<'_, Arc<Requests>>) -> Option<PlxRequest> {
    requests.latest()
}

/// 프로세스 맨 앞에서 부른다. 이 프로세스가 상주 인스턴스가 되면 돌아오고,
/// 실행 중인 인스턴스에 넘겼거나 분리 실행했으면 여기서 프로세스를 끝낸다.
pub fn start(t_main_ms: f64, requests: Arc<Requests>) -> Server {
    let detached = std::env::var_os(ENV_DETACHED).is_some();
    let detach = std::env::var(ENV_DETACH).ok();
    // 측정 기준점: 넘겨받은 시각, 없으면 OS가 이 프로세스를 만든 시각(실행 파일 로딩 포함).
    // 단, `cargo run`(tauri dev)은 Unix에서 exec로 자기 자신을 plx로 바꾸므로 생성 시각에 빌드 시간이
    // 들어간다. cargo가 넣어 주는 CARGO 변수가 있으면 main 시각을 쓴다.
    let t0_ms = std::env::var(ENV_T0)
        .ok()
        .and_then(|v| v.parse().ok())
        .or_else(|| {
            std::env::var_os("CARGO")
                .is_none()
                .then(plx_core::time::process_start_ms)
                .flatten()
        })
        .unwrap_or(t_main_ms);
    // 이 값들이 앱이 띄우는 프로세스(에디터 → 그 안의 터미널 → plx)로 새지 않게 지운다.
    // SAFETY: main 맨 앞, 다른 스레드가 생기기 전이다.
    unsafe {
        for key in [ENV_DETACHED, ENV_DETACH, ENV_T0] {
            std::env::remove_var(key);
        }
    }

    let invocation = current_invocation(t0_ms).unwrap_or_else(|msg| fail(&msg, 2));
    // 잘못된 인자는 상주 인스턴스를 건드리지 않고 이 터미널에서 바로 알린다.
    match cli::parse(&invocation.args, Path::new(&invocation.cwd), cli::stat) {
        Ok(Parsed::Request(_)) => {}
        Ok(Parsed::Help) => print_and_exit(cli::USAGE),
        Ok(Parsed::Version) => print_and_exit(&format!("plx {}", env!("CARGO_PKG_VERSION"))),
        Err(e) => fail(&format!("{e}\n자세한 사용법: plx --help"), 2),
    }
    let flavor = cfg!(debug_assertions).then_some("dev");
    let endpoint = Endpoint::new(APP_ID, flavor)
        .unwrap_or_else(|e| fail(&format!("인스턴스 경로를 준비하지 못했다: {e}"), 1));

    let lock = match instance::claim(&endpoint, &invocation, CLAIM_TIMEOUT) {
        Ok(Claim::Forwarded(reply)) if reply.ok => std::process::exit(0),
        Ok(Claim::Forwarded(reply)) => fail(reply.error.as_deref().unwrap_or("알 수 없는 오류"), 1),
        Ok(Claim::Primary(lock)) => lock,
        Err(e) => fail(&e.to_string(), 1),
    };

    if should_detach(detached, detach.as_deref()) {
        // 락을 놓고 분리된 자식이 다시 잡게 한다. 그 사이 다른 호출이 primary가 되면 자식은 그쪽에 넘긴다.
        drop(lock);
        let t0 = t0_ms.to_string();
        let envs = [(ENV_DETACHED, "1"), (ENV_T0, t0.as_str())];
        match instance::spawn_detached(&invocation.args, Path::new(&invocation.cwd), &envs) {
            Ok(()) => std::process::exit(0),
            Err(e) => fail(&format!("백그라운드 실행에 실패했다: {e}"), 1),
        }
    }

    if let Err(message) = requests.submit(invocation, true) {
        fail(&message, 2);
    }
    lock.serve(move |invocation| match requests.submit(invocation, false) {
        Ok(()) => Reply::ok(),
        Err(message) => Reply::error(message),
    })
    .unwrap_or_else(|e| fail(&format!("인스턴스 소켓을 열지 못했다: {e}"), 1))
}

fn should_detach(detached: bool, detach: Option<&str>) -> bool {
    if detached {
        return false;
    }
    match detach {
        Some("1") => true,
        Some("0") => false,
        // 개발 빌드는 `tauri dev`가 자식 프로세스로 붙잡고 있어야 하므로 분리하지 않는다.
        _ => !cfg!(debug_assertions) && instance::launched_from_shell(),
    }
}

fn current_invocation(t0_ms: f64) -> Result<Invocation, String> {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string().map_err(|arg| {
                format!("인자가 유효한 유니코드가 아니다: {}", arg.to_string_lossy())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cwd = std::env::current_dir()
        .map_err(|e| format!("현재 디렉터리를 알 수 없다: {e}"))?
        .into_os_string()
        .into_string()
        .map_err(|cwd| {
            format!(
                "현재 디렉터리 경로가 유효한 유니코드가 아니다: {}",
                cwd.to_string_lossy()
            )
        })?;
    Ok(Invocation {
        version: PROTOCOL_VERSION,
        args,
        cwd,
        t0_ms,
        pid: std::process::id(),
    })
}

fn print_and_exit(text: &str) -> ! {
    cli::attach_parent_console();
    println!("{text}");
    std::process::exit(0)
}

fn fail(message: &str, code: i32) -> ! {
    cli::attach_parent_console();
    eprintln!("plx: {message}");
    std::process::exit(code)
}
