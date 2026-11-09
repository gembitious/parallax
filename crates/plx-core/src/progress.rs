//! 진행 중인 git 작업(M15): merge·rebase·cherry-pick·revert·am·bisect가 멈춘 상태와 충돌 파일.
//!
//! git이 워크트리별 git 디렉터리에 남기는 파일(`MERGE_HEAD`, `rebase-merge/`, `CHERRY_PICK_HEAD` …)과 인덱스의 충돌
//! 단계만 읽는다(git을 띄우지 않는다, 읽기 전용). 어느 작업인지 고르는 차례는 gix의 `Repository::state`(git의
//! `wt_status_get_state`와 같은 차례)를 따르고, 단계·브랜치·적용 중인 커밋은 그 작업의 파일에서 읽는다. 결과는
//! `git status`가 알리는 것과 같다(골든 테스트).

use gix::ObjectId;
use serde::Serialize;

use crate::repo::{Repo, Result, decode_text, err};

/// 멈춰 있는 작업의 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    /// `git am`(메일 패치 적용)
    Am,
    Bisect,
}

/// 적용 중인 커밋(rebase·cherry-pick·revert가 멈춘 커밋, merge의 상대 쪽)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applying {
    pub sha: String,
    /// 커밋을 읽지 못했으면(지워졌거나 받지 않았다) 없다
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub operation: Operation,
    /// rebase·am의 지금 단계(1부터)와 모두. 모르면 없다
    pub step: Option<u32>,
    pub total: Option<u32>,
    /// rebase하는 브랜치(짧은 이름, detached HEAD를 rebase하면 없다), bisect를 시작한 브랜치(또는 커밋)
    pub branch: Option<String>,
    /// rebase의 새 밑(onto) 커밋
    pub onto: Option<String>,
    /// 적용 중인 커밋. merge는 합치는 커밋 모두(octopus면 여럿)
    pub applying: Vec<Applying>,
    /// cherry-pick·revert가 여러 커밋을 차례로 하는 중이다(`sequencer/todo`)
    pub sequence: bool,
}

/// 진행 중인 작업(없으면 `None`)
pub fn read(repo: &Repo) -> Result<Option<Progress>> {
    use gix::state::InProgress as S;
    let local = repo.local();
    let Some(state) = local.state() else {
        return Ok(None);
    };
    let dir = local.path().to_owned();
    let text = |name: &str| -> Option<String> {
        let bytes = std::fs::read(dir.join(name)).ok()?;
        let text = String::from_utf8_lossy(&bytes).trim().to_owned();
        (!text.is_empty()).then_some(text)
    };
    let number = |name: &str| text(name).and_then(|t| t.parse::<u32>().ok());
    let heads = |name: &str| -> Vec<ObjectId> {
        text(name)
            .map(|t| {
                t.lines()
                    .filter_map(|line| ObjectId::from_hex(line.trim().as_bytes()).ok())
                    .collect()
            })
            .unwrap_or_default()
    };
    let branch_of =
        |name: &str| text(name).and_then(|t| t.strip_prefix("refs/heads/").map(str::to_owned));
    let applying = |ids: Vec<ObjectId>| -> Vec<Applying> {
        ids.into_iter()
            .map(|id| Applying {
                sha: id.to_string(),
                summary: summary(&local, id),
            })
            .collect()
    };
    let mut progress = Progress {
        operation: Operation::Merge,
        step: None,
        total: None,
        branch: None,
        onto: None,
        applying: Vec::new(),
        sequence: false,
    };
    match state {
        S::Rebase | S::RebaseInteractive | S::ApplyMailboxRebase => {
            let sub = if dir.join("rebase-merge").is_dir() {
                "rebase-merge"
            } else {
                "rebase-apply"
            };
            let (step, total) = if sub == "rebase-merge" {
                ("msgnum", "end")
            } else {
                ("next", "last")
            };
            progress.operation = Operation::Rebase;
            progress.step = number(&format!("{sub}/{step}"));
            progress.total = number(&format!("{sub}/{total}"));
            progress.branch = branch_of(&format!("{sub}/head-name"));
            progress.onto =
                text(&format!("{sub}/onto")).filter(|t| ObjectId::from_hex(t.as_bytes()).is_ok());
            // 멈춘 커밋: REBASE_HEAD(충돌·edit), 없으면 rebase-merge의 stopped-sha
            let mut current = heads("REBASE_HEAD");
            if current.is_empty() {
                current = text(&format!("{sub}/stopped-sha"))
                    .and_then(|t| local.rev_parse_single(t.as_str()).ok())
                    .map(|id| vec![id.detach()])
                    .unwrap_or_default();
            }
            progress.applying = applying(current);
        }
        S::ApplyMailbox => {
            progress.operation = Operation::Am;
            progress.step = number("rebase-apply/next");
            progress.total = number("rebase-apply/last");
        }
        S::CherryPick | S::CherryPickSequence => {
            progress.operation = Operation::CherryPick;
            progress.sequence = state == S::CherryPickSequence;
            progress.applying = applying(heads("CHERRY_PICK_HEAD"));
        }
        S::Revert | S::RevertSequence => {
            progress.operation = Operation::Revert;
            progress.sequence = state == S::RevertSequence;
            progress.applying = applying(heads("REVERT_HEAD"));
        }
        S::Merge => {
            progress.operation = Operation::Merge;
            progress.applying = applying(heads("MERGE_HEAD"));
        }
        S::Bisect => {
            progress.operation = Operation::Bisect;
            progress.branch = text("BISECT_START");
        }
    }
    Ok(Some(progress))
}

fn summary(repo: &gix::Repository, id: ObjectId) -> Option<String> {
    let commit = repo.find_commit(id).ok()?;
    let decoded = commit.decode().ok()?;
    let encoding = decoded
        .extra_headers()
        .find("encoding")
        .map(|value| value.to_owned());
    Some(decode_text(
        decoded.message_summary().as_ref(),
        encoding.as_ref().map(|e| e.as_ref()),
    ))
}

/// 인덱스에서 충돌 중인(단계 1~3이 있는) 경로, 경로 순. UTF-8이 아닌 경로는 뺀다.
pub fn conflicts(repo: &Repo) -> Result<Vec<String>> {
    let index = repo
        .local()
        .index_or_empty()
        .map_err(|e| err("인덱스", e))?;
    let mut paths: Vec<String> = Vec::new();
    for entry in index.entries() {
        if entry.stage_raw() == 0 {
            continue;
        }
        let Ok(path) = std::str::from_utf8(entry.path(&index)) else {
            continue;
        };
        if paths.last().map(String::as_str) != Some(path) {
            paths.push(path.to_owned());
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
