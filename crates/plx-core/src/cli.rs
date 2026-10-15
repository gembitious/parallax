//! `plx` 인자 파싱. 문법은 CLAUDE.md "CLI 인터페이스"를 따른다.
//!
//! 두 번째 `plx` 프로세스가 먼저 파싱해 잘못된 인자는 터미널에서 바로 알리고,
//! 상주 인스턴스는 넘겨받은 argv·cwd를 같은 함수로 다시 파싱한다.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

pub const USAGE: &str = "\
사용법:
  plx                          현재 저장소 그래프
  plx <file>[:<line>]          blame, 해당 줄 선택
  plx log <file>               파일 히스토리
  plx log -L <a>,<b>:<file>    라인 범위 히스토리
  plx show <rev>               커밋 상세

옵션:
  -h, --help       도움말
  -V, --version    버전
  --               뒤의 인자를 파일 이름으로 본다 (예: plx -- log)";

/// 열 뷰. 경로는 모두 호출한 셸의 cwd 기준으로 푼 절대 경로다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Request {
    /// 현재 저장소 그래프.
    Graph,
    /// 작업트리 파일 blame. `line`이 있으면 그 줄을 선택한다.
    Blame { path: PathBuf, line: Option<u32> },
    /// 파일 히스토리. 지금은 없는(삭제된) 경로도 된다.
    Log { path: PathBuf },
    /// 라인 범위 히스토리 (`git log -L`).
    LineLog { path: PathBuf, start: u32, end: u32 },
    /// 커밋 상세.
    Show { rev: String },
}

impl Request {
    /// 창 제목·화면 표시용 한 줄 요약. 경로는 `cwd` 아래면 상대 경로로 줄인다.
    pub fn label(&self, cwd: &Path) -> String {
        let show = |path: &Path| path.strip_prefix(cwd).unwrap_or(path).display().to_string();
        match self {
            Request::Graph => "그래프".to_owned(),
            Request::Blame { path, line: None } => format!("blame {}", show(path)),
            Request::Blame {
                path,
                line: Some(line),
            } => format!("blame {}:{line}", show(path)),
            Request::Log { path } => format!("log {}", show(path)),
            Request::LineLog { path, start, end } => format!("log -L {start},{end}:{}", show(path)),
            Request::Show { rev } => format!("show {rev}"),
        }
    }
}

/// [`parse`] 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Request(Request),
    Help,
    Version,
}

/// 경로 종류. 파싱이 파일 시스템을 직접 보지 않도록 [`parse`]에 조회 함수로 넘긴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    File,
    Dir,
    Missing,
}

/// 실제 파일 시스템 조회.
pub fn stat(path: &Path) -> PathKind {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => PathKind::Dir,
        Ok(_) => PathKind::File,
        Err(_) => PathKind::Missing,
    }
}

/// 사용법 오류. 터미널에 그대로 보여 줄 문장이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

fn err<T>(message: impl Into<String>) -> Result<T, CliError> {
    Err(CliError(message.into()))
}

/// argv[0]을 뺀 인자를 파싱한다. `cwd`는 호출한 셸의 작업 디렉터리다.
pub fn parse(
    args: &[String],
    cwd: &Path,
    stat: impl Fn(&Path) -> PathKind,
) -> Result<Parsed, CliError> {
    let Some((first, rest)) = args.split_first() else {
        return Ok(Parsed::Request(Request::Graph));
    };
    let request = match first.as_str() {
        "-h" | "--help" => return Ok(Parsed::Help),
        "-V" | "--version" => return Ok(Parsed::Version),
        "--" => match rest {
            [] => Request::Graph,
            [spec] => blame(spec, cwd, &stat)?,
            _ => return err("파일은 하나만 줄 수 있다"),
        },
        "log" => log(rest, cwd)?,
        "show" => show(rest)?,
        option if option.starts_with('-') => return err(format!("알 수 없는 옵션: {option}")),
        spec => match rest {
            [] => blame(spec, cwd, &stat)?,
            _ => return err("파일은 하나만 줄 수 있다"),
        },
    };
    Ok(Parsed::Request(request))
}

