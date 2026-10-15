//! named pipe 전송 (Windows).

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GetLastError,
    HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

/// `\\.\pipe\...` 형식의 pipe 이름.
pub type SocketName = String;

const PIPE_BUFFER: u32 = 64 * 1024;

pub fn locate(app_id: &str, stem: &str) -> io::Result<(PathBuf, SocketName)> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(app_id);
    std::fs::create_dir_all(&dir)?;
    // pipe 이름은 시스템 전역이라 사용자 이름을 넣어 다른 사용자의 인스턴스와 겹치지 않게 한다.
    let user: String = std::env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    Ok((
        dir.join(format!("{stem}.lock")),
        format!(r"\\.\pipe\{app_id}.{user}.{stem}"),
    ))
}

pub fn socket_in_dir(_dir: &Path, name: &str) -> SocketName {
    format!(r"\\.\pipe\{name}")
}

pub fn connect(name: &SocketName) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(name)
}

/// pipe 인스턴스가 하나도 없는 경우(primary 없음, 또는 연결 처리 사이의 짧은 틈).
pub fn is_absent(e: &io::Error) -> bool {
    e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32)
}

/// 모든 pipe 인스턴스가 다른 연결을 처리하는 중인 경우.
pub fn is_busy(e: &io::Error) -> bool {
    e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
}

pub struct Listener {
    name: Vec<u16>,
    first: OwnedHandle,
}

impl Listener {
    pub fn bind(name: &SocketName) -> io::Result<Self> {
        let name: Vec<u16> = OsStr::new(name)
            .encode_wide()
            .chain(iter::once(0))
            .collect();
        // FIRST_PIPE_INSTANCE: 같은 이름의 서버가 이미 있으면 실패한다(락에 더한 이중 안전장치).
        let first = create_instance(&name, true)?;
        Ok(Listener { name, first })
    }

    pub fn run(self, on_conn: impl Fn(&mut File) + Send + Sync + 'static) {
        let on_conn = Arc::new(on_conn);
        let mut next = Some(self.first);
        loop {
            let pipe = match next.take() {
                Some(pipe) => pipe,
                None => match create_instance(&self.name, false) {
                    Ok(pipe) => pipe,
                    Err(e) => {
                        eprintln!("plx: named pipe 생성 실패: {e}");
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                },
            };
            // SAFETY: pipe는 열린 named pipe 핸들이다. 동기 모드라 OVERLAPPED는 넘기지 않는다.
            let connected = unsafe { ConnectNamedPipe(raw(&pipe), std::ptr::null_mut()) } != 0
                || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            if !connected {
                continue;
            }
            let on_conn = on_conn.clone();
            let spawned = thread::Builder::new()
                .name("plx-instance-conn".into())
                .spawn(move || {
                    let mut file = File::from(pipe);
                    on_conn(&mut file);
                    // SAFETY: file은 연결된 pipe 핸들이다. 클라이언트가 응답을 다 읽을 때까지 기다린 뒤 끊는다.
                    unsafe {
                        FlushFileBuffers(raw(&file));
                        DisconnectNamedPipe(raw(&file));
                    }
                });
            if let Err(e) = spawned {
                eprintln!("plx: 인스턴스 연결 스레드 생성 실패: {e}");
            }
        }
    }
}

fn raw(handle: &impl AsRawHandle) -> HANDLE {
    handle.as_raw_handle() as HANDLE
}

fn create_instance(name: &[u16], first: bool) -> io::Result<OwnedHandle> {
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: name은 NUL로 끝나는 UTF-16 문자열이다. 보안 속성은 기본값(만든 사용자와 관리자만 쓰기 가능)을 쓴다.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            std::ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 방금 만든 유효한 핸들이고 소유권을 넘겨받는다.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle as _) })
}

pub fn cleanup(_name: &SocketName) {}

/// 터미널에서 실행된 이 프로세스가 가진 포그라운드 권한을 상주 인스턴스에 넘긴다.
/// 넘기지 않으면 Windows가 창을 앞으로 가져오지 않고 작업 표시줄만 깜빡인다.
pub fn allow_foreground() {
    // SAFETY: 인자 없는 단순 호출이다. 실패해도 창을 앞으로 못 가져올 뿐이다.
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

/// 탐색기·시작 메뉴로 실행된 GUI 프로세스는 표준 핸들이 없다. Git Bash 같은 셸은 핸들을 넘겨준다.
pub fn launched_from_shell() -> bool {
    [
        io::stdin().as_raw_handle(),
        io::stdout().as_raw_handle(),
        io::stderr().as_raw_handle(),
    ]
    .into_iter()
    .any(|h| !h.is_null() && !std::ptr::eq(h, INVALID_HANDLE_VALUE))
}

pub fn spawn_detached(mut command: Command) -> io::Result<()> {
    // 셸이 속한 job(터미널을 닫으면 함께 종료되는)에서 빠져나온다. job이 허용하지 않으면 그대로 띄운다.
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
    match command.spawn() {
        Ok(_) => Ok(()),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            command.spawn().map(drop)
        }
        Err(e) => Err(e),
    }
}
