//! 단일 인스턴스와 `plx` 호출 전달.
//!
//! `plx`를 실행하면 프로세스 맨 앞(Tauri 초기화 전)에서 [`claim`]을 부른다.
//! - 상주 중인 인스턴스(primary)가 있으면 로컬 소켓으로 [`Invocation`]을 넘기고 응답을 받은 뒤 끝난다.
//! - 없으면 락 파일을 잡아 이 프로세스가 primary가 되고, [`PrimaryLock::serve`]로 이후 호출을 받는다.
//!
//! 전송은 Windows named pipe, Unix 도메인 소켓이다. primary 판정은 락 파일(`File::try_lock`)로 한다.
//! 프로세스가 죽으면 OS가 락을 풀어 주므로 낡은 락은 남지 않는다. Unix 소켓 파일은 남을 수 있지만
//! 락을 잡은 쪽만 지우고 다시 bind하므로 경쟁이 없다.
//! 락·소켓 자리는 환경 변수(TMPDIR, XDG_RUNTIME_DIR, LOCALAPPDATA)가 아니라 사용자로 정한다. 셸마다 값이 달라도
//! (`nix develop`의 TMPDIR, Dock에서 뜬 앱) 같은 인스턴스를 찾는다.
//! 프레임은 `u32`(LE) 길이 + JSON이고, 연결 하나에 요청 하나·응답 하나다. 응답은 [`IO_TIMEOUT`]까지만 기다린다.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

/// [`Invocation`] 형식이 바뀌면 올린다. 다른 버전끼리는 요청을 거절한다. 기본값이 있는 필드를 더하는 것은
/// 올리지 않는다(앱을 새로 깔았는데 예전 인스턴스가 떠 있어도 서로 받는다. 모르는 필드는 버린다).
pub const PROTOCOL_VERSION: u32 = 1;
const MAX_FRAME_LEN: u32 = 1 << 20;
/// 연결 하나에서 요청을 보내고 응답을 받기까지의 한도. 상주 인스턴스가 멈췄으면 터미널을 붙잡지 않고 알린다.
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// `plx` 호출 하나. 상주 인스턴스에 넘기는 내용 전부다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invocation {
    pub version: u32,
    /// argv[0]을 뺀 인자.
    pub args: Vec<String>,
    /// 호출한 셸의 작업 디렉터리.
    pub cwd: String,
    /// 호출 시작 시각(Unix epoch ms). 지연 측정의 기준점이다.
    pub t0_ms: f64,
    /// 호출한 프로세스가 `main()`에 들어온 시각(Unix epoch ms). `t0_ms`부터 여기까지가 실행 파일을 띄우는 데 든
    /// 시간이다(동적 라이브러리 싣기 포함). 넣기 전의 호출은 보내지 않는다(0).
    #[serde(default)]
    pub main_ms: f64,
    pub pid: u32,
    /// 호출한 셸의 환경 변수 가운데 상주 인스턴스가 쓸 것(에디터 고르기: `editor::FORWARDED_ENV`).
    /// 상주 앱은 다른 환경(Finder에서 뜬 앱의 짧은 PATH 등)에서 떴을 수 있다.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// 상주 인스턴스의 응답.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn ok() -> Self {
        Reply {
            ok: true,
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Reply {
            ok: false,
            error: Some(message.into()),
        }
    }
}

/// 인스턴스 하나를 가리키는 락 파일 경로와 소켓 이름.
#[derive(Debug, Clone)]
pub struct Endpoint {
    lock_path: PathBuf,
    socket: sys::SocketName,
}

impl Endpoint {
    /// `app_id`는 번들 식별자다. `flavor`에 개발 빌드는 `Some("dev")`를 넘겨
    /// 개발 인스턴스와 릴리스 인스턴스가 서로의 호출을 받지 않게 한다.
    pub fn new(app_id: &str, flavor: Option<&str>) -> io::Result<Self> {
        let stem = match flavor {
            Some(flavor) => format!("instance-{flavor}"),
            None => "instance".to_owned(),
        };
        let (lock_path, socket) = sys::locate(app_id, &stem)?;
        Ok(Endpoint { lock_path, socket })
    }

