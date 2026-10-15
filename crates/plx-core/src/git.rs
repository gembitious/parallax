//! git CLI 실행.
//!
//! 이력 질의(blame, log)는 git CLI를 스트리밍으로 쓴다. 모든 호출은 이 모듈의 [`command`]로 만든다.
//! - `-c core.quotepath=false`: 한글 경로를 `\355\225\234`처럼 이스케이프하지 않는다.
//! - `GIT_OPTIONAL_LOCKS=0`: 읽기 전용 앱이 index.lock 같은 선택적 락을 잡지 않게 한다.
//! - Windows는 `CREATE_NO_WINDOW`로 콘솔 창이 뜨지 않게 한다.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

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
}
