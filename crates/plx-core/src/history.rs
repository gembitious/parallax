//! 파일 히스토리 요청: `git log -- <경로>`를 스트리밍하고, rename으로 생긴 파일은 옛 이름의 이력으로 잇는다.
//!
//! `git log --follow`를 쓰지 않는다(측정: bench/results.md):
//! - rename 앞 구간에서 changed-path Bloom 필터를 거의 못 써서 느리다.
//! - diff가 없는 merge 커밋을 빼서 merge에서 바뀐 버전이 빠지고, 날짜 순으로 걷다가 경로를 바꾸므로
//!   다른 브랜치의 커밋이 빠지거나 끼어든다.
//!
//! 구간마다 그 이름으로 git log의 기본 이력 단순화를 그대로 따른다. 구간 안에서 파일이 생긴 커밋(상태 A)은
//! 첫 부모와 비교해 rename 원본을 찾는다(gix, 프로세스 없음). 원본이 있으면 그 커밋의 첫 부모부터 옛 이름으로
//! 다음 구간을 잇는다. 다음 구간의 git은 원본을 찾는 즉시 띄워 두고(채널이 차면 git이 기다린다) 앞 구간이 끝난
//! 뒤에 읽으므로 화면 목록은 뒤에 붙기만 한다. 같은 커밋은 한 번만 보낸다(이름을 되돌린 이력 등).
//!
//! git 출력은 구간마다 다른 스레드에서 읽는다. 커밋이 드문드문 나와도(긴 이력에서 몇 번 안 바뀐 파일)
//! 받은 커밋을 다음 출력까지 붙잡지 않고 16ms 안에 보낸다.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gix::ObjectId;
use serde::{Deserialize, Serialize};

use crate::cache::{CacheKey, DiskCache};
use crate::commit::FileStatus;
use crate::git::Canceller;
use crate::line_log::LineLogStream;
use crate::linemap::LineMap;
use crate::log::{EntrySource, LogEntry, LogError, LogStream};
use crate::repo::{FilePath, Repo, RepoCache};
use crate::requests::Requests;
use crate::text;

/// 성능 예산의 "첫 50개"
pub const PAGE: usize = 50;
/// 첫 묶음 뒤로는 이 간격마다 모아서 보낸다.
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
/// 이름을 이만큼 넘게 거슬러 올라가지 않는다(이름을 주고받는 이력에서도 끝나게).
const MAX_SEGMENTS: usize = 64;
/// 캐시에서 읽은 결과는 이만큼씩 나눠 보낸다(첫 묶음은 [`PAGE`]개). 한 메시지가 너무 커지지 않게.
const REPLAY_CHUNK: usize = 5000;
/// 이보다 많은 결과(저장소 전체 히스토리 등)는 캐시에 넣지 않는다. 파일 하나가 수십 MB가 된다.
const MAX_CACHED_ENTRIES: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMeta {
    /// 작업트리 경로
    pub repo: String,
    /// 저장소 기준 경로(`/` 구분). 저장소 전체면 `.`
    pub path: String,
    /// 리비전을 정해 요청했으면 그 커밋 sha
    pub rev: Option<String>,
    /// 이력을 거슬러 올라가기 시작한 커밋. 커밋이 없는 저장소면 없다.
    pub start: Option<String>,
    /// 라인 히스토리면 시작 커밋 버전의 줄 범위(작업트리 줄 번호로 요청했으면 HEAD 버전으로 옮긴 범위)
    pub lines: Option<LineRange>,
}

/// 줄 범위(1부터, 양쪽 포함)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

/// 측정값. 시각은 모두 요청을 받은 때부터(ms)다.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStats {
    /// git이 [`PAGE`]개째 커밋을 내놓은 때(그보다 적으면 마지막 git이 끝난 때). 성능 예산의 기준
    /// ("git log 첫 50개 + 30ms")이다. 캐시에서 읽었으면 없다.
    pub git_page_ms: Option<f64>,
    pub first_entry_ms: Option<f64>,
    /// [`PAGE`]개째를 보낸 때. 그보다 적으면 없다.
    pub first_page_ms: Option<f64>,
    pub total_ms: f64,
    pub entries: usize,
    /// git log를 돌린 구간 수(rename마다 하나씩 는다)
    pub segments: usize,
    /// git을 띄우지 않고 캐시에서 읽었다.
    pub cached: bool,
}