/// `<file>[:<line>]`. 이름에 콜론이 든 파일이 실제로 있으면 줄 번호로 나누지 않는다.
fn blame(spec: &str, cwd: &Path, stat: &impl Fn(&Path) -> PathKind) -> Result<Request, CliError> {
    if spec.is_empty() {
        return err("파일 이름이 비어 있다");
    }
    let whole = resolve(cwd, spec);
    match stat(&whole) {
        PathKind::File => {
            return Ok(Request::Blame {
                path: whole,
                line: None,
            });
        }
        PathKind::Dir => return err(format!("디렉터리는 blame할 수 없다: {spec}")),
        PathKind::Missing => {}
    }
    let Some((file, line)) = split_line(spec)? else {
        return err(format!("파일이 없다: {spec}"));
    };
    let path = resolve(cwd, file);
    match stat(&path) {
        PathKind::File => Ok(Request::Blame {
            path,
            line: Some(line),
        }),
        PathKind::Dir => err(format!("디렉터리는 blame할 수 없다: {file}")),
        PathKind::Missing => err(format!("파일이 없다: {file}")),
    }
}

/// `file:123` → `("file", 123)`. 줄 번호 꼴이 아니면 `None`.
fn split_line(spec: &str) -> Result<Option<(&str, u32)>, CliError> {
    let Some((file, line)) = spec.rsplit_once(':') else {
        return Ok(None);
    };
    if file.is_empty() || line.is_empty() || !line.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    // Windows의 `C:5`는 드라이브 C의 상대 경로 `5`다.
    if cfg!(windows) && file.len() == 1 && file.as_bytes()[0].is_ascii_alphabetic() {
        return Ok(None);
    }
    Ok(Some((file, line_number(line)?)))
}

fn line_number(text: &str) -> Result<u32, CliError> {
    match text.parse::<u32>() {
        Ok(0) => err("줄 번호는 1부터 센다"),
        Ok(n) => Ok(n),
        Err(_) => err(format!("줄 번호가 올바르지 않다: {text}")),
    }
}

const LOG_USAGE: &str = "plx log <file> 또는 plx log -L <a>,<b>:<file>";

fn log(rest: &[String], cwd: &Path) -> Result<Request, CliError> {
    let args: Vec<&str> = rest.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["-L", spec] => line_log(spec, cwd),
        [flag] if flag.len() > 2 && flag.starts_with("-L") => line_log(&flag[2..], cwd),
        ["--", path] if !path.is_empty() => Ok(Request::Log {
            path: resolve(cwd, path),
        }),
        [path] if !path.is_empty() && !path.starts_with('-') => Ok(Request::Log {
            path: resolve(cwd, path),
        }),
        [option, ..] if option.starts_with('-') && !matches!(*option, "--" | "-L") => {
            err(format!("알 수 없는 옵션: {option}\n{LOG_USAGE}"))
        }
        _ => err(LOG_USAGE),
    }
}

