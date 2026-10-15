//! 성능 측정. 개발 빌드이거나 `PLX_PERF=1`이면 켠다.
//!
//! 프론트엔드 HUD가 요청마다 단계별 시간을 그린다. 이 모듈은 기준 시각을 알려 주고,
//! `PLX_PERF_LOG`가 있으면 끝난 요청을 JSON 한 줄씩 기록한다(벤치 스크립트가 읽는다).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

use plx_core::time::epoch_ms;
use serde::Serialize;
use tauri::ipc::Channel;

const ENV_PERF: &str = "PLX_PERF";
const ENV_PERF_LOG: &str = "PLX_PERF_LOG";

pub struct Perf {
    enabled: bool,
    process_start_ms: f64,
    main_start_ms: f64,
    log: Option<Mutex<File>>,
}

impl Perf {
    pub fn from_env(main_start_ms: f64) -> Self {
        let enabled = cfg!(debug_assertions) || std::env::var(ENV_PERF).is_ok_and(|v| v == "1");
        let log = std::env::var_os(ENV_PERF_LOG)
            .filter(|_| enabled)
            .and_then(
                |path| match OpenOptions::new().create(true).append(true).open(&path) {
                    Ok(file) => Some(Mutex::new(file)),
                    Err(e) => {
                        eprintln!(
                            "plx: 성능 기록 파일을 열지 못했다 ({}): {e}",
                            path.to_string_lossy()
                        );
                        None
                    }
                },
            );
        Perf {
            enabled,
            // `cargo run`으로 실행되면 생성 시각에 빌드 시간이 들어간다(instance::start 참고).
            process_start_ms: std::env::var_os("CARGO")
                .is_none()
                .then(plx_core::time::process_start_ms)
                .flatten()
                .unwrap_or(main_start_ms),
            main_start_ms,
            log,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfInfo {
    enabled: bool,
    logging: bool,
    /// 상주 인스턴스 프로세스가 만들어진 시각(epoch ms).
    process_start_ms: f64,
    /// `main()`에 들어온 시각(epoch ms).
    main_start_ms: f64,
}

#[tauri::command]
pub fn perf_info(perf: tauri::State<'_, Perf>) -> PerfInfo {
    PerfInfo {
        enabled: perf.enabled,
        logging: perf.log.is_some(),
        process_start_ms: perf.process_start_ms,
        main_start_ms: perf.main_start_ms,
    }
}

/// HUD가 끝난 요청 하나를 넘긴다. `PLX_PERF_LOG`가 있을 때만 기록한다.
#[tauri::command]
pub fn perf_record(perf: tauri::State<'_, Perf>, entry: serde_json::Value) {
    if let Some(log) = &perf.log {
        let mut file = log.lock().unwrap();
        let _ = writeln!(file, "{entry}");
    }
}

/// IPC 왕복 측정용. 백엔드가 받은 시각을 돌려준다.
#[tauri::command]
pub fn perf_ping() -> f64 {
    epoch_ms()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeChunk {
    index: u32,
    sent_ms: f64,
    payload: String,
}

/// Channel 스트리밍 측정용. `size`바이트짜리 청크를 `count`개 보내고 끝난 시각을 돌려준다.
#[tauri::command]
pub async fn perf_stream(
    count: u32,
    size: usize,
    on_chunk: Channel<ProbeChunk>,
) -> Result<f64, String> {
    let payload = "x".repeat(size.min(1 << 20));
    for index in 0..count.min(100_000) {
        on_chunk
            .send(ProbeChunk {
                index,
                sent_ms: epoch_ms(),
                payload: payload.clone(),
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(epoch_ms())
}
