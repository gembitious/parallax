//! 새 버전 알림(M8-2)의 Rust 쪽: 릴리스 페이지를 기본 브라우저로 연다. 최신 버전은 화면이 GitHub API로 묻는다
//! (src/update/check.ts). 여는 주소는 이 저장소의 릴리스 페이지뿐이다: 화면이 준 글을 그대로 OS에 넘기지 않는다.
//!
//! 브라우저는 띄우기만 하고 기다리지 않는다(거두기만 한다). Windows는 콘솔 창이 뜨지 않게 `CREATE_NO_WINDOW`로 띄운다.

use std::process::{Command, Stdio};

/// 릴리스를 올리는 저장소
pub const REPOSITORY: &str = "gembitious/parallax";

/// `v<주>.<부>.<수>` 태그의 릴리스 페이지. 다른 모양의 태그는 None이다.
pub fn release_url(tag: &str) -> Option<String> {
    let version = tag.strip_prefix('v')?;
    let parts: Vec<&str> = version.split('.').collect();
    let numeric = |p: &&str| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit());
    if parts.len() != 3 || !parts.iter().all(numeric) {
        return None;
    }
    Some(format!(
        "https://github.com/{REPOSITORY}/releases/tag/{tag}"
    ))
}

/// 이 저장소의 릴리스 페이지(`release_url`)를 기본 브라우저로 연다.
pub fn open(tag: &str) -> std::io::Result<()> {
    let url = release_url(tag).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("릴리스 태그가 아니다: {tag}"),
        )
    })?;
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(&url);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").arg(&url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(&url);
        c
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    std::thread::Builder::new()
        .name("plx-browser".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_url_only_takes_version_tags() {
        assert_eq!(
            release_url("v0.1.0").as_deref(),
            Some("https://github.com/gembitious/parallax/releases/tag/v0.1.0")
        );
        assert!(release_url("v10.20.300").is_some());
        for bad in [
            "0.1.0",
            "v0.1",
            "v0.1.0.1",
            "v0.1.0-rc1",
            "v0..1",
            "v0.1.0 && x",
            "v0.1.0/../../x",
            "va.b.c",
            "v",
            "",
        ] {
            assert_eq!(release_url(bad), None, "{bad}");
        }
    }
}
