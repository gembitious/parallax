//! 워크트리 목록(M4): 주 작업트리와 링크된 워크트리(`git worktree add`).
//!
//! 에이전트가 워크트리마다 브랜치를 두고 일하면 화면을 워크트리 사이로 옮겨 다닌다. 그래프는 다른 워크트리가
//! 체크아웃한 브랜치·HEAD에 그 워크트리 이름을 붙인다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gix::ObjectId;
use gix::bstr::ByteSlice;
use serde::Serialize;

use crate::repo::{Error, Repo, Result, decode_text};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    /// 작업트리 경로(사라졌을 수 있다)
    pub path: String,
    /// 화면에 보이는 이름: 작업트리 폴더 이름(폴더를 모르면 id)
    pub name: String,
    /// 링크된 워크트리의 `.git/worktrees/<id>` id(git이 공백 등을 바꾼 이름). 주 작업트리는 없다.
    pub id: Option<String>,
    /// 주 작업트리
    pub main: bool,
    /// 지금 보고 있는 작업트리
    pub current: bool,
    /// HEAD가 가리키는 브랜치(detached면 없다)
    pub branch: Option<String>,
    pub head: Option<String>,
    pub summary: Option<String>,
    /// HEAD 커밋의 커미터 시각(초)
    pub time: Option<i64>,
    /// `git worktree lock`
    pub locked: bool,
    /// 작업트리 폴더가 사라졌다(`git worktree prune` 대상)
    pub missing: bool,
}

fn err(context: &str, e: impl std::fmt::Display) -> Error {
    Error(format!("{context}: {e}"))
}

fn canonical(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// 저장소의 워크트리들: 주 작업트리(bare가 아니면) 먼저, 그다음 링크된 워크트리를 이름 순으로.
pub fn list(repo: &Repo) -> Result<Vec<Worktree>> {
    let local = repo.local();
    let current = canonical(repo.workdir());
    let mut out = Vec::new();
    let main = local.main_repo().map_err(|e| err("주 저장소", e))?;
    if let Some(workdir) = main.workdir() {
        let path = canonical(workdir);
        out.push(describe(&main, &path, None, false, &current));
    }
    let mut linked = Vec::new();
    for proxy in local.worktrees().map_err(|e| err("워크트리 목록", e))? {
        let id = proxy.id().ok().map(|id| id.to_str_lossy().into_owned());
        let locked = proxy.is_locked();
        let base = proxy.base().ok();
        let Ok(wt) = proxy.into_repo_with_possibly_inaccessible_worktree() else {
            continue;
        };
        let path = base.as_deref().map_or_else(PathBuf::new, canonical_lenient);
        linked.push(describe(
            &wt,
            &path,
            Some(id.unwrap_or_default()),
            locked,
            &current,
        ));
    }
    linked.sort_by(|a, b| a.name.cmp(&b.name));
    out.extend(linked);
    Ok(out)
}

/// 지운 폴더는 canonicalize하지 못하므로 있는 부모까지만 맞춘다.
fn canonical_lenient(path: &Path) -> PathBuf {
    if let Ok(path) = dunce::canonicalize(path) {
        return path;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => canonical(parent).join(name),
        _ => path.to_owned(),
    }
}

/// `id`가 없으면 주 작업트리다.
fn describe(
    repo: &gix::Repository,
    path: &Path,
    id: Option<String>,
    locked: bool,
    current: &Path,
) -> Worktree {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| id.clone())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let mut worktree = Worktree {
        path: path.to_string_lossy().into_owned(),
        name,
        main: id.is_none(),
        id,
        current: path == current,
        branch: None,
        head: None,
        summary: None,
        time: None,
        locked,
        missing: !path.is_dir(),
    };
    if let Ok(head) = repo.head() {
        worktree.branch = head.referent_name().and_then(|name| {
            name.as_bstr()
                .strip_prefix(b"refs/heads/")
                .map(|b| b.to_str_lossy().into_owned())
        });
        if let Some(id) = head.id() {
            worktree.head = Some(id.to_string());
            if let Ok(commit) = repo.find_commit(id.detach())
                && let Ok(decoded) = commit.decode()
            {
                let message = decode_text(decoded.message, decoded.encoding);
                worktree.summary = Some(message.lines().next().unwrap_or("").trim().to_owned());
                worktree.time = decoded
                    .committer()
                    .ok()
                    .and_then(|s| s.time().ok())
                    .map(|t| t.seconds);
            }
        }
    }
    worktree
}

/// 다른 워크트리가 체크아웃한 것: 브랜치 이름 → 워크트리 이름, detached HEAD 커밋 → 워크트리 이름.
/// 지금 보고 있는 작업트리는 뺀다(그 HEAD는 그래프가 따로 표시한다).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CheckedOut {
    pub branches: HashMap<String, String>,
    pub detached: Vec<(ObjectId, String)>,
}

pub fn checked_out_elsewhere(repo: &Repo) -> CheckedOut {
    let mut out = CheckedOut::default();
    let Ok(worktrees) = list(repo) else {
        return out;
    };
    for worktree in worktrees.into_iter().filter(|w| !w.current) {
        match (worktree.branch, worktree.head) {
            (Some(branch), _) => {
                out.branches.insert(branch, worktree.name);
            }
            (None, Some(head)) => {
                if let Ok(id) = ObjectId::from_hex(head.as_bytes()) {
                    out.detached.push((id, worktree.name));
                }
            }
            (None, None) => {}
        }
    }
    out
}