    /// 락 파일을 `dir`에 두는 엔드포인트. 테스트처럼 경로를 직접 정할 때 쓴다.
    /// Windows pipe 이름은 `name`으로 만들므로 시스템 전체에서 겹치지 않는 이름을 줘야 한다.
    pub fn in_dir(dir: &Path, name: &str) -> Self {
        Endpoint {
            lock_path: dir.join(format!("{name}.lock")),
            socket: sys::socket_in_dir(dir, name),
        }
    }

    /// 락 파일이 있는 디렉터리(이 사용자 전용). 분리 실행 기록처럼 인스턴스에 딸린 파일을 둔다.
    pub fn dir(&self) -> &Path {
        self.lock_path.parent().unwrap_or(Path::new("."))
    }
}

/// [`claim`]의 결과.
pub enum Claim {
    /// 상주 인스턴스가 호출을 받았다.
    Forwarded(Reply),
    /// 상주 인스턴스가 없어 이 프로세스가 primary가 됐다.
    Primary(PrimaryLock),
}

/// 상주 인스턴스에 호출을 넘기거나, 없으면 primary 락을 잡는다.
///
/// 락은 잡혀 있는데 소켓이 아직 없으면 다른 프로세스가 primary로 뜨는 중이므로 `timeout`까지 다시 시도한다.
pub fn claim(endpoint: &Endpoint, invocation: &Invocation, timeout: Duration) -> io::Result<Claim> {
    sys::allow_foreground();
    let deadline = Instant::now() + timeout;
    let mut backoff = Duration::from_millis(1);
    loop {
        match sys::connect(&endpoint.socket) {
            Ok(stream) => return exchange(stream, invocation, IO_TIMEOUT).map(Claim::Forwarded),
            Err(e) if sys::is_absent(&e) => {
                if let Some(lock) = try_lock(endpoint)? {
                    return Ok(Claim::Primary(lock));
                }
            }
            Err(e) if sys::is_busy(&e) => {}
            Err(e) => return Err(e),
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "실행 중인 parallax 인스턴스가 응답하지 않는다",
            ));
        }
        thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_millis(20));
    }
}

fn send(stream: &mut (impl Read + Write), invocation: &Invocation) -> io::Result<Reply> {
    write_frame(stream, &serde_json::to_vec(invocation)?)?;
    Ok(serde_json::from_slice(&read_frame(stream)?)?)
}

/// 요청을 보내고 응답을 `timeout`까지 기다린다. 주고받기는 다른 스레드에서 해서 Windows pipe처럼 읽기 시간
/// 제한이 없는 전송에서도 멈춘 상주 인스턴스가 터미널을 붙잡지 않는다(남은 스레드는 이 프로세스와 함께 끝난다).
fn exchange<S>(mut stream: S, invocation: &Invocation, timeout: Duration) -> io::Result<Reply>
where
    S: Read + Write + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let invocation = invocation.clone();
    thread::Builder::new()
        .name("plx-instance-send".into())
        .spawn(move || {
            let _ = tx.send(send(&mut stream, &invocation));
        })?;
    let unresponsive = || {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "실행 중인 parallax가 {}초 안에 응답하지 않았다. 트레이에서 종료한 뒤 다시 실행한다",
                timeout.as_secs()
            ),
        )
    };
    match rx.recv_timeout(timeout) {
        Ok(Ok(reply)) => Ok(reply),
        // 소켓의 시간 제한(Unix)에 걸렸다: OS 오류 문장 대신 같은 안내를 한다.
        Ok(Err(e))
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Err(unresponsive())
        }
        Ok(Err(e)) => Err(io::Error::new(
            e.kind(),
            format!("실행 중인 parallax와 주고받지 못했다: {e}"),
        )),
        Err(_) => Err(unresponsive()),
    }
}

