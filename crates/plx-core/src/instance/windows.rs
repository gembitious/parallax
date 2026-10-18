//! named pipe 전송 (Windows).
//!
//! pipe 이름은 시스템 전역이라 이 프로세스 토큰의 사용자 SID와 세션 번호를 넣는다(환경 변수 USERNAME은 셸마다
//! 바꿀 수 있고 다른 세션의 같은 사용자와도 겹친다). 서버는 이 사용자만 열 수 있는 DACL로 pipe를 만든다(기본값은
//! Everyone에게 읽기를 준다). 클라이언트는 신원 확인 수준(SECURITY_IDENTIFICATION)으로 열어 서버가 클라이언트를
//! 가장하지 못하게 하고, 서버 프로세스의 사용자가 같은지 본 뒤에 보낸다(다른 사용자가 이름을 먼저 만들어 두고
//! 호출을 가로채지 못하게).

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::iter;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GetLastError,
    HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LocalFree, SetHandleInformation,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenSessionId, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, GetCurrentProcess,
    OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows_sys::core::PWSTR;

/// `\\.\pipe\...` 형식의 pipe 이름.
pub type SocketName = String;

const PIPE_BUFFER: u32 = 64 * 1024;

pub fn locate(app_id: &str, stem: &str) -> io::Result<(PathBuf, SocketName)> {
    let dir = local_app_data().join(app_id);
    std::fs::create_dir_all(&dir)?;
    let user = CurrentUser::get()?;
    Ok((
        dir.join(format!("{stem}.lock")),
        format!(r"\\.\pipe\{app_id}.{}.{}.{stem}", user.sid, user.session),
    ))
}

pub fn socket_in_dir(_dir: &Path, name: &str) -> SocketName {
    format!(r"\\.\pipe\{name}")
}

/// 이 사용자의 로컬 앱 데이터 디렉터리. 환경 변수 LOCALAPPDATA는 셸마다 바꿀 수 있어 먼저 셸 API로 묻는다.
fn local_app_data() -> PathBuf {
    let mut path: PWSTR = std::ptr::null_mut();
    // SAFETY: rfid는 정적 GUID이고 토큰 null은 이 사용자다. path는 성공 여부와 관계없이 CoTaskMemFree로 푼다.
    let known = unsafe {
        let hr = SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            KF_FLAG_DEFAULT as u32,
            std::ptr::null_mut(),
            &mut path,
        );
        let dir = (hr >= 0 && !path.is_null()).then(|| PathBuf::from(wide(path)));
        CoTaskMemFree(path as *const _);
        dir
    };
    known
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .filter(|p| p.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
}

/// NUL로 끝나는 UTF-16 문자열.
///
/// # Safety
/// `text`는 NUL로 끝나는 유효한 UTF-16 문자열을 가리켜야 한다.
unsafe fn wide(text: *const u16) -> OsString {
    // SAFETY: 부른 쪽이 NUL로 끝남을 보장한다.
    unsafe {
        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
        OsString::from_wide(std::slice::from_raw_parts(text, len))
    }
}

/// 이 프로세스 토큰의 사용자(SID 문자열)와 세션 번호.
struct CurrentUser {
    sid: String,
    session: u32,
}