/// 화면으로 보내는 이벤트. `meta` → `entries`* → 끝 이벤트(`done`·`error`·`cancelled`) 하나 순서다.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HistoryEvent {
    Meta(HistoryMeta),
    Entries {
        entries: Vec<LogEntry>,
    },
    Done {
        /// 요청을 받고 `meta`를 보낼 때까지
        meta_ms: f64,
        stats: HistoryStats,
    },
    Error {
        message: String,
    },
    Cancelled,
}

impl HistoryEvent {
    pub fn is_end(&self) -> bool {
        matches!(
            self,
            HistoryEvent::Done { .. } | HistoryEvent::Error { .. } | HistoryEvent::Cancelled
        )
    }
}

/// 캐시에 넣는 결과
#[derive(Serialize, Deserialize)]
struct Cached {
    entries: Vec<LogEntry>,
    segments: usize,
}

enum Plan<S> {
    Git {
        repo: Arc<Repo>,
        stream: Box<S>,
        key: Option<CacheKey>,
    },
    Cached(Cached),
    /// 커밋이 없는 저장소
    Empty,
}

fn start(
    repos: &RepoCache,
    cache: Option<&DiskCache>,
    file: &FilePath,
    rev: Option<&str>,
) -> Result<(HistoryMeta, Plan<LogStream>), String> {
    let (repo, mut rel) = repos.locate(file).map_err(|e| e.to_string())?;
    if rel.is_empty() {
        rel = ".".to_owned();
    }
    let start = match rev {
        Some(rev) => Some(repo.resolve_commit(rev).map_err(|e| e.to_string())?),
        None => repo.head().map_err(|e| e.to_string())?,
    };
    let meta = HistoryMeta {
        repo: repo.workdir().to_string_lossy().into_owned(),
        path: rel.clone(),
        rev: rev.and(start).map(|id| id.to_string()),
        start: start.map(|id| id.to_string()),
        lines: None,
    };
    let Some(start) = start else {
        return Ok((meta, Plan::Empty));
    };
    // 얕은 클론(이력을 더 받으면)과 부모를 바꾼 저장소(`git replace`)는 결과가 바뀐다.
    let key = cache.filter(|_| repo.cacheable()).map(|_| {
        CacheKey::history(
            repo.workdir(),
            &start.to_string(),
            &rel,
            &repo.history_settings(),
        )
    });
    if let Some(hit) = key
        .as_ref()
        .zip(cache)
        .and_then(|(key, cache)| cache.get::<Cached>(key))
    {
        return Ok((meta, Plan::Cached(hit)));
    }
    let stream = LogStream::spawn(repo.workdir(), &[start.to_string()], &rel)
        .map_err(|e| format!("git log 실행 실패: {e}"))?;
    Ok((
        meta,
        Plan::Git {
            repo,
            stream: Box::new(stream),
            key,
        },
    ))
}

