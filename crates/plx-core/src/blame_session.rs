//! 파일 하나의 blame 요청 처리. 앱(Tauri 명령)은 이벤트를 채널로 넘기기만 한다.
//!
//! 순서: 저장소 찾기 → 저장소 기준 경로 → git blame을 먼저 띄움 → 그동안 파일 내용을 읽어 [`BlameMeta`]
//! → [`Blame::stream`]으로 블록 묶음. 화면은 내용을 먼저 그리고 거터를 점진적으로 채운다.
//! 요청은 [`Sessions`]에 id로 등록하고, 화면이 다른 파일·리비전으로 넘어가면 그 id로 취소한다.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::Serialize;

use crate::blame::{Blame, BlameBlock, BlameCommit, BlameError, Canceller, Chunk, Stats};
use crate::repo::RepoCache;
use crate::text::{self, Decoded};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameMeta {
    /// 작업트리 경로(이후 요청에서 저장소를 가리킬 때 쓴다)
    pub repo: String,
    /// 저장소 기준 경로(`/` 구분)
    pub path: String,
    /// 고정 리비전이면 그 커밋 sha. 작업트리 blame이면 없다.
    pub rev: Option<String>,
    /// 바이너리 파일이면 없다.
    pub content: Option<Decoded>,
    pub binary: bool,
}

/// 화면으로 보내는 이벤트. `meta` → `chunk`* → 끝 이벤트(`done`·`error`·`cancelled`) 하나 순서다.
/// 내용을 읽기 전에 실패하거나 취소되면 `meta` 없이 끝 이벤트만 간다.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionEvent {
    Meta(BlameMeta),
    Chunk(Chunk),
    Done {
        /// 요청을 받고 내용(`meta`)을 보낼 때까지
        meta_ms: f64,
        /// git blame 측정값. 바이너리 파일이면 git을 끝까지 돌리지 않아 0이다.
        stats: Stats,
    },
    Error {
        message: String,
    },
    Cancelled,
}

impl SessionEvent {
    pub fn is_end(&self) -> bool {
        matches!(
            self,
            SessionEvent::Done { .. } | SessionEvent::Error { .. } | SessionEvent::Cancelled
        )
    }
}

/// 내용 다음에 보낼 blame.
enum Source {
    Git(Blame),
    /// 작업트리 파일이 HEAD에 없다(새 파일·추적하지 않는 파일). 모든 줄이 커밋 안 됨이다.
    /// git blame은 이 경우 실패하므로 띄우지 않고 직접 만든다.
    Uncommitted(Chunk),
    /// 바이너리 파일
    Nothing,
}

/// 요청을 열고 git blame을 띄운다. git을 먼저 띄우고 그동안 파일을 읽는다.
fn start(repos: &RepoCache, path: &Path, rev: Option<&str>) -> Result<(BlameMeta, Source), String> {
    let repo = repos.for_path(path).map_err(|e| e.to_string())?;
    let rel = repo.relative_path(path).map_err(|e| e.to_string())?;
    let commit = rev
        .map(|rev| repo.resolve_commit(rev))
        .transpose()
        .map_err(|e| e.to_string())?;
    let sha = commit.map(|c| c.to_string());
    let tracked = match commit {
        Some(_) => true,
        None => match repo.head().map_err(|e| e.to_string())? {
            Some(head) => repo.has_file(head, &rel).map_err(|e| e.to_string())?,
            None => false,
        },
    };

    // 파일을 읽는 동안 git이 일을 시작하도록 먼저 띄운다.
    let blame = tracked
        .then(|| Blame::spawn(repo.workdir(), &rel, sha.as_deref()))
        .transpose()
        .map_err(|e| format!("git blame 실행 실패: {e}"))?;
    let bytes = match commit {
        Some(commit) => repo.blob_at(commit, &rel).and_then(|blob| {
            blob.ok_or_else(|| {
                crate::repo::Error(format!(
                    "{}에 {rel} 파일이 없다",
                    short(sha.as_deref().unwrap_or(""))
                ))
            })
        }),
        None => std::fs::read(path).map_err(|e| {
            crate::repo::Error(format!("파일을 읽지 못했다 ({}): {e}", path.display()))
        }),
    };
    // 실패하거나 바이너리면 git은 버린다(Drop이 끝내고 거둔다).
    let bytes = bytes.map_err(|e| e.to_string())?;
    let binary = text::is_binary(&bytes);
    let content = (!binary).then(|| text::decode(&bytes));
    let source = match (blame, &content) {
        (_, None) => Source::Nothing,
        (Some(blame), Some(_)) => Source::Git(blame),
        (None, Some(content)) => Source::Uncommitted(uncommitted(&rel, content.line_count)),
    };
    let meta = BlameMeta {
        repo: repo.workdir().to_string_lossy().into_owned(),
        path: rel,
        rev: sha,
        content,
        binary,
    };
    Ok((meta, source))
}

