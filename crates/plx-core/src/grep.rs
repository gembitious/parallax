//! 저장소 전체 검색(M7-3): `git grep`을 흘려보내 파일마다 맞은 줄을 보낸다.
//!
//! 작업트리(추적하는 파일, 인덱스를 따른다) 또는 리비전 하나의 트리를 찾는다. git이 맞은 자리를 알려 주도록
//! `--color=always`로 맞은 글자만 칠하게 하고(다른 칸의 색은 `normal`로 끈다) 그 표시를 걷어 내며 자리를 센다.
//! 화면의 정규식(JavaScript)과 git의 정규식(POSIX ERE)이 달라 화면에서 다시 맞추지 않는다.
//!
//! 출력은 `--null -n`: `<경로>\0<줄>\0<줄 내용>\n`. 리비전을 찾으면 경로 앞에 `<리비전>:`이 붙는다.
//! 저장소를 바꾸지 않는다(`git grep`은 인덱스를 고쳐 쓰지 않고, [`crate::git::command`]가 선택적 잠금을 끈다).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::git::{self, StreamError, Streaming};
use crate::repo::RepoCache;
use crate::requests::{self, Requests};

/// 맞은 줄을 이만큼까지만 보낸다(넘으면 git을 끝내고 `truncated`).
pub const MAX_MATCHES: usize = 20_000;
/// 줄 내용을 이만큼(글자)만 보낸다. 맞은 자리가 뒤에 있으면 그 앞을 잘라 맞은 자리가 보이게 한다.
pub const MAX_LINE_CHARS: usize = 400;
/// 잘린 줄에서 첫 맞은 자리 앞에 남기는 글자 수
const LEAD_CHARS: usize = 40;
/// 받은 결과를 화면에 넘기는 간격(첫 결과는 바로)
const FLUSH: Duration = Duration::from_millis(30);

/// git이 맞은 글자를 감싸는 표시(`color.grep.matchSelected=reverse`)
const MATCH_START: &[u8] = b"\x1b[7m";
const MATCH_END: &[u8] = b"\x1b[m";

/// 찾을 것
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepQuery {
    pub pattern: String,
    /// 정규식(`-E`). 아니면 글자 그대로(`-F`)
    #[serde(default)]
    pub regex: bool,
    /// 대소문자를 가린다. 아니면 `-i`
    #[serde(default)]
    pub case_sensitive: bool,
    /// 낱말로만(`-w`)
    #[serde(default)]
    pub word: bool,
    /// 찾을 리비전. 없으면 작업트리
    #[serde(default)]
    pub rev: Option<String>,
    /// 찾을 경로(git pathspec: `src/`, `*.ts`, 앞에 `!`면 뺀다). 비면 저장소 전체
    #[serde(default)]
    pub paths: Vec<String>,
}

/// 맞은 줄 하나
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepLine {
    /// 1부터
    pub line: u32,
    /// 줄 내용(표시를 걷은 것, 길면 잘린 것)
    pub text: String,
    /// `text`에서 맞은 자리 `[시작, 끝)`, UTF-16 단위(화면이 바로 자른다)
    pub ranges: Vec<[u32; 2]>,
    /// 앞을 잘랐다(`text`가 줄 처음이 아니다)
    pub cut_start: bool,
    /// 뒤를 잘랐다
    pub cut_end: bool,
}

/// 한 파일의 맞은 줄들
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepFile {
    /// 저장소 기준 경로
    pub path: String,
    pub lines: Vec<GrepLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GrepEvent {
    /// 찾기 시작했다. `rev`는 푼 커밋(작업트리면 없다), `repo`는 작업트리 경로
    Meta {
        repo: String,
        rev: Option<String>,
    },
    /// 새로 맞은 줄들. 같은 파일이 두 묶음에 걸치면 뒤 묶음에 이어진다(경로 순으로 온다).
    Files {
        files: Vec<GrepFile>,
    },
    Done {
        matches: usize,
        files: usize,
        /// [`MAX_MATCHES`]에서 멈췄다
        truncated: bool,
        /// 첫 결과까지(없으면 끝까지)
        first_ms: f64,
        total_ms: f64,
    },
    Error {
        message: String,
    },
    Cancelled,
}

impl GrepEvent {
    pub fn is_end(&self) -> bool {
        matches!(
            self,
            GrepEvent::Done { .. } | GrepEvent::Error { .. } | GrepEvent::Cancelled
        )
    }
}

/// 해석할 수 없는 출력
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "git grep 출력을 읽지 못했다: {}", self.0)
    }
}

