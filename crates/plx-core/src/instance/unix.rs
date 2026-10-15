//! Unix 도메인 소켓 전송 (macOS, 개발용 Linux).

use std::io::{self, IsTerminal};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub type SocketName = PathBuf;

/// `sockaddr_un.sun_path` 한도(macOS 104바이트, NUL 포함)를 넘지 않게 한다.
const MAX_SOCKET_PATH: usize = 103;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub fn locate(app_id: &str, stem: &str) -> io::Result<(PathBuf, SocketName)> {
    // SAFETY: getuid는 항상 성공한다.
    let uid = unsafe { libc::getuid() };
    let dir = runtime_base().join(format!("{app_id}.{uid}"));
    ensure_private_dir(&dir, uid)?;
    let socket = dir.join(format!("{stem}.sock"));
    if socket.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("소켓 경로가 너무 길다: {}", socket.display()),
        ));
    }
    Ok((dir.join(format!("{stem}.lock")), socket))
}

pub fn socket_in_dir(dir: &Path, name: &str) -> SocketName {
    dir.join(format!("{name}.sock"))
}

/// 사용자 전용 디렉터리: macOS는 사용자별 `$TMPDIR`, Linux는 `$XDG_RUNTIME_DIR`.
fn runtime_base() -> PathBuf {
    if cfg!(target_os = "linux")
        && let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
        && dir.is_absolute()
    {
        return dir;
    }
    std::env::temp_dir()
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
    let stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(stream)
}

/// 소켓 파일이 없거나(인스턴스 없음) 듣는 쪽이 없는(낡은 소켓) 경우.
pub fn is_absent(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
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

pub fn launched_from_shell() -> bool {
    io::stdin().is_terminal() || io::stdout().is_terminal() || io::stderr().is_terminal()
}

pub fn spawn_detached(mut command: Command) -> io::Result<()> {
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
    command.spawn().map(drop)
}
