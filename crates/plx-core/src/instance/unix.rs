//! Unix 도메인 소켓 전송 (macOS, 개발용 Linux).

use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::IO_TIMEOUT;

pub type SocketName = PathBuf;

/// `sockaddr_un.sun_path` 한도(macOS 104바이트, NUL 포함)를 넘지 않게 한다.
const MAX_SOCKET_PATH: usize = 103;

pub fn locate(app_id: &str, stem: &str) -> io::Result<(PathBuf, SocketName)> {
    // SAFETY: getuid는 항상 성공한다.
    let uid = unsafe { libc::getuid() };
    let socket_name = format!("{stem}.sock");
    // 사용자 전용 자리가 너무 깊어 소켓 경로가 한도를 넘으면 /tmp 아래로 간다(어느 프로세스든 같은 순서로 고른다).
    for base in runtime_bases(uid) {
        let dir = base.join(format!("{app_id}.{uid}"));
        let socket = dir.join(&socket_name);
        if socket.as_os_str().len() > MAX_SOCKET_PATH {
            continue;
        }
        ensure_private_dir(&dir, uid)?;
        return Ok((dir.join(format!("{stem}.lock")), socket));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("소켓 경로가 너무 길다: /tmp/{app_id}.{uid}/{socket_name}"),
    ))
}

pub fn socket_in_dir(dir: &Path, name: &str) -> SocketName {
    dir.join(format!("{name}.sock"))
}

/// 인스턴스 디렉터리를 둘 자리(앞에서부터 고른다). 환경 변수는 보지 않는다: 셸마다 TMPDIR·XDG_RUNTIME_DIR가
/// 달라도(`nix develop`, Dock에서 뜬 앱) 같은 자리여야 같은 인스턴스를 찾는다.
/// macOS는 사용자별 임시 디렉터리(`confstr(_CS_DARWIN_USER_TEMP_DIR)`, launchd가 주는 TMPDIR와 같은 곳),
/// Linux는 로그인 세션의 `/run/user/<uid>`(systemd, 이 사용자 전용일 때만). 마지막은 `/tmp`다.
fn runtime_bases(uid: u32) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    #[cfg(target_os = "macos")]
    {
        // macOS는 confstr가 이 사용자의 자리를 준다.
        let _ = uid;
        bases.extend(darwin_user_temp_dir());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let run = PathBuf::from(format!("/run/user/{uid}"));
        if is_private_dir(&run, uid) {
            bases.push(run);
        }
    }
    bases.push(PathBuf::from("/tmp"));
    bases
}

#[cfg(target_os = "macos")]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: buf는 len바이트 쓰기 가능하다. confstr는 NUL을 포함한 필요한 길이를 돌려주고 넘치면 자른다.
    let len = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    if len == 0 || len > buf.len() {
        return None;
    }
    buf.truncate(len - 1);
    let dir = PathBuf::from(std::ffi::OsString::from_vec(buf));
    dir.is_absolute().then_some(dir)
}

#[cfg(not(target_os = "macos"))]
fn is_private_dir(dir: &Path, uid: u32) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|meta| {
        meta.is_dir() && meta.uid() == uid && meta.permissions().mode() & 0o077 == 0
    })
}

/// 다른 사용자가 미리 만들어 둔 디렉터리로 소켓을 가로채지 못하게 소유자와 권한을 확인한다.
fn ensure_private_dir(dir: &Path, uid: u32) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}가 이 사용자 전용 디렉터리가 아니다", dir.display()),
        ));
    }
    Ok(())
}

pub fn connect(socket: &SocketName) -> io::Result<UnixStream> {
    // 응답 시간 제한은 부른 쪽(`exchange`)이 따로 건다. 소켓의 제한은 남은 스레드가 끝나게 하는 것이다.
    let stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(stream)
}

/// 소켓 파일이 없거나(인스턴스 없음) 듣는 쪽이 없는(낡은 소켓) 경우.
/// 그 경로에 소켓이 아닌 파일이 있으면 macOS는 ENOTSOCK, Linux는 ECONNREFUSED를 돌려준다.
/// 어느 쪽이든 락을 잡은 쪽이 지우고 다시 bind한다.
pub fn is_absent(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    ) || e.raw_os_error() == Some(libc::ENOTSOCK)
}

/// 대기열이 가득 찬 경우.
pub fn is_busy(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::WouldBlock
}

pub struct Listener(UnixListener);

impl Listener {
    /// 호출하는 쪽이 primary 락을 쥐고 있어야 한다. 그래서 남은 소켓 파일은 죽은 인스턴스의 것이다.
    pub fn bind(socket: &SocketName) -> io::Result<Self> {
        match std::fs::remove_file(socket) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        UnixListener::bind(socket).map(Listener)
    }

    pub fn run(self, on_conn: impl Fn(&mut UnixStream) + Send + Sync + 'static) {
        let on_conn = Arc::new(on_conn);
        for conn in self.0.incoming() {
            let mut stream = match conn {
                Ok(stream) => stream,
                Err(e) => {
                    // 파일 디스크립터 고갈처럼 계속 실패하는 경우에 CPU를 태우지 않게 잠깐 쉰다.
                    eprintln!("plx: 인스턴스 소켓 accept 실패: {e}");
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };
            let on_conn = on_conn.clone();
            let spawned = thread::Builder::new()
                .name("plx-instance-conn".into())
                .spawn(move || {
                    if stream.set_read_timeout(Some(IO_TIMEOUT)).is_ok()
                        && stream.set_write_timeout(Some(IO_TIMEOUT)).is_ok()
                    {
                        on_conn(&mut stream);
                    }
                });
            if let Err(e) = spawned {
                eprintln!("plx: 인스턴스 연결 스레드 생성 실패: {e}");
            }
        }
    }
}

pub fn cleanup(socket: &SocketName) {
    let _ = std::fs::remove_file(socket);
}

pub fn allow_foreground() {}

/// macOS에서 launchd가 부모면(Finder·Dock·`open`) 앱으로 뜬 것이다. 그 밖에는 부른 쪽이 기다릴 수 있다:
/// 터미널만 보면 파이프로 부르는 도구(에이전트의 셸 도구, 에디터의 `:!`, lazygit)가 앱이 끝날 때까지 멈춘다.
pub fn launched_from_shell() -> bool {
    // SAFETY: getppid는 항상 성공한다.
    !(cfg!(target_os = "macos") && unsafe { libc::getppid() } == 1)
}

pub fn spawn_detached(mut command: Command) -> io::Result<Child> {
    // 새 세션으로 떼어 내 터미널이 닫힐 때 오는 SIGHUP을 받지 않게 한다.
    // SAFETY: fork 뒤 exec 전에는 async-signal-safe 함수만 부를 수 있고, setsid는 그에 해당한다.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()
}
