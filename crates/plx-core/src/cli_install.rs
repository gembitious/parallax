//! 터미널 명령 `plx` 설치(앱의 "터미널 명령 설치". VS Code의 `code` 명령처럼 앱에서 한 번 눌러 만든다).
//!
//! macOS·Linux는 이 실행 파일을 exec하는 셸 shim을 PATH의 폴더에 쓴다(`scripts/install-macos-shim.sh`가 쓰는 것과
//! 같은 모양). macOS는 기본 PATH(`/etc/paths`)에 든 `/usr/local/bin/plx`이고, 쓸 수 없으면(새 Apple Silicon 맥에는
//! 폴더도 없다) macOS의 관리자 암호 창으로 쓴다. Linux는 `~/.local/bin/plx`이고 deb가 둔 `/usr/bin/plx`면 이미 설치된
//! 것이다. Windows는 `plx.exe`가 든 폴더를 사용자 PATH(`HKCU\Environment`)에 더하고 열린 창들에 알린다. 레지스트리
//! API로 값 전체를 읽고 써 길이 한도에 잘리지 않고, 값의 종류(REG_EXPAND_SZ)를 지킨다. 새 터미널이 명령을 찾는
//! 차례(시스템 PATH 다음 사용자 PATH)에서 다른 `plx`(옛 parallax를 풀어 둔 폴더 등)가 먼저 불리면, 그것이 사용자
//! PATH에 있으면 이 앱의 폴더를 그 앞에 넣고 시스템 PATH에 있으면 설치하지 않는다(관리자만 바꾼다).
//!
//! 그 자리에 있는 다른 `plx`(우리 shim이 아닌 파일)는 덮어쓰지 않는다. 우리 shim이 다른 자리의 앱을 부르면(앱을
//! 옮겼다) 그 shim을 고친다. 앱이 곧 사라질 자리(macOS App Translocation, 디스크 이미지, 임시 폴더)에서 돌고 있으면
//! 설치하지 않는다. 저장소는 건드리지 않는다.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// 이 앱의 터미널 명령 상태.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// PATH의 `plx`가 이 앱을 부른다
    pub installed: bool,
    /// 명령의 자리: shim 파일(macOS·Linux) 또는 PATH에 넣는 폴더(Windows)
    pub location: String,
    /// 설치하면 할 일(화면에 보인다)
    pub plan: String,
    /// 설치할 수 없는 까닭(있으면 설치하지 않는다)
    pub blocked: Option<String>,
    /// 덧붙일 말(다른 자리의 앱을 부르는 shim을 고친다, 새로 연 터미널부터 쓸 수 있다 등)
    pub note: Option<String>,
}

/// 지금 실행 파일의 터미널 명령 상태.
pub fn status() -> Status {
    match current_exe() {
        Ok(exe) => status_of(&exe),
        Err(reason) => Status {
            installed: false,
            location: String::new(),
            plan: String::new(),
            blocked: Some(reason),
            note: None,
        },
    }
}

/// 설치하고 새 상태를 돌려준다. 이미 설치됐으면 그대로 둔다. macOS는 관리자 암호 창을 띄울 수 있다.
pub fn install() -> Result<Status, String> {
    let exe = current_exe()?;
    let before = status_of(&exe);
    if before.installed {
        return Ok(before);
    }
    if let Some(reason) = before.blocked {
        return Err(reason);
    }
    platform::install(&exe, &before)?;
    let after = status_of(&exe);
    if !after.installed {
        return Err(format!(
            "설치한 뒤에도 명령이 이 앱을 부르지 않는다({})",
            after.location
        ));
    }
    Ok(Status {
        note: platform::after_install_note(),
        ..after
    })
}

fn current_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("실행 파일 경로를 알 수 없다: {e}"))?;
    Ok(dunce::canonicalize(&exe).unwrap_or(exe))
}

fn status_of(exe: &Path) -> Status {
    let mut status = platform::status(exe);
    if !status.installed && status.blocked.is_none() {
        status.blocked = transient_reason(exe, &std::env::temp_dir()).map(str::to_owned);
    }
    status
}