/// `git grep --null -n` 출력을 줄 단위로 읽는다(청크는 아무 데서나 끊겨 온다).
#[derive(Default)]
pub struct GrepParser {
    pending: Vec<u8>,
    /// 리비전을 찾을 때 경로 앞의 `<리비전>:`
    prefix: Vec<u8>,
}

/// 해석한 줄 하나(경로는 바이트 그대로)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub path: Vec<u8>,
    pub line: GrepLine,
}

impl GrepParser {
    /// `rev`는 git에 준 리비전 그대로다(경로 앞에 붙어 나온다).
    pub fn new(rev: Option<&str>) -> Self {
        GrepParser {
            pending: Vec::new(),
            prefix: rev
                .map(|r| format!("{r}:").into_bytes())
                .unwrap_or_default(),
        }
    }

    /// `chunk`에서 끝난 줄을 `out`에 더한다.
    pub fn push(&mut self, chunk: &[u8], out: &mut Vec<Record>) -> Result<(), ParseError> {
        self.pending.extend_from_slice(chunk);
        let mut start = 0;
        while let Some(at) = self.pending[start..].iter().position(|&b| b == b'\n') {
            let end = start + at;
            let record = parse_record(&self.pending[start..end], &self.prefix)?;
            out.push(record);
            start = end + 1;
        }
        self.pending.drain(..start);
        Ok(())
    }

    /// 출력이 끝났다. 줄바꿈 없이 끝난 마지막 줄을 읽는다.
    pub fn finish(&mut self, out: &mut Vec<Record>) -> Result<(), ParseError> {
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            out.push(parse_record(&rest, &self.prefix)?);
        }
        Ok(())
    }
}

/// `<경로>\0<줄>\0<내용>` 하나
fn parse_record(raw: &[u8], prefix: &[u8]) -> Result<Record, ParseError> {
    let mut parts = raw.splitn(3, |&b| b == 0);
    let path = parts.next().unwrap_or_default();
    let (Some(number), Some(text)) = (parts.next(), parts.next()) else {
        return Err(ParseError(format!(
            "칸이 모자란 줄: {}",
            String::from_utf8_lossy(&raw[..raw.len().min(80)])
        )));
    };
    let line = std::str::from_utf8(number)
        .ok()
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|&n| n > 0)
        .ok_or_else(|| {
            ParseError(format!(
                "줄 번호가 아니다: {}",
                String::from_utf8_lossy(number)
            ))
        })?;
    let path = path.strip_prefix(prefix).unwrap_or(path);
    let text = text.strip_suffix(b"\r").unwrap_or(text);
    Ok(Record {
        path: path.to_vec(),
        line: highlight(line, text),
    })
}

