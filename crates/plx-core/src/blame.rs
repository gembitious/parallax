//! `git blame --incremental` 스트리밍 파서와 실행기.
//!
//! 출력은 블록 단위이고, git이 원인을 찾는 순서(대개 최근 커밋부터)로 나온다. 줄 순서와 무관하다.
//! ```text
//! <sha> <orig_line> <final_line> <num_lines>
//! author …, author-mail …, author-time …, author-tz …,
//! committer …, committer-mail …, committer-time …, committer-tz …,
//! summary …, [boundary]                  ← 커밋이 처음 나올 때만
//! [previous <sha> <path>]                 ← 블록마다(그 커밋·경로의 부모)
//! filename <path>                         ← 블록의 끝
//! ```
//! git(builtin/blame.c)은 블록마다 출력을 flush하므로 첫 블록이 나오는 즉시 화면에 그릴 수 있다.

use std::collections::HashSet;
use std::fmt;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::git;

/// 블록이 가리키는 커밋. 같은 커밋은 처음 한 번만 나온다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameCommit {
    pub sha: String,
    pub author: String,
    pub author_mail: String,
    /// Unix 시각(초)
    pub author_time: i64,
    /// 예: `+0900`
    pub author_tz: String,
    pub committer: String,
    pub committer_mail: String,
    pub committer_time: i64,
    pub committer_tz: String,
    pub summary: String,
    /// 범위 경계(루트 커밋 등). 이보다 앞선 이력은 보지 않았다.
    pub boundary: bool,
}

/// 블록 원인 커밋의 부모와 그 부모에서의 경로(`,` 부모 리비전 blame에 쓴다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Previous {
    pub sha: String,
    pub path: String,
}

/// 연속된 줄 묶음과 그 원인 커밋.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameBlock {
    pub sha: String,
    /// 원인 커밋 버전의 파일에서 시작 줄(1부터)
    pub orig_line: u32,
    /// 지금 보는 파일에서 시작 줄(1부터)
    pub final_line: u32,
    pub num_lines: u32,
    /// 원인 커밋에서의 경로. rename을 따라갔다면 지금 경로와 다르다.
    pub path: String,
    pub previous: Option<Previous>,
}