/// 앱이 곧 사라질 자리에서 돌고 있으면 그 까닭. 그 경로를 부르는 명령은 앱을 끄면(꺼내면) 듣지 않는다.
pub fn transient_reason(exe: &Path, temp: &Path) -> Option<&'static str> {
    let text = exe.to_string_lossy();
    if text.contains("/AppTranslocation/") {
        return Some(
            "macOS가 앱을 임시 자리에서 실행하고 있다. 앱을 응용 프로그램 폴더로 옮겨 다시 연 뒤 설치한다",
        );
    }
    // 내려받은 디스크 이미지는 읽기 전용으로 붙는다. 외장 디스크(쓸 수 있다)에 둔 앱은 설치한다.
    if cfg!(target_os = "macos") && exe.starts_with("/Volumes") && read_only_volume(exe) {
        return Some(
            "디스크 이미지(DMG)에서 실행하고 있다. 앱을 응용 프로그램 폴더로 옮겨 다시 연 뒤 설치한다",
        );
    }
    let temp = dunce::canonicalize(temp).unwrap_or_else(|_| temp.to_path_buf());
    if !temp.as_os_str().is_empty() && exe.starts_with(&temp) {
        return Some("임시 폴더에서 실행하고 있다. 설치하거나 압축을 푼 폴더의 앱을 열어 설치한다");
    }
    None
}

/// `path`가 든 파일 시스템이 읽기 전용이다(알 수 없으면 false).
#[cfg(unix)]
fn read_only_volume(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: path는 NUL로 끝나는 경로이고 stat은 statvfs가 채울 값이다.
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        libc::statvfs(path.as_ptr(), &mut stat) == 0 && stat.f_flag & libc::ST_RDONLY != 0
    }
}

#[cfg(not(unix))]
fn read_only_volume(_: &Path) -> bool {
    false
}

/// shim의 둘째 줄 머리. 앱과 `scripts/install-macos-shim.sh`가 쓴 shim을 알아본다.
const SHIM_MARK: &str = "# parallax plx shim";
/// shim 파일이 이보다 크면 우리 것이 아니다.
const SHIM_MAX_BYTES: usize = 4096;

/// `bin`을 exec하는 shim. 경로를 작은따옴표 안에 그대로 넣으므로 작은따옴표·줄 바꿈이 든 경로는 만들지 않는다.
pub fn shim_script(bin: &str) -> Option<String> {
    if bin.is_empty() || bin.contains(['\'', '\n', '\r', '\0']) {
        return None;
    }
    Some(format!(
        "#!/bin/sh\n{SHIM_MARK} (parallax 앱이 만들었다)\nexec '{bin}' \"$@\"\n"
    ))
}

/// shim이 exec하는 실행 파일. 우리 shim(표시 줄과 `exec '<경로>' "$@"` 한 줄)이 아니면 None.
pub fn shim_target(script: &[u8]) -> Option<String> {
    if script.len() > SHIM_MAX_BYTES {
        return None;
    }
    let text = std::str::from_utf8(script).ok()?;
    let mut lines = text.lines();
    if lines.next()? != "#!/bin/sh" || !lines.next()?.starts_with(SHIM_MARK) {
        return None;
    }
    let bin = lines
        .next()?
        .strip_prefix("exec '")?
        .strip_suffix("' \"$@\"")?;
    // shim_script가 만들지 않는 경로(따옴표·제어 문자)면 우리 것이 아니다.
    if bin.is_empty()
        || bin.contains(['\'', '\r', '\0'])
        || lines.any(|line| !line.trim().is_empty())
    {
        return None;
    }
    Some(bin.to_owned())
}

/// Windows PATH 값(`;` 구분)에 `dir`이 있는지. 항목의 따옴표·끝 `\`를 떼고 환경 변수를 풀어(`expand`)
/// 대소문자 없이 견준다.
pub fn path_has_dir(path: &str, dir: &str, expand: impl Fn(&str) -> String) -> bool {
    let want = normalize_entry(dir, &expand);
    !want.is_empty()
        && path
            .split(';')
            .any(|entry| normalize_entry(entry, &expand) == want)
}

/// `dir`을 뒤에 더한 PATH 값. 이미 있으면 None.
pub fn path_with_dir(path: &str, dir: &str, expand: impl Fn(&str) -> String) -> Option<String> {
    if path_has_dir(path, dir, &expand) {
        return None;
    }
    let path = path.trim_end_matches(';');
    Some(if path.trim().is_empty() {
        dir.to_owned()
    } else {
        format!("{path};{dir}")
    })
}

/// `dir`을 사용자 PATH 값의 `before`째 항목(`;`로 나눈 순번) 앞으로 옮긴 값. 같은 폴더가 다른 자리에 있으면 빼고,
/// `before`가 항목 수보다 크면 뒤에 더한다. 다른 항목은 차례와 글자를 그대로 둔다.
pub fn path_with_dir_before(
    path: &str,
    dir: &str,
    before: usize,
    expand: impl Fn(&str) -> String,
) -> String {
    if path.trim().is_empty() {
        return dir.to_owned();
    }
    let want = normalize_entry(dir, &expand);
    let mut entries = Vec::new();
    let mut placed = false;
    for (index, entry) in path.split(';').enumerate() {
        if index == before {
            entries.push(dir);
            placed = true;
        }
        if want.is_empty() || normalize_entry(entry, &expand) != want {
            entries.push(entry);
        }
    }
    if !placed {
        while entries.last().is_some_and(|entry| entry.trim().is_empty()) {
            entries.pop();
        }
        entries.push(dir);
    }
    entries.join(";")
}