fn try_lock(endpoint: &Endpoint) -> io::Result<Option<PrimaryLock>> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&endpoint.lock_path)?;
    match file.try_lock() {
        Ok(()) => {
            // 진단용: 어느 프로세스가 primary인지 적어 둔다.
            file.set_len(0)?;
            write!(file, "{}", std::process::id())?;
            Ok(Some(PrimaryLock {
                file,
                endpoint: endpoint.clone(),
            }))
        }
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// primary 자격. 놓으면(drop) 락이 풀린다.
pub struct PrimaryLock {
    file: File,
    endpoint: Endpoint,
}

impl PrimaryLock {
    /// 이후 `plx` 호출을 받기 시작한다. `handler`는 연결마다 별도 스레드에서 불린다.
    pub fn serve<F>(self, handler: F) -> io::Result<Server>
    where
        F: Fn(Invocation) -> Reply + Send + Sync + 'static,
    {
        let listener = sys::Listener::bind(&self.endpoint.socket)?;
        let handler = Arc::new(handler);
        thread::Builder::new()
            .name("plx-instance".into())
            .spawn(move || listener.run(move |stream| handle(stream, &*handler)))?;
        Ok(Server {
            _lock: self.file,
            socket: self.endpoint.socket,
        })
    }
}

/// 호출을 받는 중인 primary. 살아 있는 동안 락을 쥐고 있다.
pub struct Server {
    _lock: File,
    socket: sys::SocketName,
}

impl Server {
    /// 종료 직전에 부른다. 남은 소켓 파일을 지운다(Unix). 지우지 못해도 다음 primary가 정리한다.
    pub fn close(&self) {
        sys::cleanup(&self.socket);
    }
}

fn handle(stream: &mut (impl Read + Write), handler: &(dyn Fn(Invocation) -> Reply + Send + Sync)) {
    let reply = match read_frame(stream) {
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::UnexpectedEof
            ) =>
        {
            Reply::error("요청을 다 받지 못했다(끊겼거나 시간 초과)")
        }
        Err(e) => Reply::error(format!("요청을 읽지 못했다: {e}")),
        Ok(frame) => match parse_invocation(&frame) {
            Ok(invocation) => handler(invocation),
            Err(e) => Reply::error(e.to_string()),
        },
    };
    if let Ok(bytes) = serde_json::to_vec(&reply) {
        let _ = write_frame(stream, &bytes);
    }
}

fn parse_invocation(frame: &[u8]) -> io::Result<Invocation> {
    let unreadable = |e: serde_json::Error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("요청을 읽지 못했다: {e}"),
        )
    };
    // 형식이 바뀌어도 버전부터 읽을 수 있게 먼저 값으로 푼다.
    let value: serde_json::Value = serde_json::from_slice(frame).map_err(unreadable)?;
    let version = value.get("version").and_then(serde_json::Value::as_u64);
    if version != Some(u64::from(PROTOCOL_VERSION)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "실행 중인 parallax와 버전이 다르다 (프로토콜 {PROTOCOL_VERSION}, 호출 {}). 실행 중인 앱을 종료한 뒤 다시 실행한다",
                version.map_or_else(|| "?".to_owned(), |v| v.to_string())
            ),
        ));
    }
    serde_json::from_value(value).map_err(unreadable)
}

fn write_frame(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len())
        .ok()
        .filter(|&len| len <= MAX_FRAME_LEN)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "프레임이 너무 크다"))?;
    // 한 번에 쓴다. named pipe에서 쓰기 호출이 쪼개지지 않게 하려는 것.
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