/// 라인 히스토리를 연다. `rev`가 없으면 `lines`는 작업트리 파일의 줄이고, HEAD 버전의 줄로 옮겨 `git log -L`에 준다
/// (바꾸거나 새로 쓴 줄은 그 부분이 시작하는 줄로 간다).
fn start_lines(
    repos: &RepoCache,
    cache: Option<&DiskCache>,
    file: &FilePath,
    rev: Option<&str>,
    lines: LineRange,
) -> Result<(HistoryMeta, Plan<LineLogStream>), String> {
    let (repo, rel) = repos.locate(file).map_err(|e| e.to_string())?;
    let start = match rev {
        Some(rev) => Some(repo.resolve_commit(rev).map_err(|e| e.to_string())?),
        None => repo.head().map_err(|e| e.to_string())?,
    };
    let mut meta = HistoryMeta {
        repo: repo.workdir().to_string_lossy().into_owned(),
        path: rel.clone(),
        rev: rev.and(start).map(|id| id.to_string()),
        start: start.map(|id| id.to_string()),
        lines: None,
    };
    let Some(start) = start else {
        return Ok((meta, Plan::Empty));
    };
    let sha = start.to_string();
    let short = &sha[..sha.len().min(10)];
    let blob = repo
        .blob_at(start, &rel)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!("{short}에 {rel} 파일이 없다(커밋하지 않은 파일은 라인 히스토리가 없다)")
        })?;
    if text::is_binary(&blob) {
        return Err(format!(
            "바이너리 파일이라 라인 히스토리를 보지 않는다: {rel}"
        ));
    }
    let total = text::line_count(&blob);
    if total == 0 {
        return Err(format!("{short}의 {rel}은 빈 파일이다"));
    }
    let (mut first, mut last) = (lines.start.min(lines.end), lines.start.max(lines.end));
    // 작업트리 줄은 clean 모양(줄 끝을 git처럼 바꾼 것)으로 옮긴다: CRLF 작업트리도 blob과 줄 단위로 맞는다.
    if rev.is_none()
        && let Ok(Some(worktree)) = repo.worktree_clean(&rel)
        && worktree != blob
    {
        let map = LineMap::new(&blob, &worktree);
        (first, last) = (map.to_before(first), map.to_before(last));
    }
    let last = last.clamp(1, total);
    let first = first.clamp(1, last);
    meta.lines = Some(LineRange {
        start: first,
        end: last,
    });
    let key = cache.filter(|_| repo.cacheable()).map(|_| {
        CacheKey::line_history(
            repo.workdir(),
            &sha,
            &rel,
            first,
            last,
            &repo.line_history_settings(),
        )
    });
    if let Some(hit) = key
        .as_ref()
        .zip(cache)
        .and_then(|(key, cache)| cache.get::<Cached>(key))
    {
        return Ok((meta, Plan::Cached(hit)));
    }
    let stream = LineLogStream::spawn(repo.workdir(), &sha, &rel, first, last)
        .map_err(|e| format!("git log -L 실행 실패: {e}"))?;
    Ok((
        meta,
        Plan::Git {
            repo,
            stream: Box::new(stream),
            key,
        },
    ))
}

/// 진행 중인 히스토리 요청들. 앱 상태로 하나 둔다.
#[derive(Default)]
pub struct HistorySessions {
    requests: Requests,
    cache: Option<Arc<DiskCache>>,
}