/// 맞은 표시를 걷어 낸 내용과 맞은 자리(UTF-16). 길면 [`MAX_LINE_CHARS`]로 자른다.
pub fn highlight(line: u32, raw: &[u8]) -> GrepLine {
    // 표시를 걷으며 맞은 자리를 바이트로 센다.
    let mut bytes = Vec::with_capacity(raw.len());
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut open: Option<usize> = None;
    let mut i = 0;
    while i < raw.len() {
        if raw[i..].starts_with(MATCH_START) {
            open.get_or_insert(bytes.len());
            i += MATCH_START.len();
        } else if raw[i..].starts_with(MATCH_END) {
            if let Some(start) = open.take()
                && start < bytes.len()
            {
                spans.push((start, bytes.len()));
            }
            i += MATCH_END.len();
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    if let Some(start) = open
        && start < bytes.len()
    {
        spans.push((start, bytes.len()));
    }
    // 바이트 자리 → 글자 자리. 깨진 바이트 덩어리는 바꿈 글자 하나로 센다(from_utf8_lossy와 같다).
    let text = String::from_utf8_lossy(&bytes);
    let chars: Vec<char> = text.chars().collect();
    let mut char_at = vec![0usize; bytes.len() + 1];
    let mut c = 0;
    let mut b = 0;
    for chunk in bytes.utf8_chunks() {
        let valid = chunk.valid();
        for (k, slot) in char_at[b..b + valid.len()].iter_mut().enumerate() {
            if valid.is_char_boundary(k) {
                c += usize::from(k > 0);
            }
            *slot = c;
        }
        if !valid.is_empty() {
            c += 1;
        }
        b += valid.len();
        let invalid = chunk.invalid().len();
        char_at[b..b + invalid].fill(c);
        c += usize::from(invalid > 0);
        b += invalid;
    }
    char_at[bytes.len()] = c;
    // 끝은 마지막 바이트가 든 글자의 뒤(글자 가운데서 끝난 자리도 그 글자를 칠한다). 겹친 자리는 합친다(붙은 자리는 따로 둔다: `값값`은 맞은 것 둘).
    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for &(start, end) in &spans {
        let range = (char_at[start], char_at[end - 1] + 1);
        match ranges.last_mut() {
            Some(last) if range.0 < last.1 => last.1 = last.1.max(range.1),
            _ => ranges.push(range),
        }
    }
    // 자르기(글자 단위)
    let total = chars.len();
    let mut from = 0;
    if total > MAX_LINE_CHARS
        && let Some(&(first, _)) = ranges.first()
        && first > LEAD_CHARS
    {
        from = (first - LEAD_CHARS).min(total.saturating_sub(MAX_LINE_CHARS));
    }
    let to = (from + MAX_LINE_CHARS).min(total);
    let shown: String = chars[from..to].iter().collect();
    ranges.retain(|&(s, e)| e > from && s < to);
    // 글자 → UTF-16
    let mut utf16_at = Vec::with_capacity(to - from + 1);
    let mut u = 0u32;
    for c in &chars[from..to] {
        utf16_at.push(u);
        u += c.len_utf16() as u32;
    }
    utf16_at.push(u);
    let ranges = ranges
        .into_iter()
        .map(|(s, e)| [utf16_at[s.max(from) - from], utf16_at[e.min(to) - from]])
        .collect();
    GrepLine {
        line,
        text: shown,
        ranges,
        cut_start: from > 0,
        cut_end: to < total,
    }
}

/// `git grep` 인자(패턴과 경로 앞까지). `rev`는 푼 커밋이다.
pub fn grep_args(query: &GrepQuery, rev: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-c",
        "color.grep.filename=normal",
        "-c",
        "color.grep.lineNumber=normal",
        "-c",
        "color.grep.column=normal",
        "-c",
        "color.grep.separator=normal",
        "-c",
        "color.grep.function=normal",
        "-c",
        "color.grep.context=normal",
        "-c",
        "color.grep.selected=normal",
        "-c",
        "color.grep.matchContext=normal",
        "-c",
        "color.grep.matchSelected=reverse",
        // 사용자 설정이 결과 모양을 바꾸지 않게(줄 번호, 정규식 종류, 하위 모듈)
        "-c",
        "grep.lineNumber=true",
        "-c",
        "grep.column=false",
        "-c",
        "grep.patternType=default",
        "-c",
        "grep.extendedRegexp=false",
        "-c",
        "grep.fullName=true",
        "-c",
        "submodule.recurse=false",
        "grep",
        "--color=always",
        "--null",
        "-n",
        "-I",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(if query.regex { "-E" } else { "-F" }.to_owned());
    if !query.case_sensitive {
        args.push("-i".to_owned());
    }
    if query.word {
        args.push("-w".to_owned());
    }
    args.push("-e".to_owned());
    args.push(query.pattern.clone());
    if let Some(rev) = rev {
        args.push(rev.to_owned());
    }
    args.push("--".to_owned());
    for path in &query.paths {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        // `!x`는 빼기(git의 `:!x`)
        args.push(match path.strip_prefix('!') {
            Some(rest) => format!(":(exclude){rest}"),
            None => path.to_owned(),
        });
    }
    args
}

/// 진행 중인 검색들. 앱 상태로 하나 둔다.
#[derive(Default)]
pub struct GrepSessions {
    requests: Requests,
}

impl GrepSessions {
    pub fn new() -> Self {
        Self::default()
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

    /// `id` 검색을 끝까지 처리한다. `repo`는 저장소 안의 아무 경로다. 끝 이벤트는 반드시 하나 보낸다.
    /// `on_event`가 `false`를 돌려주면(받는 쪽이 사라졌다) 멈춘다.
    pub fn run(
        &self,
        id: u64,
        repos: &RepoCache,
        repo: &std::path::Path,
        query: &GrepQuery,
        mut on_event: impl FnMut(GrepEvent) -> bool,
    ) {
        let end = requests::catch(
            || self.drive(id, repos, repo, query, &mut on_event),
            |message| GrepEvent::Error { message },
        );
        self.requests.finish(id);
        on_event(end);
    }

    fn drive(
        &self,
        id: u64,
        repos: &RepoCache,
        repo: &std::path::Path,
        query: &GrepQuery,
        on_event: &mut impl FnMut(GrepEvent) -> bool,
    ) -> GrepEvent {
        let started = Instant::now();
        if query.pattern.is_empty() {
            return GrepEvent::Error {
                message: "찾을 글을 넣는다".to_owned(),
            };
        }
        let repo = match repos.for_path(repo) {
            Ok(repo) => repo,
            Err(e) => return GrepEvent::Error { message: e.0 },
        };
        let rev = match query
            .rev
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
        {
            Some(rev) => match repo.resolve_commit(rev) {
                Ok(id) => Some(id.to_string()),
                Err(e) => return GrepEvent::Error { message: e.0 },
            },
            None => None,
        };
        let workdir = repo.workdir().to_path_buf();
        if !on_event(GrepEvent::Meta {
            repo: workdir.to_string_lossy().into_owned(),
            rev: rev.clone(),
        }) {
            return GrepEvent::Cancelled;
        }
        let mut command = git::command(&workdir);
        command.args(grep_args(query, rev.as_deref()));
        let mut stream = match Streaming::spawn(command, "git grep") {
            Ok(stream) => stream,
            Err(e) => {
                return GrepEvent::Error {
                    message: format!("git 실행 실패: {e}"),
                };
            }
        };
        if !self.requests.attach(id, stream.canceller()) {
            return GrepEvent::Cancelled;
        }
        let mut parser = GrepParser::new(rev.as_deref());
        let mut batch = Batch::default();
        let mut first_ms: Option<f64> = None;
        let mut buf = vec![0u8; 64 * 1024];
        let mut truncated = false;
        let mut records = Vec::new();
        let mut last_flush = Instant::now() - FLUSH;
        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(StreamError::Cancelled) => return GrepEvent::Cancelled,
                Err(e) => {
                    return GrepEvent::Error {
                        message: e.to_string(),
                    };
                }
            };
            if let Err(e) = parser.push(&buf[..n], &mut records) {
                return if stream.abort() {
                    GrepEvent::Cancelled
                } else {
                    GrepEvent::Error {
                        message: e.to_string(),
                    }
                };
            }
            for record in records.drain(..) {
                if batch.matches >= MAX_MATCHES {
                    truncated = true;
                    break;
                }
                batch.add(record);
            }
            if truncated {
                break;
            }
            if batch.has_new() && last_flush.elapsed() >= FLUSH {
                first_ms.get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.0);
                if !on_event(GrepEvent::Files {
                    files: batch.take(),
                }) {
                    stream.abort();
                    return GrepEvent::Cancelled;
                }
                last_flush = Instant::now();
            }
        }
        if truncated {
            // 다 받지 않았다: git을 끝낸다(종료 코드는 보지 않는다).
            if stream.abort() {
                return GrepEvent::Cancelled;
            }
        } else {
            if let Err(e) = parser.finish(&mut records) {
                return GrepEvent::Error {
                    message: e.to_string(),
                };
            }
            for record in records.drain(..) {
                if batch.matches >= MAX_MATCHES {
                    truncated = true;
                    break;
                }
                batch.add(record);
            }
            match stream.finish() {
                Ok(()) => {}
                Err(StreamError::Cancelled) => return GrepEvent::Cancelled,
                // 하나도 맞지 않으면 git grep은 1로 끝난다(표준 오류는 비어 있다).
                Err(StreamError::Git(message)) if message.ends_with("실패(exit status: 1)") => {}
                Err(e) => {
                    return GrepEvent::Error {
                        message: e.to_string(),
                    };
                }
            }
        }
        if batch.has_new() {
            first_ms.get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.0);
            if !on_event(GrepEvent::Files {
                files: batch.take(),
            }) {
                return GrepEvent::Cancelled;
            }
        }
        let total_ms = started.elapsed().as_secs_f64() * 1000.0;
        GrepEvent::Done {
            matches: batch.matches,
            files: batch.files,
            truncated,
            first_ms: first_ms.unwrap_or(total_ms),
            total_ms,
        }
    }
}