/// 모든 줄이 커밋 안 됨인 묶음. git이 미커밋 줄에 쓰는 값을 따른다.
fn uncommitted(path: &str, lines: u32) -> Chunk {
    let sha = "0".repeat(40);
    let now = crate::time::epoch_ms() as i64 / 1000;
    let commit = BlameCommit {
        sha: sha.clone(),
        author: "Not Committed Yet".to_owned(),
        author_mail: "<not.committed.yet>".to_owned(),
        author_time: now,
        author_tz: "+0000".to_owned(),
        committer: "Not Committed Yet".to_owned(),
        committer_mail: "<not.committed.yet>".to_owned(),
        committer_time: now,
        committer_tz: "+0000".to_owned(),
        summary: format!("Version of {path} from {path}"),
        boundary: false,
    };
    let blocks = (lines > 0)
        .then(|| BlameBlock {
            sha,
            orig_line: 1,
            final_line: 1,
            num_lines: lines,
            path: path.to_owned(),
            previous: None,
        })
        .into_iter()
        .collect();
    Chunk {
        commits: vec![commit],
        blocks,
    }
}

/// 진행 중인 blame 요청들. 앱 상태로 하나 둔다.
#[derive(Default)]
pub struct Sessions {
    next_id: AtomicU64,
    slots: Mutex<HashMap<u64, Slot>>,
}

enum Slot {
    /// 등록만 했다(git을 띄우기 전).
    Starting,
    /// git을 띄우기 전에 취소됐다.
    Cancelled,
    Running(Canceller),
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// 새 요청 id. 이 id로 [`Sessions::run`]을 부르고 [`Sessions::cancel`]로 멈춘다.
    /// 화면은 이 id를 받은 뒤에만 취소할 수 있으므로 "시작 전 취소"도 등록된 요청에만 일어난다.
    pub fn register(&self) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.slots.lock().unwrap().insert(id, Slot::Starting);
        id
    }

    /// 요청을 멈춘다. 이미 끝났거나 모르는 id면 `false`.
    pub fn cancel(&self, id: u64) -> bool {
        let mut slots = self.slots.lock().unwrap();
        match slots.get_mut(&id) {
            Some(Slot::Running(canceller)) => canceller.cancel(),
            Some(slot) => *slot = Slot::Cancelled,
            None => return false,
        }
        true
    }

    /// 끝나지 않은 요청 수.
    pub fn active(&self) -> usize {
        self.slots.lock().unwrap().len()
    }

    /// `id` 요청을 끝까지 처리한다. 끝 이벤트는 반드시 하나 보낸다.
    /// `on_event`가 `false`를 돌려주면(받는 쪽이 사라졌다) 취소한다.
    pub fn run(
        &self,
        id: u64,
        repos: &RepoCache,
        path: &Path,
        rev: Option<&str>,
        mut on_event: impl FnMut(SessionEvent) -> bool,
    ) {
        let end = self.drive(id, repos, path, rev, &mut on_event);
        self.slots.lock().unwrap().remove(&id);
        on_event(end);
    }

    fn drive(
        &self,
        id: u64,
        repos: &RepoCache,
        path: &Path,
        rev: Option<&str>,
        on_event: &mut impl FnMut(SessionEvent) -> bool,
    ) -> SessionEvent {
        let started = Instant::now();
        if self.is_cancelled(id) {
            return SessionEvent::Cancelled;
        }
        let (meta, source) = match start(repos, path, rev) {
            Ok(started) => started,
            Err(message) => return SessionEvent::Error { message },
        };
        // 시작하는 동안 취소가 왔으면 끝낸다(git은 Drop이 끝낸다). 아니면 이제부터 취소는 git을 바로 끝낸다.
        {
            let mut slots = self.slots.lock().unwrap();
            if matches!(slots.get(&id), Some(Slot::Cancelled)) {
                return SessionEvent::Cancelled;
            }
            if let Source::Git(blame) = &source {
                slots.insert(id, Slot::Running(blame.canceller()));
            }
        }
        let meta_ms = started.elapsed().as_secs_f64() * 1000.0;
        let sent = on_event(SessionEvent::Meta(meta));
        let blame = match source {
            Source::Git(blame) => blame,
            Source::Uncommitted(chunk) => {
                let stats = Stats {
                    blocks: chunk.blocks.len(),
                    commits: chunk.commits.len(),
                    ..Stats::default()
                };
                if sent && !on_event(SessionEvent::Chunk(chunk)) {
                    return SessionEvent::Cancelled;
                }
                return SessionEvent::Done { meta_ms, stats };
            }
            Source::Nothing => {
                return SessionEvent::Done {
                    meta_ms,
                    stats: Stats::default(),
                };
            }
        };
        let canceller = blame.canceller();
        if !sent {
            canceller.cancel();
        }
        let result = blame.stream(|chunk| {
            if !on_event(SessionEvent::Chunk(chunk)) {
                canceller.cancel();
            }
        });
        match result {
            Ok(stats) => SessionEvent::Done { meta_ms, stats },
            Err(BlameError::Cancelled) => SessionEvent::Cancelled,
            Err(e) => SessionEvent::Error {
                message: e.to_string(),
            },
        }
    }

    fn is_cancelled(&self, id: u64) -> bool {
        matches!(self.slots.lock().unwrap().get(&id), Some(Slot::Cancelled))
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}