/// Windows PATH의 한 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    /// 사용자 PATH(`HKCU`)의 항목이다(아니면 시스템 PATH)
    pub user: bool,
    /// 그 값을 `;`로 나눈 순번
    pub index: usize,
    /// 따옴표를 떼고 환경 변수를 푼 폴더
    pub dir: String,
}

/// 새로 여는 터미널이 명령을 찾는 차례로 `found`가 참인 첫 폴더. Windows는 시스템 PATH 뒤에 사용자 PATH를 붙여
/// 새 프로세스에 준다. 빈 항목과 절대 경로가 아닌 항목(터미널의 현재 폴더 기준이다)은 건너뛴다.
pub fn first_in_path(
    system: &str,
    user: &str,
    expand: impl Fn(&str) -> String,
    found: impl Fn(&str) -> bool,
) -> Option<PathEntry> {
    [(false, system), (true, user)]
        .into_iter()
        .flat_map(|(user, value)| {
            value
                .split(';')
                .enumerate()
                .map(move |(index, entry)| (user, index, entry))
        })
        .find_map(|(user, index, entry)| {
            let dir = unquote_expand(entry, &expand);
            (windows_absolute(&dir) && found(&dir)).then_some(PathEntry { user, index, dir })
        })
}

/// 두 Windows 폴더 경로가 글자로 같은 폴더다(`/`와 `\`, 끝 `\`, 대소문자 무시).
pub fn same_windows_dir(a: &str, b: &str) -> bool {
    let a = fold_dir(a);
    !a.is_empty() && a == fold_dir(b)
}

fn windows_absolute(dir: &str) -> bool {
    let bytes = dir.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    drive || dir.starts_with(r"\\") || dir.starts_with("//")
}

fn unquote_expand(entry: &str, expand: &impl Fn(&str) -> String) -> String {
    let entry = entry.trim().trim_matches('"');
    if entry.contains('%') {
        expand(entry)
    } else {
        entry.to_owned()
    }
}