impl HistorySessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// 끝까지 온 히스토리를 `cache`에 넣고, 같은 요청은 git을 띄우지 않고 캐시에서 읽는다.
    pub fn with_cache(cache: Arc<DiskCache>) -> Self {
        HistorySessions {
            cache: Some(cache),
            ..Self::default()
        }
    }

    pub fn register(&self) -> u64 {
        self.requests.register()
    }

    pub fn cancel(&self, id: u64) -> bool {
        self.requests.cancel(id)
    }

    pub fn active(&self) -> usize {
        self.requests.active()
    }

    /// `id` 요청을 끝까지 처리한다. `rev`가 없으면 HEAD부터다. 끝 이벤트는 반드시 하나 보낸다.
    /// `on_event`가 `false`를 돌려주면(받는 쪽이 사라졌다) 취소한다.
    pub fn run(
        &self,
        id: u64,
        repos: &RepoCache,
        file: impl Into<FilePath>,
        rev: Option<&str>,
        mut on_event: impl FnMut(HistoryEvent) -> bool,
    ) {
        let end = self.drive(id, repos, &file.into(), rev, &mut on_event);
        self.requests.finish(id);
        on_event(end);
    }

    /// `id` 라인 히스토리 요청을 끝까지 처리한다(`git log -L`). 끝 이벤트는 반드시 하나 보낸다.
    pub fn run_lines(
        &self,
        id: u64,
        repos: &RepoCache,
        file: impl Into<FilePath>,
        rev: Option<&str>,
        lines: LineRange,
        mut on_event: impl FnMut(HistoryEvent) -> bool,
    ) {
        let end = self.drive_lines(id, repos, &file.into(), rev, lines, &mut on_event);
        self.requests.finish(id);
        on_event(end);
    }

    fn drive_lines(
        &self,
        id: u64,
        repos: &RepoCache,
        file: &FilePath,
        rev: Option<&str>,
        lines: LineRange,
        on_event: &mut impl FnMut(HistoryEvent) -> bool,
    ) -> HistoryEvent {
        let started = Instant::now();
        if self.requests.is_cancelled(id) {
            return HistoryEvent::Cancelled;
        }
        let (meta, plan) = match start_lines(repos, self.cache.as_deref(), file, rev, lines) {
            Ok(started) => started,
            Err(message) => return HistoryEvent::Error { message },
        };
        let (stream, key) = match plan {
            Plan::Git { stream, key, .. } => (stream, key),
            Plan::Cached(cached) => return self.replay(id, meta, cached, started, on_event),
            Plan::Empty => return self.empty(id, meta, started, on_event),
        };
        let Some(reader) = self.attach(id, *stream) else {
            return HistoryEvent::Cancelled;
        };
        let meta_ms = ms(started);
        if !on_event(HistoryEvent::Meta(meta)) {
            self.requests.cancel(id);
        }
        let mut sink = Sink::new(started, key.is_some());
        let mut received = 0;
        loop {
            let entries = match self.next_batch(id, &reader, &mut sink, on_event) {
                Ok(Some(entries)) => entries,
                Ok(None) => break,
                Err(end) => return end,
            };
            received += entries.len();
            if received >= PAGE {
                sink.stats.git_page_ms.get_or_insert_with(|| ms(started));
            }
            for entry in entries {
                sink.push(entry);
            }
            if sink.due() && !sink.flush(on_event) {
                self.requests.cancel(id);
            }
        }
        self.done(sink, 1, key.as_ref(), meta_ms, on_event)
    }

    fn drive(
        &self,
        id: u64,
        repos: &RepoCache,
        file: &FilePath,
        rev: Option<&str>,
        on_event: &mut impl FnMut(HistoryEvent) -> bool,
    ) -> HistoryEvent {
        let started = Instant::now();
        if self.requests.is_cancelled(id) {
            return HistoryEvent::Cancelled;
        }
        let (meta, plan) = match start(repos, self.cache.as_deref(), file, rev) {
            Ok(started) => started,
            Err(message) => return HistoryEvent::Error { message },
        };
        let path = meta.path.clone();
        let (repo, stream, key) = match plan {
            Plan::Git { repo, stream, key } => (repo, stream, key),
            Plan::Cached(cached) => return self.replay(id, meta, cached, started, on_event),
            Plan::Empty => return self.empty(id, meta, started, on_event),
        };
        let Some(reader) = self.attach(id, *stream) else {
            return HistoryEvent::Cancelled;
        };
        let meta_ms = ms(started);
        let first_start = meta.start.clone().unwrap_or_default();
        if !on_event(HistoryEvent::Meta(meta)) {
            self.requests.cancel(id);
        }

        let mut sink = Sink::new(started, key.is_some());
        let mut received = 0;
        let mut seen = HashSet::new();
        let mut segments = 1;
        // 옛 이름을 찾지 못한 커밋이 있었다(부분 클론에서 blob이 없는 등). 이어 붙이지 못한 결과는 넣지 않는다.
        let mut incomplete = false;
        // 이미 띄운 (옛 이름, 시작 커밋). 이름을 주고받는 이력에서도 같은 구간을 다시 돌지 않는다.
        let mut spawned: HashSet<(String, String)> = HashSet::new();
        spawned.insert((path.clone(), first_start));
        // 읽을 구간들. 맨 위부터 읽는다: 파일이 rename으로 생긴 커밋에서 구간을 나눠, 옛 이름의 이력을 먼저 보이고
        // 같은 이름의 더 오래된 커밋(그 이름을 전에 쓴 다른 파일)은 그 뒤에 보인다.
        let mut stack = vec![Segment {
            path,
            reader,
            pending: Vec::new(),
        }];
        // 처음 찾은 rename에서 미리 띄운 옛 이름 구간(옛 이름, 시작 커밋). 구간을 다 읽는 동안 git이 일을 시작한다.
        let mut prefetched: Option<(String, String, Reader)> = None;
        while let Some(Segment {
            path,
            reader,
            pending,
        }) = stack.pop()
        {
            // 이 구간에서 찾은 옛 이름과 그 이름으로 거슬러 올라갈 시작 커밋들(처음 찾은 순서)
            let mut found: Vec<(String, Vec<String>)> = Vec::new();
            let mut pending = Some(pending).filter(|p| !p.is_empty());
            'read: loop {
                let entries = match pending.take() {
                    Some(entries) => entries,
                    None => match self.next_batch(id, &reader, &mut sink, on_event) {
                        Ok(Some(entries)) => {
                            received += entries.len();
                            if received >= PAGE {
                                sink.stats.git_page_ms.get_or_insert_with(|| ms(started));
                            }
                            entries
                        }
                        Ok(None) => break,
                        Err(end) => return end,
                    },
                };
                let mut entries = entries.into_iter();
                while let Some(mut entry) = entries.next() {
                    // rename으로 생긴 뒤 같은 이름이 지워진 커밋이 나오면, 그 아래는 이 이름을 전에 쓴 다른 파일이다
                    // (A → B → A, 지우고 다른 파일을 같은 이름으로 옮김). 옛 이름의 이력을 먼저 보이고 그 뒤에 읽는다.
                    if !found.is_empty()
                        && entry.status == Some(FileStatus::Deleted)
                        && entry.path == path
                    {
                        let mut rest = vec![entry];
                        rest.extend(entries);
                        stack.push(Segment {
                            path: path.clone(),
                            reader,
                            pending: rest,
                        });
                        break 'read;
                    }
                    let renamed = boundaries(&repo, &mut entry, &path, &mut found, &mut incomplete);
                    if let Some((old, parent)) = renamed
                        && prefetched.is_none()
                        && segments < MAX_SEGMENTS
                        && !spawned.contains(&(old.clone(), parent.clone()))
                    {
                        match LogStream::spawn(repo.workdir(), std::slice::from_ref(&parent), &old)
                        {
                            Ok(stream) => match self.attach(id, stream) {
                                Some(reader) => prefetched = Some((old, parent, reader)),
                                None => return HistoryEvent::Cancelled,
                            },
                            Err(e) => {
                                return HistoryEvent::Error {
                                    message: format!("git log 실행 실패: {e}"),
                                };
                            }
                        }
                    }
                    if seen.insert(entry.sha.clone()) {
                        sink.push(entry);
                    }
                }
                if sink.due() && !sink.flush(on_event) {
                    self.requests.cancel(id);
                }
            }
            // 처음 찾은 옛 이름부터 읽도록 거꾸로 쌓는다. 같은 이름의 경계는 모두 한 구간의 시작이다
            // (두 브랜치가 같은 rename을 했으면 옛 이름의 이력은 한 번에 합친다).
            for (old, starts) in found.into_iter().rev() {
                let starts: Vec<String> = starts
                    .into_iter()
                    .filter(|start| spawned.insert((old.clone(), start.clone())))
                    .collect();
                if starts.is_empty() || segments == MAX_SEGMENTS {
                    continue;
                }
                let ready = match prefetched.take() {
                    Some((name, start, reader)) if name == old && starts == [start.clone()] => {
                        Some(reader)
                    }
                    other => {
                        prefetched = other;
                        None
                    }
                };
                let reader = match ready {
                    Some(reader) => reader,
                    None => match LogStream::spawn(repo.workdir(), &starts, &old) {
                        Ok(stream) => match self.attach(id, stream) {
                            Some(reader) => reader,
                            None => return HistoryEvent::Cancelled,
                        },
                        Err(e) => {
                            return HistoryEvent::Error {
                                message: format!("git log 실행 실패: {e}"),
                            };
                        }
                    },
                };
                segments += 1;
                stack.push(Segment {
                    path: old,
                    reader,
                    pending: Vec::new(),
                });
            }
            // 미리 띄운 구간을 쓰지 않았으면(다른 시작 커밋과 합쳤다) 버린다.
            if prefetched
                .as_ref()
                .is_some_and(|(name, start, _)| spawned.contains(&(name.clone(), start.clone())))
            {
                prefetched = None;
            }
        }
        let key = key.filter(|_| !incomplete);
        self.done(sink, segments, key.as_ref(), meta_ms, on_event)
    }

    /// 다음 커밋 묶음. 기다리는 동안 쌓인 커밋은 제때(16ms) 보낸다. 출력이 끝났으면 `Ok(None)`,
    /// 실패·취소면 보낼 끝 이벤트.
    fn next_batch(
        &self,
        id: u64,
        reader: &Reader,
        sink: &mut Sink,
        on_event: &mut impl FnMut(HistoryEvent) -> bool,
    ) -> Result<Option<Vec<LogEntry>>, HistoryEvent> {
        loop {
            let message = match sink.deadline() {
                Some(deadline) => {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    match reader.rx().recv_timeout(wait) {
                        Ok(message) => message,
                        Err(RecvTimeoutError::Timeout) => {
                            if !sink.flush(on_event) {
                                self.requests.cancel(id);
                            }
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => Message::Gone,
                    }
                }
                None => reader.rx().recv().unwrap_or(Message::Gone),
            };
            return match message {
                Message::Entries(entries) => Ok(Some(entries)),
                Message::End(Ok(())) => Ok(None),
                Message::End(Err(LogError::Cancelled)) => Err(HistoryEvent::Cancelled),
                Message::End(Err(e)) => Err(HistoryEvent::Error {
                    message: e.to_string(),
                }),
                Message::Gone => Err(HistoryEvent::Error {
                    message: "git log를 읽는 스레드가 끝났다".into(),
                }),
            };
        }
    }

    /// 남은 커밋을 보내고 캐시에 넣는다.
    fn done(
        &self,
        mut sink: Sink,
        segments: usize,
        key: Option<&CacheKey>,
        meta_ms: f64,
        on_event: &mut impl FnMut(HistoryEvent) -> bool,
    ) -> HistoryEvent {
        let started = sink.started;
        sink.stats.git_page_ms.get_or_insert_with(|| ms(started));
        if !sink.flush(on_event) {
            return HistoryEvent::Cancelled;
        }
        let (entries, mut stats) = sink.finish();
        stats.segments = segments;
        if let (Some(cache), Some(key), Some(entries)) = (&self.cache, key, entries)
            && entries.len() <= MAX_CACHED_ENTRIES
            && let Err(e) = cache.put(key, &Cached { entries, segments })
        {
            eprintln!("plx: 히스토리 캐시에 쓰지 못했다: {e}");
        }
        HistoryEvent::Done { meta_ms, stats }
    }

    /// 커밋이 없는 저장소: meta만 보내고 끝낸다.
    fn empty(
        &self,
        id: u64,
        meta: HistoryMeta,
        started: Instant,
        on_event: &mut impl FnMut(HistoryEvent) -> bool,
    ) -> HistoryEvent {
        if self.requests.is_cancelled(id) {
            return HistoryEvent::Cancelled;
        }
        let meta_ms = ms(started);
        on_event(HistoryEvent::Meta(meta));
        HistoryEvent::Done {
            meta_ms,
            stats: HistoryStats {
                total_ms: ms(started),
                ..HistoryStats::default()
            },
        }
    }

    /// 띄운 git을 요청에 붙이고 읽기 시작한다. 이미 취소된 요청이면 `None`(git은 끝낸다).
    fn attach<S: EntrySource>(&self, id: u64, stream: S) -> Option<Reader> {
        self.requests
            .attach(id, stream.canceller())
            .then(|| Reader::start(stream))
    }

    fn replay(
        &self,
        id: u64,
        meta: HistoryMeta,
        cached: Cached,
        started: Instant,
        on_event: &mut impl FnMut(HistoryEvent) -> bool,
    ) -> HistoryEvent {
        if self.requests.is_cancelled(id) {
            return HistoryEvent::Cancelled;
        }
        let meta_ms = ms(started);
        if !on_event(HistoryEvent::Meta(meta)) {
            return HistoryEvent::Cancelled;
        }
        let entries = cached.entries.len();
        let mut stats = HistoryStats {
            segments: cached.segments,
            cached: true,
            entries,
            ..HistoryStats::default()
        };
        let mut rest = cached.entries.into_iter();
        let mut sent = 0;
        loop {
            let size = if sent == 0 { PAGE } else { REPLAY_CHUNK };
            let chunk: Vec<LogEntry> = rest.by_ref().take(size).collect();
            if chunk.is_empty() {
                break;
            }
            sent += chunk.len();
            if !on_event(HistoryEvent::Entries { entries: chunk }) {
                return HistoryEvent::Cancelled;
            }
            let now = ms(started);
            stats.first_entry_ms.get_or_insert(now);
            if sent >= PAGE {
                stats.first_page_ms.get_or_insert(now);
            }
        }
        stats.total_ms = ms(started);
        HistoryEvent::Done { meta_ms, stats }
    }
}

