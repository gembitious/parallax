//! reflog·stash 목록(M10). HEAD의 reflog(`git reflog`)와 `refs/stash`의 reflog(`git stash list`)를 gix로 읽는다
//! (git을 띄우지 않는다). 최근 것부터, 항목마다 그때 HEAD(stash)가 가리킨 커밋의 요약을 붙인다.
//!
//! reflog는 저장소를 바꾸지 않고 읽기만 한다. 사라진 커밋(gc로 지운 것)은 `exists: false`로 보이고, 줄이 깨진 항목은
//! 건너뛴다(git도 읽지 못하는 줄은 보이지 않는다). HEAD reflog는 워크트리마다 따로고(`worktrees/<이름>/logs/HEAD`),
//! stash는 모든 워크트리가 함께 쓴다.

use std::collections::HashMap;

use gix::ObjectId;
use gix::bstr::{BStr, ByteSlice};
use serde::{Deserialize, Serialize};

use crate::repo::{Error, Person, Repo, Result, decode_text};

/// 한 번에 보일 항목 수의 한도. reflog는 보통 90일(gc.reflogExpire)이라 이보다 짧다.
pub const LIMIT: usize = 5_000;

/// 어느 reflog인가
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Which {
    /// HEAD의 reflog(이 워크트리)
    Head,
    /// `refs/stash`의 reflog(stash 목록)
    Stash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflogEntry {
    /// `HEAD@{n}`·`stash@{n}`의 n(0이 가장 최근)
    pub index: usize,
    /// 그 일 뒤에 가리킨 커밋
    pub sha: String,
    /// 그 일 앞에 가리킨 커밋(처음 생긴 참조면 없다)
    pub previous: Option<String>,
    /// 일의 종류(`commit`, `checkout`, `reset` …, 메시지의 `:` 앞). stash는 없다
    pub action: Option<String>,
    /// 메시지(`:` 뒤, stash는 `WIP on main: …` 그대로)
    pub message: String,
    /// 그 일을 한 사람과 시각
    pub who: Person,
    /// 커밋의 요약(첫 줄). 커밋이 없으면(gc로 지웠다) 없다
    pub summary: Option<String>,
    pub exists: bool,
    /// stash에 추적하지 않던 파일이 있으면(`stash -u`) 그 파일들을 담은 커밋(셋째 부모)
    pub untracked: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reflog {
    /// 작업트리 경로
    pub repo: String,
    pub which: Which,
    pub entries: Vec<ReflogEntry>,
    /// [`LIMIT`]에서 멈췄다
    pub truncated: bool,
}

/// reflog를 최근 것부터 읽는다. reflog 파일이 없으면(stash를 한 번도 하지 않았다) 빈 목록이다.
pub fn read(repo: &Repo, which: Which) -> Result<Reflog> {
    let local = repo.local();
    match which {
        Which::Head => {
            let head = local.head().map_err(|e| Error(format!("HEAD: {e}")))?;
            read_lines(repo, which, head.log_iter())
        }
        Which::Stash => match local
            .try_find_reference("refs/stash")
            .map_err(|e| Error(format!("stash: {e}")))?
        {
            Some(reference) => read_lines(repo, which, reference.log_iter()),
            None => Ok(Reflog {
                repo: repo.workdir().to_string_lossy().into_owned(),
                which,
                entries: Vec::new(),
                truncated: false,
            }),
        },
    }
}

fn read_lines(
    repo: &Repo,
    which: Which,
    mut platform: gix::refs::file::log::iter::Platform<'_, '_>,
) -> Result<Reflog> {
    let local = repo.local();
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut commits: HashMap<ObjectId, Option<(String, Option<String>)>> = HashMap::new();
    let lines = platform
        .rev()
        .map_err(|e| Error(format!("reflog 읽기: {e}")))?;
    if let Some(lines) = lines {
        // 깨진 줄은 git처럼 번호(`@{n}`)를 차지하지 않는다. 사라진 커밋의 줄은 차지한다.
        for line in lines {
            let Ok(line) = line else { continue };
            if entries.len() == LIMIT {
                truncated = true;
                break;
            }
            let index = entries.len();
            let summary = commits
                .entry(line.new_oid)
                .or_insert_with(|| commit_summary(&local, line.new_oid, which))
                .clone();
            let (action, message) = split_message(line.message.as_bstr(), which);
            let time = line.signature.time;
            entries.push(ReflogEntry {
                index,
                sha: line.new_oid.to_string(),
                previous: (!line.previous_oid.is_null()).then(|| line.previous_oid.to_string()),
                action,
                message,
                who: Person {
                    name: decode_text(line.signature.name.as_bstr(), None),
                    email: decode_text(line.signature.email.as_bstr(), None),
                    time: time.seconds,
                    tz_offset: time.offset,
                },
                exists: summary.is_some(),
                untracked: summary.as_ref().and_then(|(_, u)| u.clone()),
                summary: summary.map(|(s, _)| s),
            });
        }
    }
    Ok(Reflog {
        repo: repo.workdir().to_string_lossy().into_owned(),
        which,
        entries,
        truncated,
    })
}

/// 커밋 요약과 (stash면) 셋째 부모. 커밋이 없으면 `None`
fn commit_summary(
    local: &gix::Repository,
    id: ObjectId,
    which: Which,
) -> Option<(String, Option<String>)> {
    let commit = local.find_commit(id).ok()?;
    let decoded = commit.decode().ok()?;
    let encoding = decoded
        .extra_headers()
        .find("encoding")
        .map(|value| value.as_bstr());
    let summary = decode_text(decoded.message_summary().as_ref(), encoding);
    let untracked = match which {
        Which::Stash => decoded.parents().nth(2).map(|id| id.to_string()),
        Which::Head => None,
    };
    Some((summary, untracked))
}

/// reflog 메시지를 일의 종류와 나머지로 나눈다. HEAD는 `checkout: moving from a to b`처럼 `종류: 내용`이고
/// (`commit (amend): …`, `rebase (finish): …`처럼 종류에 괄호가 붙기도 한다) stash는 메시지 그대로다.
/// 종류가 없는 줄(`update by push` 같은 손으로 쓴 메시지)은 메시지만 있다.
pub fn split_message(message: &BStr, which: Which) -> (Option<String>, String) {
    let text = message.to_str_lossy();
    let text = text.trim_end_matches(['\n', '\r']);
    if which == Which::Stash {
        return (None, text.to_owned());
    }
    match text.split_once(": ") {
        // 종류는 짧은 낱말 몇 개다(`merge origin/main`, `pull --rebase (finish)`). 긴 것은 메시지 안의 `: `다.
        Some((action, rest)) if !action.is_empty() && action.len() <= 60 => {
            (Some(action.to_owned()), rest.to_owned())
        }
        _ => (None, text.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_action() {
        let s = |m: &str| split_message(m.into(), Which::Head);
        assert_eq!(
            s("checkout: moving from main to dev"),
            (Some("checkout".into()), "moving from main to dev".into())
        );
        assert_eq!(
            s("commit (amend): fix: typo"),
            (Some("commit (amend)".into()), "fix: typo".into())
        );
        assert_eq!(s("update by push"), (None, "update by push".into()));
        assert_eq!(s(""), (None, String::new()));
        assert_eq!(
            split_message("WIP on main: abc msg".into(), Which::Stash),
            (None, "WIP on main: abc msg".into())
        );
    }
}
