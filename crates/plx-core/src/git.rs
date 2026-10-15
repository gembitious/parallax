//! git CLI 실행.
//!
//! 이력 질의(blame, log)는 git CLI를 스트리밍으로 쓴다. 모든 호출은 이 모듈의 [`command`]로 만든다.
//! - `-c core.quotepath=false`: 한글 경로를 `\355\225\234`처럼 이스케이프하지 않는다.
//! - `GIT_OPTIONAL_LOCKS=0`: 읽기 전용 앱이 index.lock 같은 선택적 락을 잡지 않게 한다.
//! - Windows는 `CREATE_NO_WINDOW`로 콘솔 창이 뜨지 않게 한다.

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

/// `repo_dir`에서 실행할 git 명령. 인자는 호출하는 쪽에서 붙인다.
pub fn command(repo_dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(repo_dir)
        .args(["-c", "core.quotepath=false"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// 짧은 명령을 실행하고 표준 출력을 바이트로 받는다. 실패하면 표준 오류를 담은 오류를 돌려준다.
pub fn output<I, S>(repo_dir: &Path, args: I) -> io::Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let out = command(repo_dir)
        .args(args)
        .stderr(Stdio::piped())
        .output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(io::Error::other(format!(
            "git 실패({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// 출력을 스트리밍으로 읽는 git 프로세스(blame, log). 다른 스레드에서 [`Canceller`]로 멈출 수 있다.
/// 끝까지 읽지 않고 버리면 git을 끝내고 거둔다(상주 프로세스에 좀비가 쌓이지 않게).
pub struct Streaming {
    /// 오류 메시지 앞머리(예: `git blame`)
    label: &'static str,
    child: Arc<Mutex<Child>>,
    cancelled: Arc<AtomicBool>,
    stdout: ChildStdout,
    stderr: Option<JoinHandle<String>>,
    started: Instant,
    spawn_ms: f64,
}

impl Drop for Streaming {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

#[derive(Clone)]
pub struct Canceller {
    child: Arc<Mutex<Child>>,
    cancelled: Arc<AtomicBool>,
}

impl Canceller {
    /// git을 끝낸다. 읽던 쪽은 [`StreamError::Cancelled`]로 끝난다.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _ = self.child.lock().unwrap_or_else(|e| e.into_inner()).kill();
    }
}

/// 스트리밍이 성공하지 못한 이유.
#[derive(Debug)]
pub enum StreamError {
    Cancelled,
    Io(io::Error),
    /// git이 실패했다(경로가 이력에 없음 등). 표준 오류의 첫 줄을 담는다.
    Git(String),
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamError::Cancelled => f.write_str("취소했다"),
            StreamError::Io(e) => write!(f, "git 실행 실패: {e}"),
            StreamError::Git(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for StreamError {}

impl Streaming {
    /// `command`([`command`]로 만들고 인자를 붙인 것)를 띄운다. 표준 출력·오류는 여기서 파이프로 바꾼다.
    /// `label`은 실패 메시지 앞머리다.
    pub fn spawn(mut command: Command, label: &'static str) -> io::Result<Streaming> {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let started = Instant::now();
        let mut child = command.spawn()?;
        let spawn_ms = started.elapsed().as_secs_f64() * 1000.0;
        let stdout = child.stdout.take().expect("stdout");
        let mut stderr = child.stderr.take().expect("stderr");
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        Ok(Streaming {
            label,
            child: Arc::new(Mutex::new(child)),
            cancelled: Arc::new(AtomicBool::new(false)),
            stdout,
            stderr: Some(stderr),
            started,
            spawn_ms,
        })
    }

    pub fn canceller(&self) -> Canceller {
        Canceller {
            child: self.child.clone(),
            cancelled: self.cancelled.clone(),
        }
    }

    /// 띄우기 직전 시각(측정 기준점)
    pub fn started(&self) -> Instant {
        self.started
    }

    /// 프로세스를 만드는 데 걸린 시간(ms)
    pub fn spawn_ms(&self) -> f64 {
        self.spawn_ms
    }

    /// 표준 출력을 읽는다. 0이면 git이 출력을 닫았다. 실패하면 git을 끝낸다.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        loop {
            match self.stdout.read(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let cancelled = self.abort();
                    return Err(if cancelled {
                        StreamError::Cancelled
                    } else {
                        StreamError::Io(e)
                    });
                }
            }
        }
    }

    /// 출력을 다 읽은 뒤 git이 성공했는지 본다.
    pub fn finish(mut self) -> Result<(), StreamError> {
        let status = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .wait()
            .map_err(StreamError::Io)?;
        let stderr = self
            .stderr
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        if self.cancelled.load(Ordering::SeqCst) {
            return Err(StreamError::Cancelled);
        }
        if !status.success() {
            return Err(StreamError::Git(failure(self.label, &stderr, status)));
        }
        Ok(())
    }

    /// 읽기를 그만두고 git을 끝내고 거둔다. 그 전에 취소됐으면 `true`
    /// (읽던 쪽은 자기 오류 대신 취소로 끝낸다: 취소로 끊긴 출력은 해석 오류처럼 보이기 쉽다).
    pub fn abort(&self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
        let _ = child.wait();
        self.cancelled.load(Ordering::SeqCst)
    }

    /// `dropping_reaps_the_child`(Unix)에서만 쓴다.
    #[cfg(all(test, unix))]
    fn pid(&self) -> u32 {
        self.child.lock().unwrap().id()
    }
}

/// `git blame 실패: no such path …`처럼 표준 오류의 첫 줄을 붙인 메시지.
fn failure(label: &str, stderr: &str, status: ExitStatus) -> String {
    let message = stderr
        .lines()
        .map(|l| l.trim_start_matches("fatal: ").trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if message.is_empty() {
        format!("{label} 실패({status})")
    } else {
        format!("{label} 실패: {message}")
    }
}

/// git이 경로를 C 문자열처럼 따옴표로 감싸 내보낸 경우 원래 바이트로 되돌린다.
/// `core.quotepath=false`여도 큰따옴표·역슬래시·제어 문자가 든 경로는 이렇게 나온다.
pub fn unquote_path(raw: &[u8]) -> Vec<u8> {
    let Some(inner) = raw
        .strip_prefix(b"\"")
        .and_then(|rest| rest.strip_suffix(b"\""))
    else {
        return raw.to_vec();
    };
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        let b = inner[i];
        if b != b'\\' || i + 1 == inner.len() {
            out.push(b);
            i += 1;
            continue;
        }
        let next = inner[i + 1];
        i += 2;
        match next {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'a' => out.push(0x07),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'v' => out.push(0x0b),
            b'0'..=b'7' => {
                // \ooo: 8진수 세 자리(UTF-8 바이트 하나)
                let mut value = u32::from(next - b'0');
                let mut digits = 1;
                while digits < 3 && i < inner.len() && (b'0'..=b'7').contains(&inner[i]) {
                    value = value * 8 + u32::from(inner[i] - b'0');
                    i += 1;
                    digits += 1;
                }
                out.push(value as u8);
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquote_plain_and_escaped() {
        assert_eq!(unquote_path(b"src/main.rs"), b"src/main.rs");
        assert_eq!(unquote_path(b"\"a\\\"b\\\\c\""), b"a\"b\\c");
        assert_eq!(unquote_path(b"\"tab\\there\""), b"tab\there");
        // quotepath=true일 때의 한글 "한" = \355\225\234
        assert_eq!(
            unquote_path(b"\"\\355\\225\\234.txt\""),
            "한.txt".as_bytes()
        );
    }

    #[test]
    fn git_is_available() {
        let out = output(Path::new("."), ["--version"]).expect("git --version");
        assert!(out.starts_with(b"git version"));
    }

    #[test]
    fn streaming_reads_to_the_end_or_reports_git_errors() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut cmd = command(dir);
        cmd.arg("--version");
        let mut git = Streaming::spawn(cmd, "git version").unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 7];
        loop {
            match git.read(&mut buf).unwrap() {
                0 => break,
                n => out.extend_from_slice(&buf[..n]),
            }
        }
        git.finish().unwrap();
        assert!(out.starts_with(b"git version"));

        let mut cmd = command(dir);
        cmd.args(["rev-parse", "--verify", "no-such-ref-for-plx"]);
        let mut git = Streaming::spawn(cmd, "git rev-parse").unwrap();
        while git.read(&mut buf).unwrap() > 0 {}
        let error = git.finish().unwrap_err().to_string();
        assert!(error.starts_with("git rev-parse 실패: "), "{error}");
    }

    /// 읽지 않고 버린 git 프로세스를 거둔다. 거두지 않으면 waitpid가 아직 기다릴 자식을 찾는다.
    #[cfg(unix)]
    #[test]
    fn dropping_reaps_the_child() {
        // 이 crate 디렉터리(저장소가 아니어도 된다: git이 실패해도 프로세스는 뜬다).
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        for cancel_first in [false, true] {
            let mut cmd = command(dir);
            cmd.args(["blame", "--incremental", "--", "Cargo.toml"]);
            let git = Streaming::spawn(cmd, "git blame").unwrap();
            let pid = git.pid() as libc::pid_t;
            if cancel_first {
                git.canceller().cancel();
            }
            drop(git);
            // SAFETY: 이 프로세스의 자식 pid에 대한 waitpid. WNOHANG이라 막히지 않는다.
            let r = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
            let errno = io::Error::last_os_error().raw_os_error();
            assert_eq!(
                (r, errno),
                (-1, Some(libc::ECHILD)),
                "이미 거둔 자식이어야 한다"
            );
        }
    }
}