/// 한 이름의 `git log` 구간. 나눈 구간이면 `pending`(이미 받은 남은 커밋)부터 읽는다.
struct Segment {
    path: String,
    reader: Reader,
    pending: Vec<LogEntry>,
}

/// `entry`에서 `path`가 옛 이름으로 이어지는 곳을 `found`에 더한다. 파일이 rename으로 생긴 커밋(상태 A)이면 상태를
/// rename으로 바꾸고 (옛 이름, 첫 부모)를 돌려준다. merge는 파일이 없는 부모마다 옛 이름을 찾는다(곁가지에서 옮긴
/// 파일의 본줄기 이력). 옛 이름을 찾다 실패하면(얕은 클론, 부분 클론에서 blob이 없음) `incomplete`를 켜고 rename이
/// 아닌 것으로 본다.
fn boundaries(
    repo: &Repo,
    entry: &mut LogEntry,
    path: &str,
    found: &mut Vec<(String, Vec<String>)>,
    incomplete: &mut bool,
) -> Option<(String, String)> {
    if path == "." || entry.path != path {
        return None;
    }
    let id = ObjectId::from_hex(entry.sha.as_bytes()).ok()?;
    let mut add = |old: &str, parent: &str| match found.iter_mut().find(|(name, _)| name == old) {
        Some((_, starts)) if starts.iter().any(|s| s == parent) => {}
        Some((_, starts)) => starts.push(parent.to_owned()),
        None => found.push((old.to_owned(), vec![parent.to_owned()])),
    };
    let mut failed = |e: crate::repo::Error| {
        eprintln!("plx: rename 원본을 찾지 못했다 ({}): {e}", entry.sha);
        *incomplete = true;
    };
    if entry.parents.len() > 1 {
        for parent in &entry.parents {
            let Ok(parent_id) = ObjectId::from_hex(parent.as_bytes()) else {
                continue;
            };
            match repo.has_file(parent_id, path) {
                Ok(true) => {}
                Ok(false) => match repo.rename_source_from(id, parent_id, path) {
                    Ok(Some(old)) => add(&old, parent),
                    Ok(None) => {}
                    Err(e) => failed(e),
                },
                // 얕은 클론의 경계 너머 부모
                Err(e) => failed(e),
            }
        }
        return None;
    }
    if entry.status != Some(FileStatus::Added) || entry.parents.is_empty() {
        return None;
    }
    match repo.rename_source(id, path) {
        Ok(Some(old)) => {
            let parent = entry.parents[0].clone();
            add(&old, &parent);
            entry.status = Some(FileStatus::Renamed);
            entry.old_path = Some(old.clone());
            Some((old, parent))
        }
        Ok(None) => None,
        Err(e) => {
            failed(e);
            None
        }
    }
}

