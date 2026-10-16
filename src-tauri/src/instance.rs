//! `plx` 호출을 받아 앱에 전달한다. 프로세스 사이 전송은 `plx_core::instance`가 맡는다.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use plx_core::cli::{self, Parsed, Request};
use plx_core::editor::Env;
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
/// 분리 실행한 자식이 창을 띄울 준비를 마치면 만드는 파일. 부모(셸이 기다리는 프로세스)가 이것을 기다린다.
const ENV_READY: &str = "PLX_READY";
/// 상주 인스턴스가 작업 디렉터리를 떠나기 전에 절대 경로로 바꿔 둘 경로 변수(상대 경로로 줄 수 있다).
const PATH_ENVS: [&str; 2] = ["PLX_CACHE_DIR", "PLX_PERF_LOG"];

/// 상주 인스턴스가 막 뜨는 중일 때 기다리는 한도. 콜드 스타트보다 넉넉해야 한다.
const CLAIM_TIMEOUT: Duration = Duration::from_secs(10);
/// 분리 실행한 자식이 이 안에 준비되지 않으면 부모는 더 기다리지 않는다(느린 첫 실행이면 계속 뜨는 중이다).
const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// 자식이 뜨지 못하고 끝났을 때 터미널에 보일 기록의 줄 수
const LOG_TAIL_LINES: usize = 20;

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
    /// 마지막 호출의 셸 환경 변수(에디터 고르기)
    env: BTreeMap<String, String>,
    /// 분리 실행된 자식이면 준비를 마쳤을 때 만들 파일(부모가 기다린다)
    ready_file: Option<PathBuf>,
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
        inner.env = invocation.env;
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

    /// 앱 초기화(setup) 끝에서 부른다(창은 이미 있다). 그 전에 온 요청 중 마지막 것만 보여 주면 된다.
    /// 분리 실행된 자식이면 기다리는 부모에게 준비를 마쳤다고 알린다.
    pub fn attach(&self, app: AppHandle) {
        let mut inner = self.inner.lock().unwrap();
        inner.app = Some(app.clone());
        let pending = std::mem::take(&mut inner.pending)
            .then(|| inner.latest.clone())
            .flatten();
        let ready_file = inner.ready_file.take();
        drop(inner);
        if let Some(request) = pending {
            // 콜드 스타트: 창은 프론트엔드가 첫 화면을 그린 뒤(app_ready) 보인다.
            deliver(&app, &request, window::is_ready());
        }
        if let Some(path) = ready_file
            && let Err(e) = std::fs::write(&path, b"ok")
        {
            eprintln!("plx: 준비 알림을 쓰지 못했다({}): {e}", path.display());
        }
    }

    /// 분리 실행된 자식이면 준비를 마쳤을 때([`Requests::attach`]) 만들 파일.
    fn notify_ready_at(&self, path: Option<PathBuf>) {
        self.inner.lock().unwrap().ready_file = path;
    }

    pub fn latest(&self) -> Option<PlxRequest> {
        self.inner.lock().unwrap().latest.clone()
    }

    /// 에디터를 고를 환경: 마지막 `plx` 호출(지금 쓰는 터미널)의 셸 값을 먼저 본다.
    pub fn editor_env(&self) -> Env {
        Env::new(self.inner.lock().unwrap().env.clone())
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
    let ready_file = std::env::var_os(ENV_READY).map(PathBuf::from);
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
    // 상주하면 작업 디렉터리를 떠나므로(아래) 상대 경로로 준 경로 변수는 지금 절대 경로로 바꿔 둔다.
    // SAFETY: main 맨 앞, 다른 스레드가 생기기 전이다.
    unsafe {
        for key in [ENV_DETACHED, ENV_DETACH, ENV_T0, ENV_READY] {
            std::env::remove_var(key);
        }
        for key in PATH_ENVS {
            if let Some(value) = std::env::var_os(key)
                && let Ok(absolute) = std::path::absolute(&value)
            {
                std::env::set_var(key, absolute);
            }
        }
    }

    let invocation = current_invocation(t0_ms).unwrap_or_else(|msg| fail(&msg, 2));
    let cwd = PathBuf::from(&invocation.cwd);
    // 잘못된 인자는 상주 인스턴스를 건드리지 않고 이 터미널에서 바로 알린다.
    let request = match cli::parse(&invocation.args, &cwd, cli::stat) {
        Ok(Parsed::Request(request)) => request,
        Ok(Parsed::Help) => print_and_exit(cli::USAGE),
        Ok(Parsed::Version) => print_and_exit(&format!("plx {}", env!("CARGO_PKG_VERSION"))),
        Err(e) => fail(&format!("{e}\n자세한 사용법: plx --help"), 2),
    };
    // 대상 저장소도 앱과 같은 방법으로 열어 본다. 대상을 줬거나 터미널에서 불렀으면 실패를 이 터미널에 알린다.
    // 인자 없이 앱으로 띄웠으면(Finder·Dock·시작 메뉴, 작업 디렉터리가 `/`나 설치 위치) 알릴 곳이 없으니 그냥 뜨고
    // 첫 화면 안내를 보인다(저장소가 아닌 곳의 그래프 오류 대신).
    let opens_repo = match cli::check_repo(&request, &cwd) {
        Ok(()) => true,
        Err(e) if !invocation.args.is_empty() || cli::has_terminal() => fail(&e.to_string(), 2),
        Err(_) => false,
    };
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
        let ready = endpoint.dir().join(format!("ready-{}", std::process::id()));
        let log = endpoint.dir().join("launch.log");
        let _ = std::fs::remove_file(&ready);
        let t0 = t0_ms.to_string();
        let envs = [
            (ENV_DETACHED, OsStr::new("1")),
            (ENV_T0, OsStr::new(&t0)),
            (ENV_READY, ready.as_os_str()),
        ];
        match instance::spawn_detached(&invocation.args, &cwd, &envs, Some(&log)) {
            Ok(child) => wait_until_ready(child, &ready, &log),
            Err(e) => fail(&format!("백그라운드 실행에 실패했다: {e}"), 1),
        }
    }

    requests.notify_ready_at(ready_file);
    if opens_repo && let Err(message) = requests.submit(invocation, true) {
        fail(&message, 2);
    }
    leave_working_directory();
    lock.serve(move |invocation| match requests.submit(invocation, false) {
        Ok(()) => Reply::ok(),
        Err(message) => Reply::error(message),
    })
    .unwrap_or_else(|e| fail(&format!("인스턴스 소켓을 열지 못했다: {e}"), 1))
}