impl CurrentUser {
    fn get() -> io::Result<CurrentUser> {
        // SAFETY: GetCurrentProcess는 닫지 않아도 되는 가짜 핸들을 돌려준다.
        let token = open_token(unsafe { GetCurrentProcess() })?;
        let sid = sid_string(token_user(&token)?.sid())?;
        let (mut session, mut len) = (0u32, 0u32);
        // SAFETY: TokenSessionId는 DWORD 하나를 채운다.
        let ok = unsafe {
            GetTokenInformation(
                raw(&token),
                TokenSessionId,
                (&raw mut session).cast(),
                size_of::<u32>() as u32,
                &mut len,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(CurrentUser { sid, session })
    }
}

fn open_token(process: HANDLE) -> io::Result<OwnedHandle> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: process는 열린 프로세스 핸들이고 token은 받을 자리다.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 방금 연 토큰 핸들의 소유권을 넘겨받는다.
    Ok(unsafe { OwnedHandle::from_raw_handle(token as _) })
}

/// 토큰의 TOKEN_USER. SID는 같은 버퍼 안에 있다(포인터 정렬을 위해 u64로 잡는다).
struct TokenUserInfo(Vec<u64>);

impl TokenUserInfo {
    fn sid(&self) -> PSID {
        // SAFETY: 버퍼는 GetTokenInformation(TokenUser)이 채운 TOKEN_USER로 시작한다.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

fn token_user(token: &OwnedHandle) -> io::Result<TokenUserInfo> {
    let mut len = 0u32;
    // 크기를 먼저 묻는다(ERROR_INSUFFICIENT_BUFFER로 실패하며 len을 채운다).
    // SAFETY: 버퍼 없이 크기만 묻는다.
    unsafe { GetTokenInformation(raw(token), TokenUser, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: buf는 len바이트 이상이고 8바이트로 정렬돼 있다.
    let ok = unsafe {
        GetTokenInformation(
            raw(token),
            TokenUser,
            buf.as_mut_ptr().cast(),
            len,
            &mut len,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(TokenUserInfo(buf))
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut text: PWSTR = std::ptr::null_mut();
    // SAFETY: sid는 유효한 SID다. 성공하면 text는 LocalFree로 풀어야 하는 문자열이다.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 방금 받은 NUL로 끝나는 문자열이고 다 읽은 뒤 푼다.
    let sid = unsafe {
        let sid = wide(text).to_string_lossy().into_owned();
        LocalFree(text.cast());
        sid
    };
    Ok(sid)
}

pub fn connect(name: &SocketName) -> io::Result<File> {
    // 신원 확인 수준으로만 연다: 서버가 이 클라이언트를 가장해 이 사용자의 권한으로 일하지 못한다.
    let pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .security_qos_flags(SECURITY_IDENTIFICATION)
        .open(name)?;
    check_server_user(&pipe)?;
    Ok(pipe)
}

/// pipe를 만든 서버 프로세스가 이 사용자인지 본다.
fn check_server_user(pipe: &File) -> io::Result<()> {
    let mut pid = 0u32;
    // SAFETY: pipe는 연결된 클라이언트 쪽 pipe 핸들이다.
    if unsafe { GetNamedPipeServerProcessId(raw(pipe), &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 질의 권한만 요청한다. 실패하면 null이다.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 방금 연 프로세스 핸들의 소유권을 넘겨받는다.
    let process = unsafe { OwnedHandle::from_raw_handle(process as _) };
    let server = token_user(&open_token(raw(&process))?)?;
    // SAFETY: GetCurrentProcess는 가짜 핸들이다.
    let me = token_user(&open_token(unsafe { GetCurrentProcess() })?)?;
    // SAFETY: 두 SID 모두 살아 있는 버퍼 안에 있다.
    if unsafe { EqualSid(server.sid(), me.sid()) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "parallax 인스턴스 pipe를 다른 사용자의 프로세스가 열었다",
        ));
    }
    Ok(())
}

/// pipe 인스턴스가 하나도 없는 경우(primary 없음, 또는 연결 처리 사이의 짧은 틈).
pub fn is_absent(e: &io::Error) -> bool {
    e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32)
}

/// 모든 pipe 인스턴스가 다른 연결을 처리하는 중인 경우.
pub fn is_busy(e: &io::Error) -> bool {
    e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
}

/// 이 사용자만 쓸 수 있는 pipe 보안 설명자: 상속을 막은 DACL에 그 사용자의 모든 권한 하나(`D:P(A;;GA;;;SID)`).
struct PipeSecurity(PSECURITY_DESCRIPTOR);

// SAFETY: 만든 뒤 읽기만 하는 설명자다(pipe 인스턴스를 만들 때 넘긴다).
unsafe impl Send for PipeSecurity {}

impl PipeSecurity {
    fn for_current_user() -> io::Result<PipeSecurity> {
        let user = CurrentUser::get()?;
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})", user.sid)
            .encode_utf16()
            .chain(iter::once(0))
            .collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: sddl은 NUL로 끝난다. 성공하면 descriptor는 LocalFree로 풀어야 한다.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(PipeSecurity(descriptor))
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW가 할당한 설명자다.
        unsafe { LocalFree(self.0) };
    }
}

pub struct Listener {
    name: Vec<u16>,
    security: PipeSecurity,
    first: OwnedHandle,
}

impl Listener {
    pub fn bind(name: &SocketName) -> io::Result<Self> {
        let name: Vec<u16> = OsStr::new(name)
            .encode_wide()
            .chain(iter::once(0))
            .collect();
        let security = PipeSecurity::for_current_user()?;
        // FIRST_PIPE_INSTANCE: 같은 이름의 서버가 이미 있으면 실패한다(락에 더한 이중 안전장치).
        let first = create_instance(&name, &security, true)?;
        Ok(Listener {
            name,
            security,
            first,
        })
    }

    pub fn run(self, on_conn: impl Fn(&mut File) + Send + Sync + 'static) {
        let on_conn = Arc::new(on_conn);
        let mut next = Some(self.first);
        loop {
            let pipe = match next.take() {
                Some(pipe) => pipe,
                None => match create_instance(&self.name, &self.security, false) {
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

fn create_instance(name: &[u16], security: &PipeSecurity, first: bool) -> io::Result<OwnedHandle> {
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let attributes = security.attributes();
    // SAFETY: name은 NUL로 끝나는 UTF-16 문자열이고 attributes는 이 호출 동안 살아 있다.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            &attributes,
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

pub fn spawn_detached(mut command: Command) -> io::Result<Child> {
    // 셸이 넘긴 표준 핸들(Git Bash의 파이프)을 자식이 물려받지 않게 한다. Rust는 상속 가능한 핸들을 모두 물려주므로
    // (`inherit_handles`는 아직 불안정하다) 그대로 두면 `$(plx …)`·`plx … | cat`처럼 파이프가 닫히기를 기다리는
    // 셸이 앱이 끝날 때까지 돌아오지 않는다. 이 프로세스는 곧 끝나므로 제 표준 핸들의 상속만 끈다.
    for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: 표준 핸들을 묻고 상속 표시만 바꾼다. 실패해도(콘솔 핸들 등) 그대로 띄운다.
        unsafe {
            let handle = GetStdHandle(id);
            if !handle.is_null() && !std::ptr::eq(handle, INVALID_HANDLE_VALUE) {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
    // 셸이 속한 job(터미널을 닫으면 함께 종료되는)에서 빠져나온다. job이 허용하지 않으면 그대로 띄운다.
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
    match command.spawn() {
        Ok(child) => Ok(child),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            command.spawn()
        }
        Err(e) => Err(e),
    }
}