enum Message {
    Entries(Vec<LogEntry>),
    End(Result<(), LogError>),
    /// 읽는 스레드가 끝 메시지 없이 사라졌다.
    Gone,
}

/// git log 하나를 다른 스레드에서 읽는다. 채널이 작아서 읽는 쪽이 늦으면 git도 기다린다.
/// 버리면 git을 끝내고 스레드가 끝날 때까지 기다린다(프로세스를 거둔 뒤에 돌아온다).
struct Reader {
    rx: Option<Receiver<Message>>,
    canceller: Canceller,
    thread: Option<JoinHandle<()>>,
}

impl Reader {
    fn start<S: EntrySource>(mut stream: S) -> Reader {
        let canceller = stream.canceller();
        let (tx, rx) = mpsc::sync_channel(8);
        let thread = std::thread::spawn(move || {
            let mut batch = Vec::new();
            loop {
                match stream.read(&mut batch) {
                    Ok(more) => {
                        if !batch.is_empty()
                            && tx
                                .send(Message::Entries(std::mem::take(&mut batch)))
                                .is_err()
                        {
                            return; // 받는 쪽이 사라졌다. stream을 버리면 git을 끝낸다.
                        }
                        if !more {
                            let _ = tx.send(Message::End(stream.finish()));
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Message::End(Err(e)));
                        return;
                    }
                }
            }
        });
        Reader {
            rx: Some(rx),
            canceller,
            thread: Some(thread),
        }
    }

