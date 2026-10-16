//! 파일 하나의 blame 요청 처리. 앱(Tauri 명령)은 이벤트를 채널로 넘기기만 한다.
//!
//! 순서: 저장소 찾기 → 저장소 기준 경로 → 리비전은 git blame을 먼저 띄우고 그동안 blob을 읽고, 작업트리는 파일을
//! 읽어 그 내용으로 git blame을 띄움(`--contents -`) → [`BlameMeta`] → [`Blame::stream`]으로 블록 묶음.
//! 화면은 내용을 먼저 그리고 거터를 점진적으로 채운다.
//! 요청은 [`Sessions`]에 id로 등록하고, 화면이 다른 파일·리비전으로 넘어가면 그 id로 취소한다.

use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::blame::{Blame, BlameBlock, BlameCommit, BlameError, Chunk, Stats};
use crate::cache::{CacheKey, DiskCache};
use crate::repo::{FilePath, Repo, RepoCache};
use crate::requests::Requests;
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
    /// git blame을 돌린다. 끝까지 오면 열쇠로 캐시에 넣는다.
    Git(Blame, Option<CacheKey>),
    /// 캐시에서 읽었다.
    Cached(Chunk),
    /// 추적하지 않는 작업트리 파일(HEAD에도 인덱스에도 없다). 모든 줄이 커밋 안 됨이다.
    /// git blame은 이 경우 실패하므로 띄우지 않고 직접 만든다.
    Uncommitted(Chunk),
    /// 바이너리 파일
    Nothing,
}

/// 요청을 열고 git blame을 띄운다. 캐시에 있으면 git을 띄우지 않는다.
/// 리비전은 git을 먼저 띄우고 그동안 blob을 읽는다. 작업트리는 파일을 먼저 읽어 그 내용을 git에 준다
/// (`--contents -`): 화면에 보이는 내용과 blame한 내용이 같고, 내용으로 캐시 열쇠를 만든다.
fn start(
    repos: &RepoCache,
    cache: Option<&DiskCache>,
    file: &FilePath,
    rev: Option<&str>,
) -> Result<(BlameMeta, Source), String> {
    let (repo, rel) = repos.locate(file).map_err(|e| e.to_string())?;
    let commit = rev
        .map(|rev| repo.resolve_commit(rev))
        .transpose()
        .map_err(|e| e.to_string())?;
    let Some(commit) = commit else {
        return start_worktree(&repo, cache, &rel);
    };
    let sha = commit.to_string();
    // 얕은 클론(이력을 더 받으면)과 부모를 바꾼 저장소(`git replace`)는 결과가 바뀐다.
    let key = cache
        .filter(|_| repo.cacheable())
        .map(|_| CacheKey::commit(repo.workdir(), &sha, &rel, &repo.blame_settings()));
    let cached = key
        .as_ref()
        .zip(cache)
        .and_then(|(key, cache)| cache.get(key));
    // blob을 읽는 동안 git이 일을 시작하도록 먼저 띄운다.
    let blame = cached
        .is_none()
        .then(|| Blame::spawn(repo.workdir(), &rel, Some(&sha)))
        .transpose()
        .map_err(|e| format!("git blame 실행 실패: {e}"))?;
    // 실패하거나 바이너리면 git은 버린다(Drop이 끝내고 거둔다).
    let bytes = repo
        .blob_at(commit, &rel)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{}에 {rel} 파일이 없다", short(&sha)))?;
    let binary = text::is_binary(&bytes);
    let content = (!binary).then(|| text::decode(&bytes));
    let source = match (blame, cached, &content) {
        (_, _, None) => Source::Nothing,
        (Some(blame), _, Some(_)) => Source::Git(blame, key),
        (None, Some(chunk), Some(_)) => Source::Cached(chunk),
        (None, None, Some(_)) => unreachable!("캐시에 없으면 git을 띄운다"),
    };
    let meta = BlameMeta {
        repo: repo.workdir().to_string_lossy().into_owned(),
        path: rel,
        rev: Some(sha),
        content,
        binary,
    };
    Ok((meta, source))
}

