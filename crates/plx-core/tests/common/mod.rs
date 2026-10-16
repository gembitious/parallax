//! 통합 테스트 공통: fixtures 저장소 준비와 사용자 설정에서 떼어 낸 git 실행.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

pub fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/make-repos.sh")
}

pub fn bash() -> PathBuf {
    if let Some(path) = std::env::var_os("PLX_BASH").filter(|p| !p.is_empty()) {
        return path.into();
    }
    // Windows의 PATH에는 WSL의 bash.exe가 먼저 있을 수 있다. git과 같이 설치된 bash를 쓴다.
    // `git --exec-path` = <Git>/mingw64/libexec/git-core → <Git>/bin/bash.exe
    if cfg!(windows)
        && let Ok(out) = Command::new("git").arg("--exec-path").output()
    {
        let exec_path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if let Some(bash) = exec_path
            .ancestors()
            .map(|dir| dir.join("bin").join("bash.exe"))
            .find(|candidate| candidate.is_file())
        {
            return bash;
        }
    }
    PathBuf::from("bash")
}

/// fixtures 저장소가 있는 디렉터리(`history`, `encoding`). 스크립트 내용이 같으면 다시 만들지 않는다.
pub fn fixtures() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let bytes = std::fs::read(script()).expect("fixtures 스크립트");
        // FNV-1a: 스크립트가 바뀌면 다른 디렉터리를 쓴다.
        let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("fixtures-{hash:016x}"));
        let done = dir.join(".done");
        if !done.exists() {
            let _ = std::fs::remove_dir_all(&dir);
            // Windows 경로의 역슬래시도 bash가 읽지만 슬래시로 넘기는 편이 안전하다.
            let slash = |p: &Path| p.to_string_lossy().replace('\\', "/");
            let status = Command::new(bash())
                .arg(slash(&script()))
                .arg(slash(&dir))
                .status()
                .expect("bash 실행 (Windows는 Git for Windows, 또는 PLX_BASH)");
            assert!(status.success(), "fixtures/make-repos.sh 실패: {status}");
            std::fs::write(&done, b"").expect(".done");
        }
        dir
    })
}

pub fn history() -> PathBuf {
    fixtures().join("history")
}

pub fn encoding() -> PathBuf {
    fixtures().join("encoding")
}

/// 사용자 git 설정(diff.algorithm, status.showUntrackedFiles 등)의 영향을 받지 않게 실행한다.
pub fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git 실행");
    assert!(
        out.status.success(),
        "git {args:?} 실패: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

pub fn git_text(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git(repo, args))
        .expect("UTF-8 출력")
        .trim_end()
        .to_owned()
}

/// 테스트 하나가 쓰는 빈 저장소(브랜치 main, 커밋할 수 있는 사용자 설정). 같은 이름이면 지우고 다시 만든다.
pub fn scratch_repo(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("scratch-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("저장소 디렉터리");
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "T"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "core.autocrlf", "false"]);
    dir
}

/// 작업트리를 모두 커밋하고 sha를 돌려준다.
pub fn commit_all(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git_text(dir, &["rev-parse", "HEAD"])
}

/// 열 줄짜리 파일. `edited` 줄만 다르게 쓴다.
pub fn ten_lines(edited: &[usize]) -> String {
    (1..=10)
        .map(|i| {
            if edited.contains(&i) {
                format!("line {i} edited, the quick brown fox jumps\n")
            } else {
                format!("line {i}: the quick brown fox jumps over\n")
            }
        })
        .collect()
}

/// [`git`]과 같고 작성·커밋 시각을 `t`(Unix 시각)로 정한다. 같은 초에 만든 커밋도 git log 순서가 정해진다.
pub fn git_at(repo: &Path, args: &[&str], t: i64) -> Vec<u8> {
    // `@`이 없으면 git은 작은 수(1000 등)를 Unix 시각으로 읽지 않는다.
    let date = format!("@{t} +0000");
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git 실행");
    assert!(
        out.status.success(),
        "git {args:?} 실패: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// 작업트리를 모두 `t` 시각으로 커밋하고 sha를 돌려준다(바뀐 것이 없어도 커밋한다).
pub fn commit_at(dir: &Path, message: &str, t: i64) -> String {
    git(dir, &["add", "-A"]);
    git_at(dir, &["commit", "-q", "--allow-empty", "-m", message], t);
    git_text(dir, &["rev-parse", "HEAD"])
}