/// 미커밋 줄에 git이 붙이는 sha.
pub fn is_uncommitted(sha: &str) -> bool {
    !sha.is_empty() && sha.bytes().all(|b| b == b'0')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Commit(BlameCommit),
    Block(BlameBlock),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "blame 출력을 해석할 수 없다: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

struct Pending {
    block: BlameBlock,
    /// 처음 나온 커밋이면 헤더를 모은다.
    commit: Option<BlameCommit>,
}

/// 바이트를 받는 대로 블록을 뱉는 파서. 청크 경계는 어디여도 된다.
#[derive(Default)]
pub struct IncrementalParser {
    partial: Vec<u8>,
    pending: Option<Pending>,
    seen: HashSet<String>,
}

impl IncrementalParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &[u8], out: &mut Vec<Event>) -> Result<(), ParseError> {
        let mut rest = chunk;
        while let Some(pos) = rest.iter().position(|&b| b == b'\n') {
            let (line, tail) = rest.split_at(pos);
            rest = &tail[1..];
            if self.partial.is_empty() {
                self.line(line, out)?;
            } else {
                let mut joined = std::mem::take(&mut self.partial);
                joined.extend_from_slice(line);
                self.line(&joined, out)?;
            }
        }
        self.partial.extend_from_slice(rest);
        Ok(())
    }

    /// 입력이 끝났을 때 부른다. 끝나지 않은 블록이 남았으면 오류다.
    pub fn finish(&mut self) -> Result<(), ParseError> {
        if !self.partial.is_empty() || self.pending.is_some() {
            return Err(ParseError("출력이 블록 중간에서 끝났다".to_owned()));
        }
        Ok(())
    }

    fn line(&mut self, line: &[u8], out: &mut Vec<Event>) -> Result<(), ParseError> {
        if self.pending.is_none() {
            return self.header(line);
        }
        let (key, value) = match line.iter().position(|&b| b == b' ') {
            Some(pos) => (&line[..pos], &line[pos + 1..]),
            None => (line, &line[line.len()..]),
        };
        if key == b"filename" {
            let mut done = self.pending.take().expect("pending");
            done.block.path = path_text(value);
            if let Some(commit) = done.commit.take() {
                self.seen.insert(commit.sha.clone());
                out.push(Event::Commit(commit));
            }
            out.push(Event::Block(done.block));
            return Ok(());
        }
        let pending = self.pending.as_mut().expect("pending");
        let text = || String::from_utf8_lossy(value).into_owned();
        let number = || {
            std::str::from_utf8(value)
                .ok()
                .and_then(|v| v.trim().parse::<i64>().ok())
                .ok_or_else(|| {
                    ParseError(format!("숫자가 아니다: {}", String::from_utf8_lossy(line)))
                })
        };
        match (key, pending.commit.as_mut()) {
            (b"previous", _) => {
                let (sha, path) = match value.iter().position(|&b| b == b' ') {
                    Some(pos) => (&value[..pos], &value[pos + 1..]),
                    None => {
                        return Err(ParseError(format!(
                            "previous 형식: {}",
                            String::from_utf8_lossy(line)
                        )));
                    }
                };
                pending.block.previous = Some(Previous {
                    sha: String::from_utf8_lossy(sha).into_owned(),
                    path: path_text(path),
                });
            }
            (b"author", Some(c)) => c.author = text(),
            (b"author-mail", Some(c)) => c.author_mail = text(),
            (b"author-time", Some(c)) => c.author_time = number()?,
            (b"author-tz", Some(c)) => c.author_tz = text(),
            (b"committer", Some(c)) => c.committer = text(),
            (b"committer-mail", Some(c)) => c.committer_mail = text(),
            (b"committer-time", Some(c)) => c.committer_time = number()?,
            (b"committer-tz", Some(c)) => c.committer_tz = text(),
            (b"summary", Some(c)) => c.summary = text(),
            (b"boundary", Some(c)) => c.boundary = true,
            // 모르는 키는 건너뛴다(git 새 버전이 필드를 더해도 깨지지 않게).
            _ => {}
        }
        Ok(())
    }

    fn header(&mut self, line: &[u8]) -> Result<(), ParseError> {
        let bad = || {
            ParseError(format!(
                "블록 머리 줄이 아니다: {}",
                String::from_utf8_lossy(line)
            ))
        };
        let text = std::str::from_utf8(line).map_err(|_| bad())?;
        let mut fields = text.split(' ');
        let (Some(sha), Some(orig), Some(fin), Some(num), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(bad());
        };
        if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let parse = |s: &str| s.parse::<u32>().map_err(|_| bad());
        let block = BlameBlock {
            sha: sha.to_owned(),
            orig_line: parse(orig)?,
            final_line: parse(fin)?,
            num_lines: parse(num)?,
            path: String::new(),
            previous: None,
        };
        let commit = (!self.seen.contains(sha)).then(|| BlameCommit {
            sha: sha.to_owned(),
            ..BlameCommit::default()
        });
        self.pending = Some(Pending { block, commit });
        Ok(())
    }
}

fn path_text(raw: &[u8]) -> String {
    String::from_utf8_lossy(&git::unquote_path(raw)).into_owned()
}

/// 화면에 보내는 묶음. 처음 나온 커밋을 블록보다 먼저 담는다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub commits: Vec<BlameCommit>,
    pub blocks: Vec<BlameBlock>,
}

impl Chunk {
    fn is_empty(&self) -> bool {
        self.commits.is_empty() && self.blocks.is_empty()
    }
}

/// 실행 시간(ms). 성능 HUD와 벤치에 쓴다.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub spawn_ms: f64,
    pub first_byte_ms: Option<f64>,
    pub first_block_ms: Option<f64>,
    pub total_ms: f64,
    pub blocks: usize,
    pub commits: usize,
    /// git을 띄우지 않고 캐시에서 읽었다.
    pub cached: bool,
}

/// 실행 중인 blame. 다른 스레드에서 [`Canceller`]로 멈출 수 있다.
/// 끝까지 읽지 않고 버리면 git을 끝내고 거둔다(상주 프로세스에 좀비가 쌓이지 않게).
pub struct Blame {
    child: Arc<Mutex<Child>>,
    cancelled: Arc<AtomicBool>,
    stdout: std::process::ChildStdout,
    stderr: Option<std::thread::JoinHandle<String>>,
    started: Instant,
    spawn_ms: f64,
}

impl Drop for Blame {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

#[derive(Clone)]
pub struct Canceller {
    child: Arc<Mutex<Child>>,
    cancelled: Arc<AtomicBool>,
}

impl Canceller {
    /// git을 끝낸다. 읽던 쪽은 [`BlameError::Cancelled`]로 끝난다.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _ = self.child.lock().unwrap().kill();
    }
}