    fn rx(&self) -> &Receiver<Message> {
        self.rx.as_ref().expect("읽는 중")
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // 채널을 먼저 닫아야 보내려고 기다리던 스레드가 끝난다.
        self.canceller.cancel();
        drop(self.rx.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 보낼 커밋을 모아 첫 묶음은 바로, 그다음은 16ms마다(첫 [`PAGE`]개를 채우면 바로) 보낸다.
struct Sink {
    started: Instant,
    pending: Vec<LogEntry>,
    last_flush: Option<Instant>,
    sent: usize,
    stats: HistoryStats,
    /// 캐시에 넣을 전체 결과
    all: Option<Vec<LogEntry>>,
}

impl Sink {
    fn new(started: Instant, keep: bool) -> Sink {
        Sink {
            started,
            pending: Vec::new(),
            last_flush: None,
            sent: 0,
            stats: HistoryStats::default(),
            all: keep.then(Vec::new),
        }
    }

    fn push(&mut self, entry: LogEntry) {
        if let Some(all) = &mut self.all {
            all.push(entry.clone());
        }
        self.pending.push(entry);
    }

    /// 지금 보내야 하는가
    fn due(&self) -> bool {
        !self.pending.is_empty()
            && (self.last_flush.is_none()
                || (self.sent < PAGE && self.sent + self.pending.len() >= PAGE)
                || self.deadline().is_some_and(|d| d <= Instant::now()))
    }

    /// 쌓인 커밋을 늦어도 이때 보낸다. 쌓인 게 없으면 없다.
    fn deadline(&self) -> Option<Instant> {
        if self.pending.is_empty() {
            return None;
        }
        Some(
            self.last_flush
                .map_or_else(Instant::now, |t| t + FLUSH_INTERVAL),
        )
    }

    /// 쌓인 커밋을 보낸다. 받는 쪽이 사라졌으면 `false`.
    fn flush(&mut self, on_event: &mut impl FnMut(HistoryEvent) -> bool) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        let entries = std::mem::take(&mut self.pending);
        self.sent += entries.len();
        let ok = on_event(HistoryEvent::Entries { entries });
        let now = ms(self.started);
        self.stats.first_entry_ms.get_or_insert(now);
        if self.stats.first_page_ms.is_none() && self.sent >= PAGE {
            self.stats.first_page_ms = Some(now);
        }
        self.last_flush = Some(Instant::now());
        ok
    }

    fn finish(self) -> (Option<Vec<LogEntry>>, HistoryStats) {
        let mut stats = self.stats;
        stats.entries = self.sent;
        stats.total_ms = ms(self.started);
        (self.all, stats)
    }
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}