/// 분리한 자식이 창을 띄울 준비를 마칠 때까지(또는 끝날 때까지) 기다린 뒤 이 프로세스를 끝낸다. 자식이 뜨지 못하고
/// 끝났으면 그 기록을 이 터미널에 보이고 실패로 끝낸다(분리한 자식의 오류는 아무도 보지 않는다). 다른 호출이 먼저
/// 상주 인스턴스가 되어 자식이 그쪽에 넘기고 끝났으면 성공이다.
fn wait_until_ready(mut child: Child, ready: &Path, log: &Path) -> ! {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if ready.exists() {
            let _ = std::fs::remove_file(ready);
            std::process::exit(0);
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => std::process::exit(0),
            Ok(Some(status)) => {
                let text = std::fs::read_to_string(log).unwrap_or_default();
                let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
                let tail = lines[lines.len().saturating_sub(LOG_TAIL_LINES)..].join("\n");
                fail(
                    &format!(
                        "parallax가 뜨지 못하고 끝났다({status}). 기록: {}\n{tail}",
                        log.display()
                    ),
                    status.code().filter(|&c| c != 0).unwrap_or(1),
                )
            }
            Ok(None) => {}
            Err(e) => fail(&format!("백그라운드 실행을 확인하지 못했다: {e}"), 1),
        }
        if Instant::now() >= deadline {
            cli::attach_parent_console();
            eprintln!(
                "plx: parallax가 {}초 안에 뜨지 않았다. 아직 뜨는 중일 수 있다(기록: {})",
                READY_TIMEOUT.as_secs(),
                log.display()
            );
            std::process::exit(0);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 상주 인스턴스는 처음 부른 셸의 작업 디렉터리를 쥐고 있지 않는다: Windows에서는 그 디렉터리를 지우거나 이름을
/// 바꿀 수 없고, macOS·Linux에서는 그 볼륨을 꺼낼 수 없다(Windows는 DLL을 찾는 자리이기도 하다). 이후 경로는 모두
/// 호출마다 넘겨받은 cwd로 푼다. 실행 파일이 있는 디렉터리로 옮긴다(앱이 떠 있는 동안 어차피 쓰는 곳이다).
fn leave_working_directory() {
    let target = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(dir) = target
        && let Err(e) = std::env::set_current_dir(&dir)
    {
        eprintln!("plx: 작업 디렉터리를 옮기지 못했다({}): {e}", dir.display());
    }
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
    let args = cli::without_launcher_args(args);
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
        env: Env::capture(),
    })
}

fn print_and_exit(text: &str) -> ! {
    cli::attach_parent_console();
    println!("{text}");
    std::process::exit(0)
}

fn fail(message: &str, code: i32) -> ! {
    cli::attach_parent_console();
    // 상주 인스턴스가 보낸 문장, 파일 이름, 자식의 기록이 들어 있다. 터미널 제어 문자는 바꿔 쓴다.
    eprintln!("plx: {}", cli::printable(message));
    std::process::exit(code)
}