#[derive(Debug)]
pub enum BlameError {
    Cancelled,
    Io(io::Error),
    Parse(ParseError),
    /// git이 실패했다(경로가 이력에 없음 등). 표준 오류를 담는다.
    Git(String),
}

impl fmt::Display for BlameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlameError::Cancelled => f.write_str("취소했다"),
            BlameError::Io(e) => write!(f, "git blame 실행 실패: {e}"),
            BlameError::Parse(e) => e.fmt(f),
            BlameError::Git(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for BlameError {}

/// 첫 블록 뒤로는 이 간격마다 모아서 보낸다(화면 갱신 횟수를 줄인다).
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);

impl Blame {
    /// `repo_dir`에서 `path`(저장소 기준, `/` 구분)를 blame한다. `rev`가 없으면 작업트리 파일이다.
    pub fn spawn(repo_dir: &Path, path: &str, rev: Option<&str>) -> io::Result<Blame> {
        let mut command = git::command(repo_dir);
        command.args(["blame", "--incremental"]);
        if let Some(rev) = rev {
            command.arg(rev);
        }
        command.arg("--").arg(path);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let started = Instant::now();
        let mut child = command.spawn()?;
        let spawn_ms = started.elapsed().as_secs_f64() * 1000.0;
        let stdout = child.stdout.take().expect("stdout");
        let mut stderr = child.stderr.take().expect("stderr");
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        Ok(Blame {
            child: Arc::new(Mutex::new(child)),
            cancelled: Arc::new(AtomicBool::new(false)),
            stdout,
            stderr: Some(stderr),
            started,
            spawn_ms,
        })
    }

    pub fn canceller(&self) -> Canceller {
        Canceller {
            child: self.child.clone(),
            cancelled: self.cancelled.clone(),
        }
    }

    /// 끝까지 읽으며 묶음을 넘긴다. 첫 블록은 바로, 그다음부터는 16ms마다 모아서 넘긴다.
    pub fn stream(mut self, mut on_chunk: impl FnMut(Chunk)) -> Result<Stats, BlameError> {
        let ms = |since: Instant| since.elapsed().as_secs_f64() * 1000.0;
        let mut stats = Stats {
            spawn_ms: self.spawn_ms,
            ..Stats::default()
        };
        let mut parser = IncrementalParser::new();
        let mut events = Vec::new();
        let mut chunk = Chunk::default();
        let mut last_flush = Instant::now();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match self.stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(self.fail(BlameError::Io(e))),
            };
            stats.first_byte_ms.get_or_insert_with(|| ms(self.started));
            if let Err(e) = parser.push(&buf[..n], &mut events) {
                return Err(self.fail(BlameError::Parse(e)));
            }
            for event in events.drain(..) {
                match event {
                    Event::Commit(commit) => {
                        stats.commits += 1;
                        chunk.commits.push(commit);
                    }
                    Event::Block(block) => {
                        stats.blocks += 1;
                        chunk.blocks.push(block);
                    }
                }
            }
            let first = stats.first_block_ms.is_none() && !chunk.blocks.is_empty();
            if first {
                stats.first_block_ms = Some(ms(self.started));
            }
            if !chunk.is_empty() && (first || last_flush.elapsed() >= FLUSH_INTERVAL) {
                on_chunk(std::mem::take(&mut chunk));
                last_flush = Instant::now();
            }
        }
        let status = self.child.lock().unwrap().wait().map_err(BlameError::Io)?;
        let stderr = self
            .stderr
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        if self.cancelled.load(Ordering::SeqCst) {
            return Err(BlameError::Cancelled);
        }
        if !status.success() {
            return Err(BlameError::Git(git_error(&stderr, status)));
        }
        parser.finish().map_err(BlameError::Parse)?;
        if !chunk.is_empty() {
            on_chunk(chunk);
        }
        stats.total_ms = ms(self.started);
        Ok(stats)
    }

    fn fail(&self, error: BlameError) -> BlameError {
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        if self.cancelled.load(Ordering::SeqCst) {
            BlameError::Cancelled
        } else {
            error
        }
    }
}