/// `<a>,<b>:<file>`. 파일 경로의 콜론(Windows 드라이브)은 그대로 둔다.
fn line_log(spec: &str, cwd: &Path) -> Result<Request, CliError> {
    let usage = || CliError(format!("범위는 <a>,<b>:<file> 꼴이어야 한다: {spec}"));
    let (range, file) = spec.split_once(':').ok_or_else(usage)?;
    let (start, end) = range.split_once(',').ok_or_else(usage)?;
    if file.is_empty() {
        return Err(usage());
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(start) || !digits(end) {
        return Err(usage());
    }
    let (start, end) = (line_number(start)?, line_number(end)?);
    if start > end {
        return err(format!("범위의 시작({start})이 끝({end})보다 크다"));
    }
    Ok(Request::LineLog {
        path: resolve(cwd, file),
        start,
        end,
    })
}

fn show(rest: &[String]) -> Result<Request, CliError> {
    match rest {
        // `-`로 시작하는 값은 나중에 git 옵션으로 해석될 수 있어 받지 않는다.
        [rev] if !rev.is_empty() && !rev.starts_with('-') && !rev.contains('\0') => {
            Ok(Request::Show { rev: rev.clone() })
        }
        [rev] if rev.starts_with('-') => err(format!("리비전은 -로 시작할 수 없다: {rev}")),
        _ => err("plx show <rev>"),
    }
}

/// 터미널에 출력하기 전에 부른다.
///
/// Windows 릴리스 빌드는 GUI 서브시스템이라 cmd·PowerShell에서 실행하면 표준 출력 핸들이 없다.
/// 그럴 때만 부모 콘솔에 붙는다. Git Bash는 핸들(파이프)을 넘겨주므로 그대로 쓴다.
pub fn attach_parent_console() {
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::os::windows::io::IntoRawHandle;
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Console::{
            ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
            STD_OUTPUT_HANDLE, SetStdHandle,
        };

        // SAFETY: 표준 핸들 조회·설정과 콘솔 연결만 한다. 실패해도 출력이 안 보일 뿐이다.
        unsafe {
            let valid = |id: STD_HANDLE| {
                let handle = GetStdHandle(id);
                !handle.is_null() && !std::ptr::eq(handle, INVALID_HANDLE_VALUE)
            };
            if valid(STD_OUTPUT_HANDLE) && valid(STD_ERROR_HANDLE) {
                return;
            }
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return;
            }
            // 콘솔에 붙은 뒤에도 표준 핸들이 비어 있으면 콘솔 출력 버퍼를 직접 연다.
            if (!valid(STD_OUTPUT_HANDLE) || !valid(STD_ERROR_HANDLE))
                && let Ok(console) = OpenOptions::new().write(true).open("CONOUT$")
            {
                // 프로세스가 끝날 때까지 쓰므로 핸들을 닫지 않는다.
                let handle = console.into_raw_handle();
                for id in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                    if !valid(id) {
                        SetStdHandle(id, handle);
                    }
                }
            }
        }
    }
}

