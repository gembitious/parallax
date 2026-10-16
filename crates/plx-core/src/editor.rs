//! 에디터로 열기(`o`): 파일을 VS Code·Zed 같은 GUI 에디터에서 `파일:줄:열`로 연다.
//!
//! 에디터는 이 순서로 고른다. 환경 변수는 마지막 `plx` 호출이 넘긴 셸의 값을 먼저 본다([`Env`]).
//! 1. `PLX_EDITOR`: 아는 이름(`code`, `zed` …)이나 경로, 또는 `{file}`·`{line}`·`{col}`을 넣은 명령 틀
//!    (예: `subl {file}:{line}`, `"C:\Program Files\Foo\foo.exe" --goto {file}:{line}`)
//! 2. `VISUAL`·`EDITOR`가 아는 GUI 에디터일 때(`code --wait`의 `--wait`는 버린다). vim 같은 터미널 에디터는
//!    GUI 앱이 띄울 터미널이 없어 쓰지 않는다.
//! 3. PATH의 `code`, `cursor`, `windsurf`, `zed`, `subl` 순
//! 4. 설치 위치: Windows의 VS Code(사용자·시스템 설치), macOS의 VS Code 앱(`vscode://file/…` 주소로 연다)
//!
//! 띄운 에디터는 기다리지 않는다(거두기만 한다). Windows는 콘솔 창이 뜨지 않게 `CREATE_NO_WINDOW`로 띄운다.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::navigate::{self, Version};
use crate::repo::{Error, Repo};

/// 에디터 종류: 줄·열을 넘기는 방법이 다르다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    /// VS Code와 그 갈래: `--goto 파일:줄:열`
    VsCode,
    /// Zed, Sublime Text: `파일:줄:열`
    Colon,
    /// JetBrains IDE: `--line 줄 --column 열 파일`
    JetBrains,
}

/// 아는 에디터(실행 파일 이름 → 화면 이름, 종류)
const KNOWN: &[(&str, &str, Family)] = &[
    ("code", "VS Code", Family::VsCode),
    ("code-insiders", "VS Code Insiders", Family::VsCode),
    ("cursor", "Cursor", Family::VsCode),
    ("windsurf", "Windsurf", Family::VsCode),
    ("codium", "VSCodium", Family::VsCode),
    ("vscodium", "VSCodium", Family::VsCode),
    ("zed", "Zed", Family::Colon),
    ("zeditor", "Zed", Family::Colon),
    ("zed-preview", "Zed Preview", Family::Colon),
    ("subl", "Sublime Text", Family::Colon),
    ("sublime_text", "Sublime Text", Family::Colon),
    ("idea", "IntelliJ IDEA", Family::JetBrains),
    ("webstorm", "WebStorm", Family::JetBrains),
    ("rustrover", "RustRover", Family::JetBrains),
    ("pycharm", "PyCharm", Family::JetBrains),
    ("goland", "GoLand", Family::JetBrains),
    ("clion", "CLion", Family::JetBrains),
];

/// PATH에서 찾아보는 순서
const SEARCH: &[&str] = &["code", "cursor", "windsurf", "zed", "subl"];

/// `plx`를 부른 셸에서 상주 인스턴스로 넘기는 환경 변수(에디터 고르기)
pub const FORWARDED_ENV: &[&str] = &["PLX_EDITOR", "VISUAL", "EDITOR", "PATH"];

/// 에디터를 고를 때 보는 환경 변수. 상주 앱은 Finder·시작 메뉴나 오래전 셸에서 떠, 앱 자신의 환경(macOS
/// Finder에서 뜬 앱의 PATH는 `/usr/bin:/bin:/usr/sbin:/sbin`)이 지금 쓰는 터미널과 다르다. 그래서 `plx`가 넘긴
/// 셸의 값([`FORWARDED_ENV`])을 먼저 보고, 셸에 없는 변수만 앱 자신의 값을 쓴다. PATH는 셸의 것 뒤에 앱의 것을
/// 잇고, 띄운 에디터도 그 PATH를 받는다.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    shell: BTreeMap<String, String>,
}

impl Env {
    /// 셸에서 넘겨받은 값들. 비었으면 앱 자신의 환경만 본다.
    pub fn new(shell: BTreeMap<String, String>) -> Env {
        Env { shell }
    }

    /// 이 프로세스의 [`FORWARDED_ENV`] 값들(`plx`가 상주 인스턴스에 넘긴다)
    pub fn capture() -> BTreeMap<String, String> {
        FORWARDED_ENV
            .iter()
            .filter_map(|&name| Some((name.to_owned(), std::env::var(name).ok()?)))
            .collect()
    }