fn read_frame(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len);
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "프레임이 너무 크다",
        ));
    }
    let mut buf = vec![0; len as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// 이 프로세스를 띄운 쪽이 끝나기를 기다릴 수 있는지 추정한다(셸, 에디터의 `:!`, lazygit, 에이전트의 셸 도구).
/// 그러면 상주 인스턴스가 될 프로세스는 [`spawn_detached`]로 떨어져 나가 부른 쪽을 바로 돌려줘야 한다.
/// 표준 입출력이 터미널이 아니어도(파이프) 기다린다. 기다리는 쪽이 없는 것은 앱으로 띄운 경우다:
/// macOS는 launchd가 부모(Finder·Dock·`open`), Windows는 표준 핸들이 없는 GUI 실행(탐색기·시작 메뉴)이다.
pub fn launched_from_shell() -> bool {
    sys::launched_from_shell()
}

/// 현재 실행 파일을 셸·터미널과 분리된 프로세스로 다시 띄운다. 터미널을 닫아도 살아남는다.
/// 자식의 표준 출력·오류는 `log`에 쓴다(없으면 버린다). 띄우지 못하고 끝난 까닭을 부른 쪽이 보여 줄 수 있다.
pub fn spawn_detached(
    args: &[String],
    cwd: &Path,
    envs: &[(&str, &OsStr)],
    log: Option<&Path>,
) -> io::Result<Child> {
    let mut command = Command::new(std::env::current_exe()?);
    command.args(args).current_dir(cwd).stdin(Stdio::null());
    match log.map(File::create).transpose()? {
        Some(file) => {
            command.stdout(file.try_clone()?).stderr(file);
        }
        None => {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    sys::spawn_detached(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn invocation(args: &[&str]) -> Invocation {
        Invocation {
            version: PROTOCOL_VERSION,
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: "/tmp/한글 경로".to_owned(),
            t0_ms: 1.5,
            main_ms: 2.5,
            pid: 42,
            env: BTreeMap::from([("PATH".to_owned(), "/opt/한글/bin".to_owned())]),
        }
    }

    /// 테스트마다 겹치지 않는 엔드포인트 (Windows pipe 이름은 시스템 전역이다).
    fn endpoint(dir: &Path) -> Endpoint {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Endpoint::in_dir(dir, &format!("plx-test-{}-{n}", std::process::id()))
    }

    fn expect_primary(claim: io::Result<Claim>) -> PrimaryLock {
        match claim.expect("claim") {
            Claim::Primary(lock) => lock,
            Claim::Forwarded(reply) => panic!("primary여야 하는데 전달됐다: {reply:?}"),
        }
    }

    fn expect_forwarded(claim: io::Result<Claim>) -> Reply {
        match claim.expect("claim") {
            Claim::Forwarded(reply) => reply,
            Claim::Primary(_) => panic!("전달돼야 하는데 primary가 됐다"),
        }
    }

    #[test]
    fn frame_roundtrip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, "한글 payload".as_bytes()).unwrap();
        assert_eq!(&buf[..4], &(("한글 payload".len()) as u32).to_le_bytes());
        let read = read_frame(&mut Cursor::new(buf)).unwrap();
        assert_eq!(read, "한글 payload".as_bytes());
    }

    #[test]
    fn rejects_oversized_frame() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_FRAME_LEN + 1).to_le_bytes());
        let err = read_frame(&mut Cursor::new(buf)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_other_protocol_version() {
        let frame = br#"{"version":999,"args":[],"cwd":"/","t0Ms":0,"pid":1,"extra":true}"#;
        let err = parse_invocation(frame).unwrap_err();
        assert!(err.to_string().contains("버전이 다르다"), "{err}");
    }

    #[test]
    fn malformed_requests_get_clear_replies() {
        let reply_to = |bytes: Vec<u8>| -> Reply {
            let mut stream = Cursor::new(bytes);
            let mut out = Vec::new();
            let mut duplex = Duplex(&mut stream, &mut out);
            handle(&mut duplex, &|_| Reply::ok());
            serde_json::from_slice(&read_frame(&mut Cursor::new(out)).unwrap()).unwrap()
        };
        let frame = |payload: &[u8]| {
            let mut buf = Vec::new();
            write_frame(&mut buf, payload).unwrap();
            buf
        };
        let garbage = reply_to(frame(b"{{{{{"));
        assert!(garbage.error.unwrap().starts_with("요청을 읽지 못했다"));
        // 길이만 보내고 끊었다
        let truncated = reply_to(100u32.to_le_bytes().to_vec());
        assert_eq!(
            truncated.error.as_deref(),
            Some("요청을 다 받지 못했다(끊겼거나 시간 초과)")
        );
        let other_version = reply_to(frame(br#"{"version":999}"#));
        assert!(other_version.error.unwrap().contains("버전이 다르다"));
        let fine = reply_to(frame(
            serde_json::to_vec(&invocation(&["a"])).unwrap().as_slice(),
        ));
        assert_eq!(fine, Reply::ok());
    }

    /// 읽기는 앞의 것, 쓰기는 뒤의 것으로
    struct Duplex<'a>(&'a mut Cursor<Vec<u8>>, &'a mut Vec<u8>);

    impl Read for Duplex<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Duplex<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.1.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn invocation_json_uses_camel_case() {
        let json = serde_json::to_string(&invocation(&["a"])).unwrap();
        assert!(json.contains("\"t0Ms\":1.5"), "{json}");
        assert_eq!(
            parse_invocation(json.as_bytes()).unwrap(),
            invocation(&["a"])
        );
    }

    #[test]
    fn accepts_invocations_without_env() {
        // 환경 변수를 넘기기 전의 plx(같은 프로토콜 버전)
        let frame = br#"{"version":1,"args":["a"],"cwd":"/","t0Ms":0,"pid":1}"#;
        let parsed = parse_invocation(frame).unwrap();
        assert!(parsed.env.is_empty());
        let json = serde_json::to_string(&parsed).unwrap();
        assert!(!json.contains("env"), "빈 환경은 보내지 않는다: {json}");
    }

    #[test]
    fn first_claim_becomes_primary_and_second_is_forwarded() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(dir.path());
        let received = Arc::new(Mutex::new(Vec::new()));

        let lock = expect_primary(claim(&ep, &invocation(&[]), Duration::from_secs(5)));
        let sink = received.clone();
        let _server = lock
            .serve(move |inv| {
                sink.lock().unwrap().push(inv);
                Reply::ok()
            })
            .unwrap();

        let second = invocation(&["log", "src/한글.rs"]);
        let reply = expect_forwarded(claim(&ep, &second, Duration::from_secs(5)));
        assert_eq!(reply, Reply::ok());
        assert_eq!(*received.lock().unwrap(), vec![second]);
    }

    #[test]
    fn handler_error_reaches_client() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(dir.path());
        let lock = expect_primary(claim(&ep, &invocation(&[]), Duration::from_secs(5)));
        let _server = lock.serve(|_| Reply::error("거절")).unwrap();
        let reply = expect_forwarded(claim(&ep, &invocation(&["x"]), Duration::from_secs(5)));
        assert_eq!(reply, Reply::error("거절"));
    }

    #[test]
    fn waits_for_primary_that_is_still_starting() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(dir.path());
        let lock = expect_primary(claim(&ep, &invocation(&[]), Duration::from_secs(5)));

        // 락만 잡고 아직 소켓을 열지 않은 primary: 조금 뒤에 serve한다.
        let starter = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            lock.serve(|_| Reply::ok()).unwrap()
        });
        let reply = expect_forwarded(claim(&ep, &invocation(&["late"]), Duration::from_secs(5)));
        assert_eq!(reply, Reply::ok());
        drop(starter.join().unwrap());
    }

    #[test]
    fn times_out_when_primary_never_serves() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(dir.path());
        let _lock = expect_primary(claim(&ep, &invocation(&[]), Duration::from_secs(5)));
        let err = match claim(&ep, &invocation(&[]), Duration::from_millis(100)) {
            Err(e) => e,
            Ok(_) => panic!("시간 초과여야 한다"),
        };
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn lock_is_released_when_primary_drops() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(dir.path());
        let lock = expect_primary(claim(&ep, &invocation(&[]), Duration::from_secs(5)));
        drop(lock);
        expect_primary(claim(&ep, &invocation(&[]), Duration::from_secs(5)));
    }

    #[cfg(unix)]
    #[test]
    fn stale_socket_file_is_replaced() {
        // 강제 종료된 primary가 남긴 소켓(듣는 쪽이 없다), 그리고 소켓이 아닌 파일.
        // connect 오류가 플랫폼마다 다르다(ECONNREFUSED, macOS는 소켓이 아닌 파일에 ENOTSOCK).
        let leave_stale_socket = |path: &Path| {
            drop(std::os::unix::net::UnixListener::bind(path).unwrap());
            assert!(path.exists(), "drop해도 소켓 파일은 남는다");
            // macOS는 소켓을 만든 뒤 CLOEXEC를 따로 건다. 그 사이에 다른 테스트가 git을 띄우면 자식이 듣는 소켓을
            // 물려받아 drop한 뒤에도 연결이 된다(받아 주는 쪽이 없어 응답 대신 EOF). 그 자식이 끝날 때까지 기다린다.
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while std::os::unix::net::UnixStream::connect(path).is_ok() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "남은 소켓이 닫히지 않는다"
                );
                thread::sleep(Duration::from_millis(10));
            }
        };
        let leave_plain_file = |path: &Path| std::fs::write(path, b"").unwrap();
        for leave in [&leave_stale_socket as &dyn Fn(&Path), &leave_plain_file] {
            let dir = tempfile::tempdir().unwrap();
            let ep = endpoint(dir.path());
            leave(&ep.socket);
            let lock = expect_primary(claim(&ep, &invocation(&[]), Duration::from_secs(5)));
            let _server = lock.serve(|_| Reply::ok()).unwrap();
            let reply = expect_forwarded(claim(&ep, &invocation(&["x"]), Duration::from_secs(5)));
            assert_eq!(reply, Reply::ok());
        }
    }

    /// 읽으면 멈추는(또는 정해진 오류를 내는) 연결: 멈춘 상주 인스턴스 흉내
    struct Stuck(Option<io::ErrorKind>);

    impl Read for Stuck {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            match self.0 {
                Some(kind) => Err(io::Error::from(kind)),
                None => {
                    thread::sleep(Duration::from_secs(30));
                    Ok(0)
                }
            }
        }
    }

    impl Write for Stuck {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn unresponsive_primary_is_reported_in_time() {
        let started = Instant::now();
        let err =
            exchange(Stuck(None), &invocation(&["x"]), Duration::from_millis(200)).unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(err.to_string().contains("응답하지 않았다"), "{err}");
        // 소켓의 시간 제한(Unix, EAGAIN)도 같은 안내다(OS 오류 문장이 아니다).
        for kind in [io::ErrorKind::WouldBlock, io::ErrorKind::TimedOut] {
            let err = exchange(
                Stuck(Some(kind)),
                &invocation(&["x"]),
                Duration::from_secs(5),
            )
            .unwrap_err();
            assert!(
                err.to_string().contains("응답하지 않았다"),
                "{kind:?}: {err}"
            );
        }
        let err = exchange(
            Stuck(Some(io::ErrorKind::BrokenPipe)),
            &invocation(&["x"]),
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(err.to_string().contains("주고받지 못했다"), "{err}");
    }

    #[test]
    fn caller_may_wait_under_test_harness() {
        // 테스트는 cargo가 파이프로 띄운다(터미널이 아니어도 부른 쪽이 기다린다).
        assert!(launched_from_shell());
    }

    /// 아래 테스트가 따로 띄우는 프로세스에서만 하는 일: 엔드포인트 디렉터리를 파일에 쓴다.
    #[test]
    fn endpoint_probe() {
        if let Some(out) = std::env::var_os("PLX_TEST_ENDPOINT_OUT") {
            let endpoint = Endpoint::new("com.gembitious.parallax.test", Some("probe")).unwrap();
            std::fs::write(out, endpoint.dir().as_os_str().as_encoded_bytes()).unwrap();
        }
    }

    /// 셸마다 TMPDIR·XDG_RUNTIME_DIR·LOCALAPPDATA가 달라도 같은 인스턴스 자리를 고른다.
    #[test]
    fn endpoint_does_not_follow_environment() {
        let dir = tempfile::tempdir().unwrap();
        let probe = |name: &str| -> Vec<u8> {
            let env_dir = dir.path().join(name);
            std::fs::create_dir_all(&env_dir).unwrap();
            let out = dir.path().join(format!("{name}.out"));
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "instance::tests::endpoint_probe", "--quiet"])
                .env("PLX_TEST_ENDPOINT_OUT", &out)
                .env("TMPDIR", &env_dir)
                .env("TMP", &env_dir)
                .env("TEMP", &env_dir)
                .env("XDG_RUNTIME_DIR", &env_dir)
                .env("LOCALAPPDATA", &env_dir)
                .stdout(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
            std::fs::read(out).unwrap()
        };
        let (a, b) = (probe("a"), probe("b"));
        assert!(!a.is_empty());
        assert_eq!(
            a,
            b,
            "{} ≠ {}",
            String::from_utf8_lossy(&a),
            String::from_utf8_lossy(&b)
        );
        assert!(
            !String::from_utf8_lossy(&a).contains(&*dir.path().to_string_lossy()),
            "환경 변수의 자리를 따라갔다: {}",
            String::from_utf8_lossy(&a)
        );
    }

    #[test]
    fn many_clients_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint(dir.path());
        let count = Arc::new(AtomicUsize::new(0));
        let lock = expect_primary(claim(&ep, &invocation(&[]), Duration::from_secs(5)));
        let counter = count.clone();
        let _server = lock
            .serve(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
                Reply::ok()
            })
            .unwrap();

        let clients: Vec<_> = (0..16)
            .map(|i| {
                let ep = ep.clone();
                thread::spawn(move || {
                    let reply = expect_forwarded(claim(
                        &ep,
                        &invocation(&[&i.to_string()]),
                        Duration::from_secs(10),
                    ));
                    assert_eq!(reply, Reply::ok());
                })
            })
            .collect();
        for client in clients {
            client.join().unwrap();
        }
        assert_eq!(count.load(Ordering::SeqCst), 16);
    }
}