/// cwd 기준으로 경로를 풀고 `.`·`..`를 글자 그대로 정리한다.
/// cwd는 `current_dir()`(심볼릭 링크가 풀린 실제 경로)이므로 대부분 실제 경로와 같다.
/// 저장소 기준 경로는 M1에서 저장소 루트를 찾을 때 다시 확정한다.
fn resolve(cwd: &Path, arg: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for component in cwd.join(arg).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 가짜 파일 시스템: cwd 기준 상대 경로 → 종류.
    struct Fs {
        cwd: PathBuf,
        entries: HashMap<PathBuf, PathKind>,
    }

    impl Fs {
        fn new() -> Self {
            let cwd = if cfg!(windows) {
                PathBuf::from(r"C:\repo\sub")
            } else {
                PathBuf::from("/repo/sub")
            };
            let mut fs = Fs {
                cwd,
                entries: HashMap::new(),
            };
            for file in [
                "main.rs",
                "src/lib.rs",
                "weird:12",
                "log",
                "-dash.rs",
                "한글/파일.rs",
                "a b.rs",
            ] {
                fs.add(file, PathKind::File);
            }
            fs.add("src", PathKind::Dir);
            fs
        }

        fn add(&mut self, rel: &str, kind: PathKind) {
            let path = resolve(&self.cwd, rel);
            self.entries.insert(path, kind);
        }

        fn parse(&self, args: &[&str]) -> Result<Parsed, CliError> {
            let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            parse(&args, &self.cwd, |p| {
                self.entries.get(p).copied().unwrap_or(PathKind::Missing)
            })
        }

        fn request(&self, args: &[&str]) -> Request {
            match self.parse(args) {
                Ok(Parsed::Request(request)) => request,
                other => panic!("{args:?} → {other:?}"),
            }
        }

        fn error(&self, args: &[&str]) -> String {
            match self.parse(args) {
                Err(CliError(message)) => message,
                other => panic!("{args:?}는 오류여야 한다: {other:?}"),
            }
        }

        fn path(&self, rel: &str) -> PathBuf {
            resolve(&self.cwd, rel)
        }
    }

    #[test]
    fn no_args_is_graph() {
        let fs = Fs::new();
        assert_eq!(fs.request(&[]), Request::Graph);
        assert_eq!(fs.request(&["--"]), Request::Graph);
    }

    #[test]
    fn help_and_version() {
        let fs = Fs::new();
        assert_eq!(fs.parse(&["-h"]), Ok(Parsed::Help));
        assert_eq!(fs.parse(&["--help"]), Ok(Parsed::Help));
        assert_eq!(fs.parse(&["-V"]), Ok(Parsed::Version));
        assert_eq!(fs.parse(&["--version"]), Ok(Parsed::Version));
    }

    #[test]
    fn blame_file_and_line() {
        let fs = Fs::new();
        assert_eq!(
            fs.request(&["main.rs"]),
            Request::Blame {
                path: fs.path("main.rs"),
                line: None
            }
        );
        assert_eq!(
            fs.request(&["src/lib.rs:42"]),
            Request::Blame {
                path: fs.path("src/lib.rs"),
                line: Some(42)
            }
        );
        assert_eq!(
            fs.request(&["./main.rs:1"]),
            Request::Blame {
                path: fs.path("main.rs"),
                line: Some(1)
            }
        );
        assert_eq!(
            fs.request(&["한글/파일.rs:7"]),
            Request::Blame {
                path: fs.path("한글/파일.rs"),
                line: Some(7)
            }
        );
        assert_eq!(
            fs.request(&["a b.rs:3"]),
            Request::Blame {
                path: fs.path("a b.rs"),
                line: Some(3)
            }
        );
    }

    #[test]
    fn blame_resolves_parent_dir() {
        let mut fs = Fs::new();
        fs.add("../top.rs", PathKind::File);
        let expected = if cfg!(windows) {
            PathBuf::from(r"C:\repo\top.rs")
        } else {
            PathBuf::from("/repo/top.rs")
        };
        assert_eq!(
            fs.request(&["../top.rs:5"]),
            Request::Blame {
                path: expected,
                line: Some(5)
            }
        );
    }

    #[test]
    fn existing_file_with_colon_is_not_split() {
        let fs = Fs::new();
        assert_eq!(
            fs.request(&["weird:12"]),
            Request::Blame {
                path: fs.path("weird:12"),
                line: None
            }
        );
    }

    #[test]
    fn double_dash_allows_reserved_names() {
        let fs = Fs::new();
        assert_eq!(
            fs.request(&["--", "log"]),
            Request::Blame {
                path: fs.path("log"),
                line: None
            }
        );
        assert_eq!(
            fs.request(&["--", "-dash.rs:2"]),
            Request::Blame {
                path: fs.path("-dash.rs"),
                line: Some(2)
            }
        );
        assert_eq!(
            fs.request(&["./log"]),
            Request::Blame {
                path: fs.path("log"),
                line: None
            }
        );
    }

    #[test]
    fn blame_errors() {
        let fs = Fs::new();
        assert!(fs.error(&["missing.rs"]).starts_with("파일이 없다"));
        assert!(fs.error(&["missing.rs:3"]).starts_with("파일이 없다"));
        assert!(fs.error(&["src"]).starts_with("디렉터리는"));
        assert!(fs.error(&["src:3"]).starts_with("디렉터리는"));
        assert_eq!(fs.error(&["main.rs:0"]), "줄 번호는 1부터 센다");
        assert!(
            fs.error(&["main.rs:99999999999"])
                .starts_with("줄 번호가 올바르지 않다")
        );
        assert_eq!(
            fs.error(&["main.rs", "src/lib.rs"]),
            "파일은 하나만 줄 수 있다"
        );
        assert_eq!(fs.error(&["-x"]), "알 수 없는 옵션: -x");
        assert_eq!(fs.error(&["--", ""]), "파일 이름이 비어 있다");
    }

    #[test]
    fn log_file() {
        let fs = Fs::new();
        assert_eq!(
            fs.request(&["log", "main.rs"]),
            Request::Log {
                path: fs.path("main.rs")
            }
        );
        // 삭제된 파일의 히스토리도 볼 수 있어야 하므로 존재 여부를 보지 않는다.
        assert_eq!(
            fs.request(&["log", "deleted.rs"]),
            Request::Log {
                path: fs.path("deleted.rs")
            }
        );
        assert_eq!(
            fs.request(&["log", "--", "-dash.rs"]),
            Request::Log {
                path: fs.path("-dash.rs")
            }
        );
    }

    #[test]
    fn line_log() {
        let fs = Fs::new();
        let expected = Request::LineLog {
            path: fs.path("src/lib.rs"),
            start: 10,
            end: 20,
        };
        assert_eq!(fs.request(&["log", "-L", "10,20:src/lib.rs"]), expected);
        assert_eq!(fs.request(&["log", "-L10,20:src/lib.rs"]), expected);
        assert_eq!(
            fs.request(&["log", "-L", "3,3:한글/파일.rs"]),
            Request::LineLog {
                path: fs.path("한글/파일.rs"),
                start: 3,
                end: 3
            }
        );
    }

    #[test]
    fn log_errors() {
        let fs = Fs::new();
        assert_eq!(fs.error(&["log"]), LOG_USAGE);
        assert_eq!(fs.error(&["log", "-L"]), LOG_USAGE);
        assert_eq!(fs.error(&["log", "a.rs", "b.rs"]), LOG_USAGE);
        assert!(fs.error(&["log", "-x"]).starts_with("알 수 없는 옵션: -x"));
        assert!(fs.error(&["log", "-L", "10:f.rs"]).starts_with("범위는"));
        assert!(fs.error(&["log", "-L", "10,20:"]).starts_with("범위는"));
        assert!(fs.error(&["log", "-L", "a,b:f.rs"]).starts_with("범위는"));
        assert!(fs.error(&["log", "-L", "10,+5:f.rs"]).starts_with("범위는"));
        assert_eq!(fs.error(&["log", "-L", "0,5:f.rs"]), "줄 번호는 1부터 센다");
        assert_eq!(
            fs.error(&["log", "-L", "20,10:f.rs"]),
            "범위의 시작(20)이 끝(10)보다 크다"
        );
    }

    #[test]
    fn show_rev() {
        let fs = Fs::new();
        assert_eq!(
            fs.request(&["show", "HEAD~2"]),
            Request::Show {
                rev: "HEAD~2".into()
            }
        );
        assert_eq!(
            fs.request(&["show", "v1.0^{commit}"]),
            Request::Show {
                rev: "v1.0^{commit}".into()
            }
        );
        assert_eq!(fs.error(&["show"]), "plx show <rev>");
        assert_eq!(fs.error(&["show", "a", "b"]), "plx show <rev>");
        assert_eq!(
            fs.error(&["show", "--output=x"]),
            "리비전은 -로 시작할 수 없다: --output=x"
        );
    }

    #[test]
    fn labels_are_relative_to_cwd() {
        let fs = Fs::new();
        assert_eq!(fs.request(&[]).label(&fs.cwd), "그래프");
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            fs.request(&["src/lib.rs:42"]).label(&fs.cwd),
            format!("blame src{sep}lib.rs:42")
        );
        assert_eq!(
            fs.request(&["log", "-L", "1,2:main.rs"]).label(&fs.cwd),
            "log -L 1,2:main.rs"
        );
        assert_eq!(
            fs.request(&["show", "abc123"]).label(&fs.cwd),
            "show abc123"
        );
    }

    #[test]
    fn request_json_shape() {
        let fs = Fs::new();
        let json = serde_json::to_value(fs.request(&["main.rs:3"])).unwrap();
        assert_eq!(json["kind"], "blame");
        assert_eq!(json["line"], 3);
        let json = serde_json::to_value(fs.request(&["log", "-L", "1,2:main.rs"])).unwrap();
        assert_eq!(json["kind"], "lineLog");
        assert_eq!(json["start"], 1);
    }

    #[test]
    fn resolve_never_climbs_above_root() {
        let root = if cfg!(windows) {
            PathBuf::from(r"C:\")
        } else {
            PathBuf::from("/")
        };
        assert_eq!(resolve(&root, "../../x"), root.join("x"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths() {
        let mut fs = Fs::new();
        fs.add(r"C:\other\a.rs", PathKind::File);
        assert_eq!(
            fs.request(&[r"C:\other\a.rs:10"]),
            Request::Blame {
                path: PathBuf::from(r"C:\other\a.rs"),
                line: Some(10)
            }
        );
        assert_eq!(
            fs.request(&["C:/other/a.rs:10"]),
            Request::Blame {
                path: PathBuf::from(r"C:\other\a.rs"),
                line: Some(10)
            }
        );
        assert_eq!(
            fs.request(&["log", "-L", r"1,2:C:\other\a.rs"]),
            Request::LineLog {
                path: PathBuf::from(r"C:\other\a.rs"),
                start: 1,
                end: 2
            }
        );
    }
}