fn git_error(stderr: &str, status: std::process::ExitStatus) -> String {
    let message = stderr
        .lines()
        .map(|l| l.trim_start_matches("fatal: ").trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if message.is_empty() {
        format!("git blame 실패({status})")
    } else {
        format!("git blame 실패: {message}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
1111111111111111111111111111111111111111 1 1 2
author 김개발
author-mail <kim@example.com>
author-time 1700000000
author-tz +0900
committer Fixture Committer
committer-mail <c@example.com>
committer-time 1700000100
committer-tz +0900
summary 첫 커밋
boundary
filename \"a\\\"b.rs\"
2222222222222222222222222222222222222222 5 3 1
author B
author-mail <b@example.com>
author-time 1700003600
author-tz +0000
committer B
committer-mail <b@example.com>
committer-time 1700003600
committer-tz +0000
summary second
previous 1111111111111111111111111111111111111111 old/a.rs
filename a.rs
1111111111111111111111111111111111111111 3 4 1
filename a.rs
";

    fn parse_all(chunks: &[&[u8]]) -> Vec<Event> {
        let mut parser = IncrementalParser::new();
        let mut out = Vec::new();
        for chunk in chunks {
            parser.push(chunk, &mut out).unwrap();
        }
        parser.finish().unwrap();
        out
    }

    #[test]
    fn parses_commits_once_and_blocks() {
        let events = parse_all(&[SAMPLE.as_bytes()]);
        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                Event::Commit(_) => "commit",
                Event::Block(_) => "block",
            })
            .collect();
        assert_eq!(kinds, ["commit", "block", "commit", "block", "block"]);
        let Event::Commit(first) = &events[0] else {
            unreachable!()
        };
        assert_eq!(first.author, "김개발");
        assert_eq!(first.author_time, 1_700_000_000);
        assert!(first.boundary);
        let Event::Block(first_block) = &events[1] else {
            unreachable!()
        };
        assert_eq!(first_block.path, "a\"b.rs");
        assert_eq!(
            (
                first_block.orig_line,
                first_block.final_line,
                first_block.num_lines
            ),
            (1, 1, 2)
        );
        let Event::Block(second) = &events[3] else {
            unreachable!()
        };
        assert_eq!(
            second.previous,
            Some(Previous {
                sha: "1111111111111111111111111111111111111111".into(),
                path: "old/a.rs".into()
            })
        );
        let Event::Block(third) = &events[4] else {
            unreachable!()
        };
        assert_eq!(third.sha, "1111111111111111111111111111111111111111");
        assert_eq!(third.previous, None);
    }

    #[test]
    fn chunk_boundaries_do_not_matter() {
        let whole = parse_all(&[SAMPLE.as_bytes()]);
        let bytes: Vec<&[u8]> = SAMPLE.as_bytes().chunks(1).collect();
        assert_eq!(parse_all(&bytes), whole);
        let bytes: Vec<&[u8]> = SAMPLE.as_bytes().chunks(7).collect();
        assert_eq!(parse_all(&bytes), whole);
    }

    #[test]
    fn truncated_output_is_an_error() {
        let mut parser = IncrementalParser::new();
        let mut out = Vec::new();
        parser.push(&SAMPLE.as_bytes()[..100], &mut out).unwrap();
        assert!(parser.finish().is_err());
    }

    #[test]
    fn rejects_garbage_header() {
        let mut parser = IncrementalParser::new();
        let mut out = Vec::new();
        assert!(parser.push(b"not a header\n", &mut out).is_err());
    }

    #[test]
    fn uncommitted_sha() {
        assert!(is_uncommitted("0000000000000000000000000000000000000000"));
        assert!(!is_uncommitted("1000000000000000000000000000000000000000"));
    }

    /// 읽지 않고 버린 blame의 git 프로세스를 거둔다. 거두지 않으면 waitpid가 아직 기다릴 자식을 찾는다.
    #[cfg(unix)]
    #[test]
    fn dropping_reaps_the_child() {
        // 이 crate 디렉터리(저장소가 아니어도 된다: git이 실패해도 프로세스는 뜬다).
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        for cancel_first in [false, true] {
            let blame = Blame::spawn(dir, "Cargo.toml", None).unwrap();
            let pid = blame.child.lock().unwrap().id() as libc::pid_t;
            if cancel_first {
                blame.canceller().cancel();
            }
            drop(blame);
            // SAFETY: 이 프로세스의 자식 pid에 대한 waitpid. WNOHANG이라 막히지 않는다.
            let r = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
            let errno = io::Error::last_os_error().raw_os_error();
            assert_eq!(
                (r, errno),
                (-1, Some(libc::ECHILD)),
                "이미 거둔 자식이어야 한다"
            );
        }
    }
}