/// 아직 보내지 않은 결과(파일 순서대로)
#[derive(Default)]
struct Batch {
    pending: BTreeMap<usize, GrepFile>,
    order: usize,
    last_path: Option<Vec<u8>>,
    matches: usize,
    files: usize,
}

impl Batch {
    fn add(&mut self, record: Record) {
        if self.last_path.as_deref() != Some(&record.path[..]) {
            self.order += 1;
            self.files += 1;
            self.last_path = Some(record.path.clone());
        }
        self.matches += 1;
        let path = String::from_utf8_lossy(&record.path).into_owned();
        self.pending
            .entry(self.order)
            .or_insert_with(|| GrepFile {
                path,
                lines: Vec::new(),
            })
            .lines
            .push(record.line);
    }

    fn has_new(&self) -> bool {
        !self.pending.is_empty()
    }

    fn take(&mut self) -> Vec<GrepFile> {
        std::mem::take(&mut self.pending).into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(raw: &str) -> GrepLine {
        highlight(1, raw.as_bytes())
    }

    #[test]
    fn highlight_strips_marks_and_counts_utf16() {
        let line = h("let \x1b[7m한글\x1b[m = \x1b[7m😀x\x1b[m;");
        assert_eq!(line.text, "let 한글 = 😀x;");
        assert_eq!(line.ranges, vec![[4, 6], [9, 12]]);
        assert!(!line.cut_start && !line.cut_end);
    }

    #[test]
    fn long_lines_keep_the_first_match_in_view() {
        let raw = format!(
            "{}\x1b[7mneedle\x1b[m{}",
            "a".repeat(1000),
            "b".repeat(1000)
        );
        let line = h(&raw);
        assert!(line.cut_start && line.cut_end);
        assert_eq!(line.text.chars().count(), MAX_LINE_CHARS);
        let [s, e] = line.ranges[0];
        assert_eq!(&line.text[s as usize..e as usize], "needle");
        assert_eq!(s as usize, LEAD_CHARS);
    }

    #[test]
    fn broken_utf8_counts_like_lossy_decoding() {
        // 바이트 둘은 따로 깨진 덩어리 둘이고, 셋째는 덜 끝난 한 글자(바꿈 글자 하나)다.
        let mut raw = b"\xff\xfe \xed\x95 ".to_vec();
        raw.extend_from_slice(b"\x1b[7mok\x1b[m");
        let line = highlight(3, &raw);
        assert_eq!(line.text, "\u{fffd}\u{fffd} \u{fffd} ok");
        assert_eq!(line.ranges, vec![[5, 7]]);
    }

    #[test]
    fn parser_reads_chunks_and_strips_the_revision() {
        let out = b"abc123:src/a.rs\x0012\x00fn \x1b[7mx\x1b[m()\nabc123:b\x00 3\x00y\n";
        let mut parser = GrepParser::new(Some("abc123"));
        let mut records = Vec::new();
        for chunk in out.chunks(5) {
            parser.push(chunk, &mut records).unwrap_or_else(|e| {
                // 두 번째 줄의 줄 번호(" 3")는 깨진 것이다.
                assert!(e.0.contains("줄 번호"));
            });
        }
        assert_eq!(records[0].path, b"src/a.rs");
        assert_eq!(records[0].line.line, 12);
        assert_eq!(records[0].line.text, "fn x()");
    }

    #[test]
    fn args_put_paths_after_the_separator() {
        let query = GrepQuery {
            pattern: "-x".to_owned(),
            regex: false,
            case_sensitive: false,
            word: true,
            rev: None,
            paths: vec!["src/".to_owned(), "!*.lock".to_owned(), " ".to_owned()],
        };
        let args = grep_args(&query, Some("abc"));
        let tail: Vec<&str> = args
            .iter()
            .skip_while(|a| *a != "-F")
            .map(String::as_str)
            .collect();
        assert_eq!(
            tail,
            [
                "-F",
                "-i",
                "-w",
                "-e",
                "-x",
                "abc",
                "--",
                "src/",
                ":(exclude)*.lock"
            ]
        );
    }
}