fn start_worktree(
    repo: &Repo,
    cache: Option<&DiskCache>,
    rel: &str,
) -> Result<(BlameMeta, Source), String> {
    // 심볼릭 링크는 가리키는 파일을 blame한다(터미널에서 준 경로도 실제 경로로 푼다).
    let rel = repo.resolve_link(rel).map_err(|e| e.to_string())?;
    let bytes = repo
        .worktree_bytes(&rel)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("작업트리에 {rel} 파일이 없다"))?;
    let binary = text::is_binary(&bytes);
    let content = (!binary).then(|| text::decode(&bytes));
    let meta = |content| BlameMeta {
        repo: repo.workdir().to_string_lossy().into_owned(),
        path: rel.clone(),
        rev: None,
        content,
        binary,
    };
    let Some(decoded) = content else {
        return Ok((meta(None), Source::Nothing));
    };
    let head = repo.head().map_err(|e| e.to_string())?;
    let in_head = match head {
        Some(head) => repo.has_file(head, &rel).map_err(|e| e.to_string())?,
        None => false,
    };
    // 인덱스에만 있는 파일(새로 add했거나 `git mv`로 옮겼다)도 git이 blame한다: 옮긴 파일은 원래 이력을 찾는다.
    // 커밋이 하나도 없으면 git blame은 실패한다.
    let tracked = in_head || (head.is_some() && repo.in_index(&rel).map_err(|e| e.to_string())?);
    if !tracked {
        let chunk = uncommitted(&rel, decoded.line_count);
        return Ok((meta(Some(decoded)), Source::Uncommitted(chunk)));
    }
    // 인덱스에만 있는 파일의 결과는 인덱스에 따라서도 바뀌므로 넣지 않는다.
    let key = match head {
        Some(head) if in_head && cache.is_some() && repo.cacheable() => Some(CacheKey::worktree(
            repo.workdir(),
            &head.to_string(),
            &rel,
            &repo.blame_settings(),
            &bytes,
            &repo.to_git(&rel, &bytes),
        )),
        _ => None,
    };
    if let Some(chunk) = key
        .as_ref()
        .zip(cache)
        .and_then(|(key, cache)| cache.get(key))
    {
        return Ok((meta(Some(decoded)), Source::Cached(chunk)));
    }
    let blame = Blame::spawn_contents(repo.workdir(), &rel, bytes)
        .map_err(|e| format!("git blame 실행 실패: {e}"))?;
    Ok((meta(Some(decoded)), Source::Git(blame, key)))
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
    requests: Requests,
    cache: Option<Arc<DiskCache>>,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// 끝까지 온 blame을 `cache`에 넣고, 같은 요청은 git을 띄우지 않고 캐시에서 읽는다.
    pub fn with_cache(cache: Arc<DiskCache>) -> Self {
        Sessions {
            cache: Some(cache),
            ..Self::default()
        }
    }

    pub fn cache(&self) -> Option<&DiskCache> {
        self.cache.as_deref()
    }

    /// 새 요청 id. 이 id로 [`Sessions::run`]을 부르고 [`Sessions::cancel`]로 멈춘다.
    pub fn register(&self) -> u64 {
        self.requests.register()
    }

    /// 요청을 멈춘다. 이미 끝났거나 모르는 id면 `false`.
    pub fn cancel(&self, id: u64) -> bool {
        self.requests.cancel(id)
    }

    /// 끝나지 않은 요청 수.
    pub fn active(&self) -> usize {
        self.requests.active()
    }

    /// `id` 요청을 끝까지 처리한다. 끝 이벤트는 반드시 하나 보낸다.
    /// `on_event`가 `false`를 돌려주면(받는 쪽이 사라졌다) 취소한다.
    pub fn run(
        &self,
        id: u64,
        repos: &RepoCache,
        file: impl Into<FilePath>,
        rev: Option<&str>,
        mut on_event: impl FnMut(SessionEvent) -> bool,
    ) {
        let end = self.drive(id, repos, &file.into(), rev, &mut on_event);
        self.requests.finish(id);
        on_event(end);
    }

    fn drive(
        &self,
        id: u64,
        repos: &RepoCache,
        file: &FilePath,
        rev: Option<&str>,
        on_event: &mut impl FnMut(SessionEvent) -> bool,
    ) -> SessionEvent {
        let started = Instant::now();
        if self.requests.is_cancelled(id) {
            return SessionEvent::Cancelled;
        }
        let (meta, source) = match start(repos, self.cache(), file, rev) {
            Ok(started) => started,
            Err(message) => return SessionEvent::Error { message },
        };
        // 시작하는 동안 취소가 왔으면 끝낸다(git은 Drop이 끝낸다). 아니면 이제부터 취소는 git을 바로 끝낸다.
        let attached = match &source {
            Source::Git(blame, _) => self.requests.attach(id, blame.canceller()),
            _ => !self.requests.is_cancelled(id),
        };
        if !attached {
            return SessionEvent::Cancelled;
        }
        let meta_ms = started.elapsed().as_secs_f64() * 1000.0;
        let sent = on_event(SessionEvent::Meta(meta));
        let (blame, key) = match source {
            Source::Git(blame, key) => (blame, key),
            Source::Cached(chunk) => return whole(chunk, true, sent, meta_ms, on_event),
            Source::Uncommitted(chunk) => return whole(chunk, false, sent, meta_ms, on_event),
            Source::Nothing => {
                return SessionEvent::Done {
                    meta_ms,
                    stats: Stats::default(),
                };
            }
        };
        let canceller = blame.canceller();
        // 받는 쪽이 사라졌다. git이 이미 다 냈어도(마지막 묶음에서 알았다) 끝은 취소다.
        let mut gone = !sent;
        if gone {
            canceller.cancel();
        }
        // 캐시에 넣을 결과를 모은다.
        let mut all = key.as_ref().map(|_| Chunk::default());
        let result = blame.stream(|chunk| {
            if let Some(all) = &mut all {
                all.commits.extend_from_slice(&chunk.commits);
                all.blocks.extend_from_slice(&chunk.blocks);
            }
            if !gone && !on_event(SessionEvent::Chunk(chunk)) {
                gone = true;
                canceller.cancel();
            }
        });
        match result {
            Ok(stats) => {
                // 다 받은 결과는 받는 쪽이 사라졌어도 넣어 둔다(다음에 열 때 쓴다).
                if let (Some(cache), Some(key), Some(all)) = (self.cache(), &key, &all)
                    && let Err(e) = cache.put(key, all)
                {
                    eprintln!("plx: blame 캐시에 쓰지 못했다: {e}");
                }
                if gone {
                    SessionEvent::Cancelled
                } else {
                    SessionEvent::Done { meta_ms, stats }
                }
            }
            Err(BlameError::Cancelled) => SessionEvent::Cancelled,
            Err(e) => SessionEvent::Error {
                message: e.to_string(),
            },
        }
    }
}

/// 한 번에 다 있는 결과(캐시, 모두 미커밋)를 묶음 하나로 보낸다.
fn whole(
    chunk: Chunk,
    cached: bool,
    sent: bool,
    meta_ms: f64,
    on_event: &mut impl FnMut(SessionEvent) -> bool,
) -> SessionEvent {
    let stats = Stats {
        blocks: chunk.blocks.len(),
        commits: chunk.commits.len(),
        cached,
        ..Stats::default()
    };
    if !sent || !on_event(SessionEvent::Chunk(chunk)) {
        return SessionEvent::Cancelled;
    }
    SessionEvent::Done { meta_ms, stats }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}