fn fold_dir(dir: &str) -> String {
    dir.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn normalize_entry(entry: &str, expand: &impl Fn(&str) -> String) -> String {
    fold_dir(&unquote_expand(entry, expand))
}

#[cfg(unix)]
mod platform {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{Status, shim_script, shim_target};

    /// 이미 있는 `plx`를 찾아볼 자리(앞이 먼저)와 새로 쓸 자리.
    pub(super) struct Places {
        pub candidates: Vec<PathBuf>,
        pub target: PathBuf,
    }

    fn places() -> Places {
        let home_bin = std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join(".local/bin/plx"));
        let usr_local = PathBuf::from("/usr/local/bin/plx");
        if cfg!(target_os = "macos") {
            Places {
                candidates: [Some(usr_local.clone()), home_bin]
                    .into_iter()
                    .flatten()
                    .collect(),
                target: usr_local,
            }
        } else {
            Places {
                candidates: [
                    Some(PathBuf::from("/usr/bin/plx")),
                    Some(usr_local.clone()),
                    home_bin.clone(),
                ]
                .into_iter()
                .flatten()
                .collect(),
                target: home_bin.unwrap_or(usr_local),
            }
        }
    }

    pub(super) fn status(exe: &Path) -> Status {
        status_in(exe, &places())
    }

    enum Found {
        /// 이 실행 파일이거나 그것을 부르는 링크·shim
        Ours,
        /// 우리 shim인데 다른 실행 파일을 부른다
        Other(String),
        /// 우리가 만들지 않은 파일
        Foreign,
    }

    fn inspect(candidate: &Path, exe: &Path) -> Option<Found> {
        let meta = fs::symlink_metadata(candidate).ok()?;
        if same_file(candidate, exe) {
            return Some(Found::Ours);
        }
        if !meta.is_file() || meta.len() > super::SHIM_MAX_BYTES as u64 {
            return Some(Found::Foreign);
        }
        Some(
            match fs::read(candidate).ok().as_deref().and_then(shim_target) {
                Some(bin) if same_file(Path::new(&bin), exe) => Found::Ours,
                Some(bin) => Found::Other(bin),
                None => Found::Foreign,
            },
        )
    }

    fn same_file(a: &Path, b: &Path) -> bool {
        matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
    }

    pub(super) fn status_in(exe: &Path, places: &Places) -> Status {
        let mut other = None;
        let mut foreign = Vec::new();
        for candidate in &places.candidates {
            match inspect(candidate, exe) {
                Some(Found::Ours) => {
                    return Status {
                        installed: true,
                        location: candidate.display().to_string(),
                        plan: String::new(),
                        blocked: None,
                        note: None,
                    };
                }
                Some(Found::Other(bin)) => {
                    other.get_or_insert((candidate.clone(), bin));
                }
                Some(Found::Foreign) => foreign.push(candidate.clone()),
                None => {}
            }
        }
        let (location, note) = match other {
            Some((candidate, bin)) => (
                candidate,
                Some(format!(
                    "지금 명령은 다른 자리의 parallax({bin})를 부른다. 다시 설치하면 이 앱을 부른다"
                )),
            ),
            None => (places.target.clone(), None),
        };
        let blocked = foreign.contains(&location).then(|| {
            format!(
                "{}에 parallax가 만들지 않은 파일이 있어 바꾸지 않는다",
                location.display()
            )
        });
        let blocked = blocked.or_else(|| match exe.to_str().map(shim_script) {
            Some(Some(_)) => None,
            Some(None) => {
                Some("앱 경로에 작은따옴표나 줄 바꿈이 있어 명령을 만들 수 없다".to_owned())
            }
            None => Some("앱 경로가 UTF-8이 아니라 명령을 만들 수 없다".to_owned()),
        });
        let mut plan = format!("만든다: {}", location.display());
        if cfg!(target_os = "macos") {
            plan.push_str(" (그 폴더에 쓸 수 없으면 관리자 암호를 묻는다)");
        }
        Status {
            installed: false,
            location: location.display().to_string(),
            plan,
            blocked,
            note,
        }
    }

    pub(super) fn install(exe: &Path, status: &Status) -> Result<(), String> {
        let script = exe
            .to_str()
            .and_then(shim_script)
            .ok_or("앱 경로로 명령을 만들 수 없다")?;
        let location = Path::new(&status.location);
        match write_shim(location, &script) {
            Ok(()) => Ok(()),
            #[cfg(target_os = "macos")]
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                write_shim_as_admin(location, &script)
            }
            Err(e) => Err(format!("{}에 쓰지 못했다: {e}", location.display())),
        }
    }

    /// 같은 폴더의 임시 파일에 쓴 뒤 이름을 바꾼다(쓰는 도중의 shim을 셸이 읽지 않게).
    pub(super) fn write_shim(location: &Path, script: &str) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = location
            .parent()
            .ok_or_else(|| io::Error::other("폴더가 없는 경로"))?;
        fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".plx.{}.tmp", std::process::id()));
        let result = fs::write(&tmp, script)
            .and_then(|()| fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)))
            .and_then(|()| fs::rename(&tmp, location));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    /// `/usr/local/bin`처럼 이 사용자가 쓸 수 없는 폴더는 macOS의 관리자 암호 창(`osascript … with administrator
    /// privileges`)으로 쓴다. 경로는 osascript 인자로 넘겨 AppleScript의 `quoted form`으로 셸에 넣는다.
    #[cfg(target_os = "macos")]
    fn write_shim_as_admin(location: &Path, script: &str) -> Result<(), String> {
        const RUN: &str = "do shell script \"/bin/mkdir -p \" & quoted form of item 2 of argv & \
            \" && /usr/bin/install -m 0755 \" & quoted form of item 3 of argv & \" \" & quoted form of item 4 of argv \
            with prompt (item 1 of argv) with administrator privileges";
        let dir = location.parent().ok_or("폴더가 없는 경로")?;
        let tmp = admin_temp_file(script).map_err(|e| format!("임시 파일을 쓰지 못했다: {e}"))?;
        let prompt = format!(
            "parallax가 터미널 명령 plx를 {}에 설치하려고 합니다.",
            dir.display()
        );
        let output = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", "on run argv", "-e", RUN, "-e", "end run"])
            .arg(prompt)
            .arg(dir)
            .arg(&tmp)
            .arg(location)
            .output();
        let _ = fs::remove_file(&tmp);
        let output = output.map_err(|e| format!("osascript를 띄우지 못했다: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        // 사용자가 암호 창을 취소하면 AppleScript 오류 -128이다.
        if stderr.contains("(-128)") {
            return Err("관리자 암호 창을 닫아 설치하지 않았다".to_owned());
        }
        Err(format!("관리자 권한으로 쓰지 못했다: {}", stderr.trim()))
    }

    /// 관리자 권한의 `install`이 복사할 shim. 이 사용자만 읽고 쓰는 새 파일로 만든다(이미 있는 파일·링크는 열지 않는다).
    #[cfg(target_os = "macos")]
    fn admin_temp_file(script: &str) -> io::Result<PathBuf> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let mut last = None;
        for attempt in 0..8u32 {
            let tmp = std::env::temp_dir().join(format!(
                "parallax-plx-{}-{nanos:x}-{attempt}.sh",
                std::process::id()
            ));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)
            {
                Ok(mut file) => {
                    return match file.write_all(script.as_bytes()) {
                        Ok(()) => Ok(tmp),
                        Err(e) => {
                            let _ = fs::remove_file(&tmp);
                            Err(e)
                        }
                    };
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("임시 파일 이름이 모두 쓰이고 있다")))
    }

    pub(super) fn after_install_note() -> Option<String> {
        // Linux의 ~/.local/bin은 그 폴더가 있을 때 로그인하면 PATH에 든다(Ubuntu 등의 ~/.profile).
        let dir = places().target.parent().map(Path::to_path_buf)?;
        let in_path = std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|entry| entry == dir));
        (!cfg!(target_os = "macos") && !in_path).then(|| {
            format!(
                "{}가 PATH에 없으면 셸 설정에 더한다: export PATH=\"{}:$PATH\"",
                dir.display(),
                dir.display()
            )
        })
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsStr;
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ,
        REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    use super::{
        PathEntry, Status, first_in_path, path_with_dir, path_with_dir_before, same_windows_dir,
    };

    const USER_KEY: &str = "Environment";
    const SYSTEM_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    /// cmd·PowerShell이 확장자 없이 부르는 `plx`(기본 PATHEXT의 실행 파일 종류).
    const PLX_FILES: [&str; 4] = ["plx.com", "plx.exe", "plx.bat", "plx.cmd"];

    fn dir_of(exe: &Path) -> String {
        exe.parent()
            .map(|dir| dunce::simplified(dir).display().to_string())
            .unwrap_or_default()
    }

    fn has_plx(dir: &str) -> bool {
        PLX_FILES.iter().any(|name| {
            std::fs::symlink_metadata(Path::new(dir).join(name)).is_ok_and(|meta| !meta.is_dir())
        })
    }

    /// `found`가 이 앱의 폴더다(글자가 달라도: 8.3 이름, 정션).
    fn is_dir_of(found: &str, dir: &str) -> bool {
        same_windows_dir(found, dir)
            || matches!(
                (std::fs::canonicalize(found), std::fs::canonicalize(dir)),
                (Ok(a), Ok(b)) if a == b
            )
    }

    fn read_value(root: HKEY, key: &str) -> String {
        read_path(root, key)
            .ok()
            .flatten()
            .map(|(path, _)| path)
            .unwrap_or_default()
    }

    /// 새로 여는 터미널에서 `plx`가 부르는 폴더. `user`는 사용자 PATH 값이다.
    fn first_plx(user: &str) -> Option<PathEntry> {
        let system = read_value(HKEY_LOCAL_MACHINE, SYSTEM_KEY);
        first_in_path(&system, user, expand, has_plx)
    }

    pub(super) fn status(exe: &Path) -> Status {
        let dir = dir_of(exe);
        let mut status = Status {
            installed: false,
            plan: format!("사용자 PATH에 넣는다: {dir}"),
            location: dir.clone(),
            blocked: None,
            note: None,
        };
        if !exe
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("plx.exe"))
        {
            status.blocked = Some(
                "실행 파일 이름이 plx.exe가 아니라 PATH에 넣어도 plx 명령이 되지 않는다".to_owned(),
            );
            return status;
        }
        match first_plx(&read_value(HKEY_CURRENT_USER, USER_KEY)) {
            Some(hit) if is_dir_of(&hit.dir, &dir) => status.installed = true,
            // 먼저 불리는 다른 plx(옛 parallax를 풀어 둔 폴더 등)가 사용자 PATH에 있으면 그 앞에 넣는다.
            Some(hit) if hit.user => {
                status.plan = format!("사용자 PATH의 {} 앞에 넣는다: {dir}", hit.dir);
                status.note = Some(format!("지금 plx 명령은 {}의 plx를 부른다", hit.dir));
            }
            // 시스템 PATH는 사용자 PATH보다 먼저이고 관리자만 바꾼다.
            Some(hit) => {
                status.blocked = Some(format!(
                    "시스템 PATH의 {}에 있는 plx가 먼저 불린다. 그 plx를 지우거나 그 폴더를 시스템 PATH에서 뺀 뒤 설치한다",
                    hit.dir
                ));
            }
            None => {}
        }
        status
    }

    pub(super) fn install(exe: &Path, _status: &Status) -> Result<(), String> {
        let dir = dir_of(exe);
        let (path, kind) =
            read_path(HKEY_CURRENT_USER, USER_KEY)?.unwrap_or((String::new(), REG_EXPAND_SZ));
        let path = match first_plx(&path) {
            Some(hit) if hit.user && !is_dir_of(&hit.dir, &dir) => {
                path_with_dir_before(&path, &dir, hit.index, expand)
            }
            _ => match path_with_dir(&path, &dir, expand) {
                Some(path) => path,
                None => return Ok(()),
            },
        };
        write_user_path(&path, kind)?;
        // 탐색기 같은 열린 창이 새 환경을 읽게 한다(그 창에서 새로 여는 터미널부터 PATH에 든다).
        let mut result = 0usize;
        let what = wide("Environment");
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                what.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                1000,
                &mut result,
            )
        };
        Ok(())
    }

    pub(super) fn after_install_note() -> Option<String> {
        Some("새로 연 터미널부터 쓸 수 있다(이미 열린 터미널은 다시 연다)".to_owned())
    }

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text)
            .encode_wide()
            .chain(iter::once(0))
            .collect()
    }

    fn expand(text: &str) -> String {
        let source = wide(text);
        let mut buf = vec![0u16; 512];
        loop {
            let len = unsafe {
                ExpandEnvironmentStringsW(source.as_ptr(), buf.as_mut_ptr(), buf.len() as u32)
            } as usize;
            if len == 0 {
                return text.to_owned();
            }
            if len <= buf.len() {
                return String::from_utf16_lossy(&buf[..len - 1]);
            }
            buf.resize(len, 0);
        }
    }

    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn open(root: HKEY, key: &str, access: u32) -> Result<Option<Key>, String> {
        let name = wide(key);
        let mut handle: HKEY = std::ptr::null_mut();
        match unsafe { RegOpenKeyExW(root, name.as_ptr(), 0, access, &mut handle) } {
            ERROR_SUCCESS => Ok(Some(Key(handle))),
            ERROR_FILE_NOT_FOUND => Ok(None),
            code => Err(format!("레지스트리 {key}를 열지 못했다(오류 {code})")),
        }
    }

    /// `Path` 값과 그 종류. 값이 없으면 None.
    fn read_path(root: HKEY, key: &str) -> Result<Option<(String, REG_VALUE_TYPE)>, String> {
        let Some(key) = open(root, key, KEY_QUERY_VALUE)? else {
            return Ok(None);
        };
        let value = wide("Path");
        let mut buf = vec![0u16; 1024];
        loop {
            let mut kind: REG_VALUE_TYPE = 0;
            let mut bytes = (buf.len() * 2) as u32;
            let code = unsafe {
                RegQueryValueExW(
                    key.0,
                    value.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    buf.as_mut_ptr().cast(),
                    &mut bytes,
                )
            };
            match code {
                ERROR_SUCCESS => {
                    if kind != REG_SZ && kind != REG_EXPAND_SZ {
                        return Err("PATH 값이 문자열이 아니다".to_owned());
                    }
                    let len = (bytes as usize / 2).min(buf.len());
                    let text = &buf[..len];
                    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
                    return Ok(Some((String::from_utf16_lossy(&text[..end]), kind)));
                }
                ERROR_MORE_DATA => buf.resize(bytes as usize / 2 + 1, 0),
                ERROR_FILE_NOT_FOUND => return Ok(None),
                code => return Err(format!("PATH 값을 읽지 못했다(오류 {code})")),
            }
        }
    }

    fn write_user_path(path: &str, kind: REG_VALUE_TYPE) -> Result<(), String> {
        let key = open(HKEY_CURRENT_USER, USER_KEY, KEY_SET_VALUE)?
            .ok_or("레지스트리 HKCU\\Environment가 없다")?;
        let value = wide("Path");
        let data = wide(path);
        let code = unsafe {
            RegSetValueExW(
                key.0,
                value.as_ptr(),
                0,
                kind,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if code != ERROR_SUCCESS {
            return Err(format!("PATH 값을 쓰지 못했다(오류 {code})"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_round_trip() {
        for bin in [
            "/Applications/parallax.app/Contents/MacOS/plx",
            "/Users/나/앱 모음/parallax.app/Contents/MacOS/plx",
            "/opt/$HOME/`x`/\"q\"/plx",
        ] {
            let script = shim_script(bin).unwrap();
            assert_eq!(
                shim_target(script.as_bytes()).as_deref(),
                Some(bin),
                "{script}"
            );
        }
        for bad in ["", "/a'b/plx", "/a\nb/plx"] {
            assert_eq!(shim_script(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn reads_script_shim() {
        // scripts/install-macos-shim.sh가 쓰는 모양
        let script = "#!/bin/sh\n# parallax plx shim (scripts/install-macos-shim.sh가 만들었다)\n\
                      exec '/Applications/parallax.app/Contents/MacOS/plx' \"$@\"\n";
        assert_eq!(
            shim_target(script.as_bytes()).as_deref(),
            Some("/Applications/parallax.app/Contents/MacOS/plx")
        );
        assert_eq!(
            shim_target(script.replace('\n', "\r\n").as_bytes()).as_deref(),
            Some("/Applications/parallax.app/Contents/MacOS/plx")
        );
    }

    #[test]
    fn foreign_scripts() {
        for script in [
            "",
            "#!/bin/sh\nexec /usr/bin/plx \"$@\"\n",
            "#!/bin/bash\n# parallax plx shim\nexec '/x/plx' \"$@\"\n",
            "#!/bin/sh\n# parallax plx shim\nexec '/x/plx' \"$@\"\nrm -rf ~\n",
            "#!/bin/sh\n# parallax plx shim\nexec '/x'/plx' \"$@\"\n",
            "#!/bin/sh\n# parallax plx shim\nexec '' \"$@\"\n",
        ] {
            assert_eq!(shim_target(script.as_bytes()), None, "{script:?}");
        }
        assert_eq!(shim_target(&vec![b'#'; SHIM_MAX_BYTES + 1]), None);
        assert_eq!(
            shim_target(b"#!/bin/sh\n# parallax plx shim\nexec '\xff' \"$@\"\n"),
            None
        );
    }

    fn expand(text: &str) -> String {
        text.replace("%LOCALAPPDATA%", r"C:\Users\나\AppData\Local")
    }

    #[test]
    fn windows_path_value() {
        let dir = r"C:\Users\나\AppData\Local\parallax";
        assert!(path_has_dir(
            r"C:\bin;%LOCALAPPDATA%\parallax\;D:\x",
            dir,
            expand
        ));
        assert!(path_has_dir(
            r#""c:/users/나/appdata/local/PARALLAX""#,
            dir,
            expand
        ));
        assert!(!path_has_dir(
            r"C:\bin;C:\Users\나\AppData\Local\parallax2",
            dir,
            expand
        ));
        assert!(!path_has_dir("", "", expand));

        assert_eq!(path_with_dir(r"%LOCALAPPDATA%\parallax", dir, expand), None);
        assert_eq!(
            path_with_dir(r"C:\bin;", dir, expand).as_deref(),
            Some(r"C:\bin;C:\Users\나\AppData\Local\parallax")
        );
        assert_eq!(
            path_with_dir(r"%USERPROFILE%\bin", dir, expand).as_deref(),
            Some(r"%USERPROFILE%\bin;C:\Users\나\AppData\Local\parallax")
        );
        assert_eq!(path_with_dir("", dir, expand).as_deref(), Some(dir));
    }

    #[test]
    fn windows_path_order() {
        let ours = r"C:\Users\나\AppData\Local\parallax";
        let old = r"C:\tools\parallax";
        let has = |dirs: &'static [&'static str]| move |dir: &str| dirs.contains(&dir);
        // 시스템 PATH가 먼저, 그 안에서는 앞 항목이 먼저다. 순번은 그 값을 `;`로 나눈 것이다.
        assert_eq!(
            first_in_path(
                r"C:\Windows;C:\Windows\System32",
                r"%LOCALAPPDATA%\parallax;;C:\tools\parallax",
                expand,
                has(&[r"C:\tools\parallax", r"C:\Users\나\AppData\Local\parallax"])
            ),
            Some(PathEntry {
                user: true,
                index: 0,
                dir: ours.to_owned()
            })
        );
        assert_eq!(
            first_in_path(
                r"C:\Windows;C:\tools\parallax\",
                ours,
                expand,
                has(&[r"C:\tools\parallax\", r"C:\Users\나\AppData\Local\parallax"])
            ),
            Some(PathEntry {
                user: false,
                index: 1,
                dir: r"C:\tools\parallax\".to_owned()
            })
        );
        // 따옴표를 떼고, 현재 폴더 기준인 항목은 보지 않는다.
        assert_eq!(
            first_in_path("", r#".;bin;"C:\tools\parallax""#, expand, |_: &str| true),
            Some(PathEntry {
                user: true,
                index: 2,
                dir: old.to_owned()
            })
        );
        assert!(first_in_path(r"\\server\share\bin", "", expand, |_: &str| true).is_some());
        assert_eq!(
            first_in_path("C:\\a;D:\\b", "E:\\c", expand, |_: &str| false),
            None
        );

        assert!(same_windows_dir(r"c:/tools/PARALLAX/", old));
        assert!(!same_windows_dir(r"C:\tools\parallax2", old));
        assert!(!same_windows_dir("", ""));
    }

    #[test]
    fn windows_path_move_before() {
        let dir = r"C:\Users\나\AppData\Local\parallax";
        // 먼저 불리는 옛 plx 앞에 넣고, 뒤에 있던 같은 폴더는 뺀다. 다른 항목의 글자와 빈 항목은 그대로다.
        assert_eq!(
            path_with_dir_before(
                r"%USERPROFILE%\bin;C:\tools\parallax;;%LOCALAPPDATA%\parallax\",
                dir,
                1,
                expand
            ),
            r"%USERPROFILE%\bin;C:\Users\나\AppData\Local\parallax;C:\tools\parallax;"
        );
        assert_eq!(
            path_with_dir_before(r"C:\tools\parallax", dir, 0, expand),
            r"C:\Users\나\AppData\Local\parallax;C:\tools\parallax"
        );
        // 순번이 항목 수를 넘으면 뒤에 더한다.
        assert_eq!(
            path_with_dir_before(r"C:\bin;", dir, 9, expand),
            r"C:\bin;C:\Users\나\AppData\Local\parallax"
        );
        assert_eq!(path_with_dir_before("  ", dir, 0, expand), dir);
    }

    #[test]
    fn transient_places() {
        let temp = std::env::temp_dir();
        let temp = dunce::canonicalize(&temp).unwrap_or(temp);
        assert!(transient_reason(&temp.join("x").join("plx"), &temp).is_some());
        assert!(
            transient_reason(
                Path::new("/private/var/folders/ab/T/AppTranslocation/1234/d/parallax.app/Contents/MacOS/plx"),
                Path::new("/nonexistent-temp")
            )
            .is_some()
        );
        assert!(
            transient_reason(
                Path::new("/Applications/parallax.app/Contents/MacOS/plx"),
                Path::new("/nonexistent-temp")
            )
            .is_none()
        );
        // 디스크 이미지만 막는다: 붙어 있지 않은(읽기 전용인지 모르는) 볼륨은 외장 디스크처럼 본다.
        assert!(
            transient_reason(
                Path::new("/Volumes/없는 디스크/parallax.app/Contents/MacOS/plx"),
                Path::new("/nonexistent-temp")
            )
            .is_none()
        );
    }

    #[cfg(unix)]
    mod shims {
        use super::super::platform::{Places, status_in, write_shim};
        use super::super::*;
        use std::fs;

        struct Setup {
            _dir: tempfile::TempDir,
            exe: PathBuf,
            places: Places,
        }

        fn setup() -> Setup {
            let dir = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(dir.path()).unwrap();
            let exe = root.join("앱/parallax.app/Contents/MacOS/plx");
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, b"binary").unwrap();
            let places = Places {
                candidates: vec![
                    root.join("usr/local/bin/plx"),
                    root.join("home/.local/bin/plx"),
                ],
                target: root.join("usr/local/bin/plx"),
            };
            Setup {
                _dir: dir,
                exe,
                places,
            }
        }

        fn install(s: &Setup) -> Status {
            let status = status_in(&s.exe, &s.places);
            write_shim(
                Path::new(&status.location),
                &shim_script(s.exe.to_str().unwrap()).unwrap(),
            )
            .unwrap();
            status_in(&s.exe, &s.places)
        }

        #[test]
        fn installs_into_target() {
            let s = setup();
            let before = status_in(&s.exe, &s.places);
            assert!(!before.installed);
            assert_eq!(before.location, s.places.target.display().to_string());
            assert_eq!(before.blocked, None);
            let after = install(&s);
            assert!(after.installed, "{after:?}");
            assert_eq!(after.location, s.places.target.display().to_string());
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&s.places.target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
            // 임시 파일이 남지 않는다.
            let names: Vec<_> = fs::read_dir(s.places.target.parent().unwrap())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, vec![std::ffi::OsString::from("plx")]);
        }

        #[test]
        fn fixes_shim_of_moved_app() {
            let s = setup();
            let old = s.places.candidates[1].clone();
            write_shim(
                &old,
                &shim_script("/Old/parallax.app/Contents/MacOS/plx").unwrap(),
            )
            .unwrap();
            let before = status_in(&s.exe, &s.places);
            assert!(!before.installed);
            assert_eq!(before.location, old.display().to_string());
            assert!(before.note.unwrap().contains("/Old/parallax.app"));
            let after = install(&s);
            assert!(after.installed);
            assert_eq!(after.location, old.display().to_string());
        }

        #[test]
        fn link_to_exe_counts() {
            let s = setup();
            fs::create_dir_all(s.places.target.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&s.exe, &s.places.target).unwrap();
            assert!(status_in(&s.exe, &s.places).installed);
        }

        #[test]
        fn leaves_foreign_plx() {
            let s = setup();
            fs::create_dir_all(s.places.target.parent().unwrap()).unwrap();
            fs::write(&s.places.target, "#!/bin/sh\necho other\n").unwrap();
            let status = status_in(&s.exe, &s.places);
            assert!(!status.installed);
            assert!(status.blocked.unwrap().contains("만들지 않은"));
        }
    }
}
