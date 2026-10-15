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
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gix::ObjectId;
use serde::{Deserialize, Serialize};

use crate::cache::{CacheKey, DiskCache};
use crate::commit::FileStatus;
use crate::git::Canceller;
use crate::log::{LogEntry, LogError, LogStream};
use crate::repo::{Repo, RepoCache};
use crate::requests::Requests;

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

enum Plan {
    Git {
        repo: Arc<Repo>,
        stream: Box<LogStream>,
        key: Option<CacheKey>,
    },
    Cached(Cached),
    /// 커밋이 없는 저장소
    Empty,
}

fn start(
    repos: &RepoCache,
    cache: Option<&DiskCache>,
    path: &Path,
    rev: Option<&str>,
) -> Result<(HistoryMeta, Plan), String> {
    let repo = repos.for_path(path).map_err(|e| e.to_string())?;
    let mut rel = repo.relative_path(path).map_err(|e| e.to_string())?;
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
    };
    let Some(start) = start else {
        return Ok((meta, Plan::Empty));
    };
    let key = cache.map(|_| CacheKey::history(repo.workdir(), &start.to_string(), &rel));
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
        path: &Path,
        rev: Option<&str>,
        mut on_event: impl FnMut(HistoryEvent) -> bool,
    ) {
        let end = self.drive(id, repos, path, rev, &mut on_event);
        self.requests.finish(id);
        on_event(end);
    }

    fn drive(
        &self,
        id: u64,
        repos: &RepoCache,
        path: &Path,
        rev: Option<&str>,
        on_event: &mut impl FnMut(HistoryEvent) -> bool,
    ) -> HistoryEvent {
        let started = Instant::now();
        if self.requests.is_cancelled(id) {
            return HistoryEvent::Cancelled;
        }
        let (meta, plan) = match start(repos, self.cache.as_deref(), path, rev) {
            Ok(started) => started,
            Err(message) => return HistoryEvent::Error { message },
        };
        let path = meta.path.clone();
        let (repo, stream, key) = match plan {
            Plan::Git { repo, stream, key } => (repo, stream, key),
            Plan::Cached(cached) => return self.replay(id, meta, cached, started, on_event),
            Plan::Empty => {
                if self.requests.is_cancelled(id) {
                    return HistoryEvent::Cancelled;
                }
                let meta_ms = ms(started);
                on_event(HistoryEvent::Meta(meta));
                return HistoryEvent::Done {
                    meta_ms,
                    stats: HistoryStats {
                        total_ms: ms(started),
                        ..HistoryStats::default()
                    },
                };
            }
        };
        let Some(mut reader) = self.attach(id, *stream) else {
            return HistoryEvent::Cancelled;
        };
        let meta_ms = ms(started);
        if !on_event(HistoryEvent::Meta(meta)) {
            self.requests.cancel(id);
        }

        let mut sink = Sink::new(started, key.is_some());
        let mut received = 0;
        let mut seen = HashSet::new();
        let mut path = path;
        let mut segments = 1;
        loop {
            // (첫 부모, 옛 이름): 이 구간에서 파일이 rename으로 생긴 커밋들
            let mut boundaries: Vec<(String, String)> = Vec::new();
            // 처음 찾은 경계에서 미리 띄운 다음 구간
            let mut next: Option<(String, String, Reader)> = None;
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
                let entries = match message {
                    Message::Entries(entries) => entries,
                    Message::End(Ok(())) => break,
                    Message::End(Err(LogError::Cancelled)) => return HistoryEvent::Cancelled,
                    Message::End(Err(e)) => {
                        return HistoryEvent::Error {
                            message: e.to_string(),
                        };
                    }
                    Message::Gone => {
                        return HistoryEvent::Error {
                            message: "git log를 읽는 스레드가 끝났다".into(),
                        };
                    }
                };
                received += entries.len();
                if received >= PAGE {
                    sink.stats.git_page_ms.get_or_insert_with(|| ms(started));
                }
                for mut entry in entries {
                    if let Some(old) = rename_source(&repo, &entry, &path) {
                        let parent = entry.parents[0].clone();
                        entry.status = Some(FileStatus::Renamed);
                        entry.old_path = Some(old.clone());
                        if next.is_none() {
                            let stream = match LogStream::spawn(
                                repo.workdir(),
                                std::slice::from_ref(&parent),
                                &old,
                            ) {
                                Ok(stream) => stream,
                                Err(e) => {
                                    return HistoryEvent::Error {
                                        message: format!("git log 실행 실패: {e}"),
                                    };
                                }
                            };
                            let Some(reader) = self.attach(id, stream) else {
                                return HistoryEvent::Cancelled;
                            };
                            next = Some((parent.clone(), old.clone(), reader));
                        }
                        boundaries.push((parent, old));
                    }
                    if seen.insert(entry.sha.clone()) {
                        sink.push(entry);
                    }
                }
                if sink.due() && !sink.flush(on_event) {
                    self.requests.cancel(id);
                }
            }

            // 다음 구간: 처음 찾은 옛 이름으로, 같은 이름에서 생긴 모든 경계의 부모부터 거슬러 올라간다.
            let Some(old) = boundaries.first().map(|(_, old)| old.clone()) else {
                break;
            };
            if segments == MAX_SEGMENTS {
                break;
            }
            let mut starts: Vec<String> = Vec::new();
            for (parent, name) in &boundaries {
                if *name == old && !starts.contains(parent) {
                    starts.push(parent.clone());
                }
            }
            reader = match next {
                Some((parent, name, ready))
                    if name == old && starts.len() == 1 && starts[0] == parent =>
                {
                    ready
                }
                _ => match LogStream::spawn(repo.workdir(), &starts, &old) {
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
            path = old;
            segments += 1;
        }
        sink.stats.git_page_ms.get_or_insert_with(|| ms(started));
        if !sink.flush(on_event) {
            return HistoryEvent::Cancelled;
        }
        let (entries, mut stats) = sink.finish();
        stats.segments = segments;
        if let (Some(cache), Some(key), Some(entries)) = (&self.cache, &key, entries)
            && entries.len() <= MAX_CACHED_ENTRIES
            && let Err(e) = cache.put(key, &Cached { entries, segments })
        {
            eprintln!("plx: 히스토리 캐시에 쓰지 못했다: {e}");
        }
        HistoryEvent::Done { meta_ms, stats }
    }

    /// 띄운 git을 요청에 붙이고 읽기 시작한다. 이미 취소된 요청이면 `None`(git은 끝낸다).
    fn attach(&self, id: u64, stream: LogStream) -> Option<Reader> {
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

/// `entry`가 `path`를 rename으로 만든 커밋이면 원래 경로. 찾지 못하면(얕은 클론, 부분 클론에서 blob이
/// 없는 경우 등) rename이 아닌 것으로 본다.
fn rename_source(repo: &Repo, entry: &LogEntry, path: &str) -> Option<String> {
    if entry.status != Some(FileStatus::Added) || entry.path != path || entry.parents.is_empty() {
        return None;
    }
    let id = ObjectId::from_hex(entry.sha.as_bytes()).ok()?;
    match repo.rename_source(id, path) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("plx: rename 원본을 찾지 못했다 ({}): {e}", entry.sha);
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
    fn start(mut stream: LogStream) -> Reader {
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