    fn var(&self, name: &str) -> Option<String> {
        match self.shell.get(name) {
            Some(value) => Some(value.clone()),
            None => std::env::var(name).ok(),
        }
        .filter(|value| !value.trim().is_empty())
    }

    /// 실행 파일을 찾을 디렉터리: 셸의 PATH, 그 뒤에 앱의 PATH(겹치면 한 번, 빈 항목은 뺀다)
    fn search_path(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let shell = self.shell.get("PATH").map(OsString::from);
        for path in shell.into_iter().chain(std::env::var_os("PATH")) {
            for dir in std::env::split_paths(&path) {
                if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
        dirs
    }

    /// PATH에서 실행 파일을 찾는다(Windows는 PATHEXT의 확장자도).
    pub fn which(&self, name: &str) -> Option<PathBuf> {
        let extensions: Vec<String> = if cfg!(windows) {
            std::env::var("PATHEXT")
                .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
                .split(';')
                .filter(|e| !e.is_empty())
                .map(str::to_owned)
                .collect()
        } else {
            vec![String::new()]
        };
        self.search_path().into_iter().find_map(|dir| {
            extensions.iter().find_map(|ext| {
                let candidate = dir.join(format!("{name}{ext}"));
                is_executable(&candidate).then_some(candidate)
            })
        })
    }
}

/// 열 대상: 파일(줄·열) 또는 디렉터리
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// 절대 경로
    pub path: PathBuf,
    /// 1부터. 없으면 파일 처음(디렉터리면 무시)
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// 고른 에디터
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editor {
    /// 화면에 보일 이름
    pub name: String,
    program: PathBuf,
    how: How,
    /// 띄울 때 줄 PATH(셸에서 넘겨받았을 때)
    path: Option<OsString>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum How {
    Known(Family),
    /// 명령 틀의 인자들(`{file}`·`{line}`·`{col}`을 바꾼다)
    Template(Vec<String>),
    /// 이름만 아는 에디터: 파일만 넘긴다
    FileOnly,
    /// macOS VS Code 앱: `open vscode://file/…`
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    VsCodeUrl,
}

impl Editor {
    /// 환경(`PLX_EDITOR`, `VISUAL`, `EDITOR`, PATH, 설치 위치)에서 에디터를 고른다.
    pub fn find(env: &Env) -> Result<Editor, String> {
        let which = |name: &str| env.which(name);
        let path = env
            .shell
            .contains_key("PATH")
            .then(|| std::env::join_paths(env.search_path()).ok())
            .flatten();
        let found = |editor: Editor| Editor {
            path: path.clone(),
            ..editor
        };
        if let Some(spec) = env.var("PLX_EDITOR") {
            return from_spec(&spec, which)
                .map(found)
                .ok_or_else(|| format!("PLX_EDITOR의 에디터를 찾지 못했다: {spec}"));
        }
        for name in ["VISUAL", "EDITOR"] {
            if let Some(editor) = env.var(name).and_then(|spec| known_gui(&spec, which)) {
                return Ok(found(editor));
            }
        }
        for name in SEARCH {
            if let Some(program) = which(name) {
                return Ok(found(known(name, program).expect("SEARCH는 KNOWN에 있다")));
            }
        }
        installed().map(found).ok_or_else(|| {
            "에디터를 찾지 못했다. VS Code(`code`)나 Zed(`zed`)를 PATH에 두거나 PLX_EDITOR를 정한다 \
             (예: PLX_EDITOR=zed, PLX_EDITOR=\"subl {file}:{line}\")"
                .to_owned()
        })
    }

    /// 에디터에 넘길 인자
    pub fn args(&self, target: &Target) -> Vec<OsString> {
        let file = target.path.as_os_str();
        let line = target.line.unwrap_or(1).max(1);
        let column = target.column.unwrap_or(1).max(1);
        let at = || {
            let mut s = file.to_owned();
            s.push(format!(":{line}:{column}"));
            s
        };
        let is_dir = target.line.is_none() && target.path.is_dir();
        match &self.how {
            // 폴더를 `open`에 그냥 주면 Finder가 연다.
            How::VsCodeUrl if is_dir => {
                vec!["-a".into(), "Visual Studio Code".into(), file.to_owned()]
            }
            How::Known(_) | How::FileOnly if is_dir => vec![file.to_owned()],
            How::Known(Family::VsCode) => vec!["--goto".into(), at()],
            How::Known(Family::Colon) => vec![at()],
            How::Known(Family::JetBrains) => vec![
                "--line".into(),
                line.to_string().into(),
                "--column".into(),
                column.to_string().into(),
                file.to_owned(),
            ],
            How::FileOnly => vec![file.to_owned()],
            How::VsCodeUrl => vec![vscode_url(&target.path, line, column).into()],
            How::Template(words) => words
                .iter()
                .map(|word| {
                    let file = file.to_string_lossy();
                    word.replace("{file}", &file)
                        .replace("{line}", &line.to_string())
                        .replace("{col}", &column.to_string())
                        .into()
                })
                .collect(),
        }
    }

    /// 에디터를 띄운다. 끝나기를 기다리지 않는다.
    pub fn open(&self, target: &Target) -> std::io::Result<()> {
        let mut command = Command::new(&self.program);
        command
            .args(self.args(target))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(dir) = target.path.parent() {
            command.current_dir(dir);
        }
        if let Some(path) = &self.path {
            command.env("PATH", path);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn()?;
        // CLI(code, zed)는 떠 있는 에디터에 넘기고 곧 끝난다. 좀비가 남지 않게 거둔다.
        std::thread::Builder::new()
            .name("plx-editor".into())
            .spawn(move || {
                let _ = child.wait();
            })?;
        Ok(())
    }
}

/// 작업트리에서 열 곳. `path`(저장소 기준)가 없으면 작업트리 폴더다. `rev`의 `line`번째 줄은 작업트리 파일의
/// 줄로 옮긴다(그사이 바뀐 줄은 그 부분이 시작하는 줄). 그 뒤에 이름이 바뀌었으면 작업트리의 새 이름을 연다.
/// 작업트리에 그 파일이 없으면(지웠으면) 오류다.
pub fn locate(
    repo: &Repo,
    path: Option<&str>,
    rev: Option<&str>,
    line: Option<u32>,
) -> crate::repo::Result<Target> {
    let Some(path) = path else {
        return Ok(Target {
            path: repo.workdir().to_path_buf(),
            line: None,
            column: None,
        });
    };
    // 그 리비전 뒤에 이름이 바뀐 파일은 작업트리의 새 이름으로 연다.
    let current = match rev {
        Some(rev) if !repo.workdir().join(path).is_file() => {
            navigate::worktree_path(repo, rev, path)?
        }
        _ => Some(path.to_owned()),
    };
    let Some(current) = current.filter(|p| repo.workdir().join(p).is_file()) else {
        return Err(Error(format!(
            "작업트리에 {path} 파일이 없다(지웠거나 커밋하지 않고 옮겼다)"
        )));
    };
    let file = repo.workdir().join(&current);
    let line = match (rev, line) {
        (Some(rev), Some(line)) => {
            let from = Version {
                rev: Some(rev.to_owned()),
                path: path.to_owned(),
            };
            let to = Version {
                rev: None,
                path: current.clone(),
            };
            // 그 리비전에 이 경로가 없으면(이름이 바뀌기 전) 줄을 그대로 쓴다.
            Some(navigate::map_line(repo, &from, &to, line).unwrap_or(line))
        }
        (_, line) => line,
    };
    Ok(Target {
        path: file,
        line,
        column: None,
    })
}

fn known(name: &str, program: PathBuf) -> Option<Editor> {
    KNOWN
        .iter()
        .find(|(known, _, _)| *known == name)
        .map(|&(_, display, family)| Editor {
            name: display.to_owned(),
            program,
            how: How::Known(family),
            path: None,
        })
}

/// 실행 파일 이름(디렉터리·확장자 빼고 소문자)
fn stem(program: &str) -> String {
    let base = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let base = base.to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat"] {
        if let Some(stripped) = base.strip_suffix(ext) {
            return stripped.to_owned();
        }
    }
    base
}

/// `PLX_EDITOR` 값: 아는 이름·경로, 명령 틀, 또는 모르는 이름(파일만 넘긴다)
fn from_spec(spec: &str, lookup: impl Fn(&str) -> Option<PathBuf>) -> Option<Editor> {
    let words = split_words(spec);
    let (program, rest) = words.split_first()?;
    let path = resolve(program, &lookup)?;
    let name = stem(program);
    if rest.iter().any(|w| w.contains("{file}")) {
        let display = KNOWN
            .iter()
            .find(|(known, _, _)| *known == name)
            .map_or(name.clone(), |(_, display, _)| (*display).to_owned());
        return Some(Editor {
            name: display,
            program: path,
            how: How::Template(rest.to_vec()),
            path: None,
        });
    }
    known(&name, path.clone()).or(Some(Editor {
        name,
        program: path,
        how: How::FileOnly,
        path: None,
    }))
}

/// `VISUAL`·`EDITOR` 값이 아는 GUI 에디터면 그것(옵션은 버린다)
fn known_gui(spec: &str, lookup: impl Fn(&str) -> Option<PathBuf>) -> Option<Editor> {
    let words = split_words(spec);
    let program = words.first()?;
    let path = resolve(program, &lookup)?;
    known(&stem(program), path)
}

/// 이름이면 PATH에서, 경로면 그대로(있을 때)
fn resolve(program: &str, lookup: &impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if program.contains(['/', '\\']) {
        let path = PathBuf::from(program);
        path.is_file().then_some(path)
    } else {
        lookup(program)
    }
}

/// 공백으로 나누되 `"…"`·`'…'` 안은 한 낱말이다.
fn split_words(spec: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in spec.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            None => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// PATH에 없을 때 설치 위치에서 찾는다.
fn installed() -> Option<Editor> {
    #[cfg(windows)]
    {
        let mut dirs = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Programs\Microsoft VS Code\bin\code.cmd"));
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(var) {
                dirs.push(PathBuf::from(base).join(r"Microsoft VS Code\bin\code.cmd"));
            }
        }
        if let Some(program) = dirs.into_iter().find(|p| p.is_file()) {
            return known("code", program);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let apps = [
            Some(PathBuf::from("/Applications")),
            home.map(|h| h.join("Applications")),
        ];
        let has_vscode = apps
            .iter()
            .flatten()
            .any(|dir| dir.join("Visual Studio Code.app").is_dir());
        if has_vscode {
            return Some(Editor {
                name: "VS Code".to_owned(),
                program: PathBuf::from("/usr/bin/open"),
                how: How::VsCodeUrl,
                path: None,
            });
        }
    }
    None
}

/// `vscode://file/<절대 경로>:<줄>:<열>`(경로는 퍼센트 인코딩, 구분자는 `/`)
fn vscode_url(path: &Path, line: u32, column: u32) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::new();
    for &b in path.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                encoded.push(b as char)
            }
            _ => encoded.push_str(&format!("%{b:02X}")),
        }
    }
    let slash = if encoded.starts_with('/') { "" } else { "/" };
    format!("vscode://file{slash}{encoded}:{line}:{column}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(format!("/bin/{name}")))
    }

    fn none(_: &str) -> Option<PathBuf> {
        None
    }

    fn target(line: Option<u32>) -> Target {
        Target {
            path: PathBuf::from("/저장소/소스/파일 이름.rs"),
            line,
            column: None,
        }
    }

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter().map(|a| a.into_string().unwrap()).collect()
    }

    #[test]
    fn known_editors_get_line_numbers() {
        let code = from_spec("code", found).unwrap();
        assert_eq!(code.name, "VS Code");
        assert_eq!(
            strings(code.args(&target(Some(12)))),
            ["--goto", "/저장소/소스/파일 이름.rs:12:1"]
        );
        let zed = from_spec("zed", found).unwrap();
        assert_eq!(
            strings(zed.args(&target(Some(3)))),
            ["/저장소/소스/파일 이름.rs:3:1"]
        );
        let idea = from_spec("idea", found).unwrap();
        assert_eq!(
            strings(idea.args(&target(Some(7)))),
            ["--line", "7", "--column", "1", "/저장소/소스/파일 이름.rs"]
        );
        // 줄이 없으면 1번째 줄
        assert_eq!(
            strings(code.args(&target(None))),
            ["--goto", "/저장소/소스/파일 이름.rs:1:1"]
        );
    }

    #[test]
    fn templates_and_unknown_editors() {
        let t = from_spec("subl {file}:{line}:{col}", found).unwrap();
        assert_eq!(t.name, "Sublime Text");
        assert_eq!(
            strings(t.args(&target(Some(5)))),
            ["/저장소/소스/파일 이름.rs:5:1"]
        );
        let custom = from_spec("myedit --at {line} \"{file}\"", found).unwrap();
        assert_eq!(custom.name, "myedit");
        assert_eq!(
            strings(custom.args(&target(Some(9)))),
            ["--at", "9", "/저장소/소스/파일 이름.rs"]
        );
        // 모르는 에디터: 파일만 넘긴다
        let gedit = from_spec("gedit", found).unwrap();
        assert_eq!(
            strings(gedit.args(&target(Some(9)))),
            ["/저장소/소스/파일 이름.rs"]
        );
        // 못 찾으면 None
        assert!(from_spec("code", none).is_none());
        assert!(from_spec("   ", found).is_none());
    }

    #[test]
    fn visual_and_editor_only_for_gui_editors() {
        assert_eq!(known_gui("code --wait", found).unwrap().name, "VS Code");
        assert_eq!(known_gui("zed -w", found).unwrap().name, "Zed");
        assert!(known_gui("vim", found).is_none());
        assert!(known_gui("nvim -u NONE", found).is_none());
    }

    #[test]
    fn executable_names() {
        assert_eq!(stem("code"), "code");
        assert_eq!(
            stem(r"C:\Users\a\AppData\Local\Programs\Microsoft VS Code\bin\code.cmd"),
            "code"
        );
        assert_eq!(stem("/usr/local/bin/Zed"), "zed");
        assert_eq!(stem("Code.EXE"), "code");
    }

    #[test]
    fn splits_quoted_words() {
        assert_eq!(
            split_words(r#""C:\Program Files\Foo\foo.exe" --goto {file}:{line}"#),
            [r"C:\Program Files\Foo\foo.exe", "--goto", "{file}:{line}"]
        );
        assert_eq!(split_words("  a  'b c'  "), ["a", "b c"]);
        assert_eq!(split_words("a ''"), ["a", ""]);
    }

    #[test]
    fn vscode_urls_encode_paths() {
        assert_eq!(
            vscode_url(Path::new("/저장소/a b.rs"), 3, 1),
            "vscode://file/%EC%A0%80%EC%9E%A5%EC%86%8C/a%20b.rs:3:1"
        );
        assert_eq!(
            vscode_url(Path::new(r"C:\repo\a.rs"), 1, 1),
            "vscode://file/C:/repo/a.rs:1:1"
        );
    }

    #[test]
    fn directories_open_without_line() {
        let dir = tempfile::tempdir().unwrap();
        let code = from_spec("code", found).unwrap();
        let folder = Target {
            path: dir.path().to_path_buf(),
            line: None,
            column: None,
        };
        assert_eq!(
            strings(code.args(&folder)),
            [dir.path().to_string_lossy().into_owned()]
        );
    }

    /// `dir`에 실행 파일 `name`을 만든다(Windows는 `.exe`).
    fn fake_editor(dir: &Path, name: &str) -> PathBuf {
        let exe = dir.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        });
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        exe
    }

    #[test]
    fn shell_values_come_first() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_editor(dir.path(), "plx-test-editor");
        let shell = |pairs: &[(&str, &str)]| {
            Env::new(
                pairs
                    .iter()
                    .map(|&(k, v)| (k.to_owned(), v.to_owned()))
                    .collect(),
            )
        };
        let path = dir.path().to_str().unwrap();
        // 앱 자신의 PATH에는 없고 셸의 PATH에만 있다
        assert_eq!(Env::default().which("plx-test-editor"), None);
        let env = shell(&[("PATH", path), ("PLX_EDITOR", "plx-test-editor {file}")]);
        assert_eq!(env.which("plx-test-editor"), Some(exe.clone()));
        let editor = Editor::find(&env).unwrap();
        assert_eq!(editor.program, exe);
        // 띄울 때 PATH는 셸의 것이 앞이다
        let given = editor.path.expect("셸의 PATH를 넘긴다");
        assert_eq!(
            std::env::split_paths(&given).next().as_deref(),
            Some(dir.path())
        );
        // 셸이 비운 변수는 앱의 값으로 돌아가지 않는다
        assert_eq!(shell(&[("PLX_EDITOR", " ")]).var("PLX_EDITOR"), None);
        // 셸 값이 없으면 PATH를 따로 주지 않는다
        let env = Env::default();
        assert_eq!(env.var("PLX_TEST_NO_SUCH_VARIABLE"), None);
    }

    #[test]
    fn search_path_skips_empty_and_repeated_dirs() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let a = std::env::temp_dir().join("plx-a");
        let joined = format!("{a}{sep}{sep}{a}", a = a.display(), sep = sep);
        let env = Env::new(BTreeMap::from([("PATH".to_owned(), joined)]));
        let dirs = env.search_path();
        assert_eq!(dirs[0], a);
        assert_eq!(dirs.iter().filter(|d| **d == a).count(), 1);
        assert!(dirs.iter().all(|d| !d.as_os_str().is_empty()));
    }

    #[cfg(unix)]
    #[test]
    fn only_executable_files_count() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("myeditor");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.path().join("plain"), "").unwrap();
        assert!(is_executable(&exe));
        assert!(!is_executable(&dir.path().join("plain")));
        assert!(!is_executable(dir.path()));
    }
}
