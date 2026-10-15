//! `git log -z --name-status` 스트리밍 파서와 실행기. 파일 히스토리([`crate::history`])가 쓴다.
//!
//! 커밋 하나의 출력(`-z`라 필드와 줄 끝이 NUL이다):
//! ```text
//! \x01<sha>\0<부모 sha들, 공백 구분>\0<작성자>\0<메일>\0<시각> <시간대>\0<요약>\0   ← FORMAT
//! \n<상태>\0<경로>\0 …                                                           ← --name-status
//! ```
//! 상태가 R·C면 경로가 둘(원래, 새)이다. 머리 다음 첫 상태 앞에만 줄바꿈이 붙는다.
//! merge 커밋은 바뀐 파일이 나오지 않는다(`--diff-merges`를 켜면 경로를 바꾸지 않은 merge까지 모두 나온다).
//! git은 커밋마다 출력을 flush하므로 커밋 하나를 받는 즉시 처리할 수 있다.

use std::fmt;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::commit::FileStatus;
use crate::git::{self, Canceller, StreamError};

/// 작성자는 `.mailmap`을 따른다(`%aN`, git blame과 같다).
const FORMAT: &str = "--format=%x01%H%x00%P%x00%aN%x00%aE%x00%ad%x00%s";

/// 파일을 바꾼 커밋 하나.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_mail: String,
    /// Unix 시각(초)
    pub author_time: i64,
    /// 예: `+0900`
    pub author_tz: String,
    pub summary: String,
    /// 이 커밋에서 파일이 어떻게 바뀌었나. merge 커밋은 git이 알려 주지 않아 없다.
    pub status: Option<FileStatus>,
    /// 이 커밋 뒤의 경로(지운 커밋이면 지우기 전 경로). 이름을 바꾸기 전 구간이면 옛 이름이다.
    pub path: String,
    /// 이 커밋에서 이름이 바뀌었으면 원래 경로
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "git log 출력 해석 실패: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug)]
pub enum LogError {
    Cancelled,
    Io(io::Error),
    Parse(ParseError),
    Git(String),
}

impl From<StreamError> for LogError {
    fn from(e: StreamError) -> Self {
        match e {
            StreamError::Cancelled => LogError::Cancelled,
            StreamError::Io(e) => LogError::Io(e),
            StreamError::Git(message) => LogError::Git(message),
        }
    }
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogError::Cancelled => f.write_str("취소했다"),
            LogError::Io(e) => write!(f, "git log 실행 실패: {e}"),
            LogError::Parse(e) => e.fmt(f),
            LogError::Git(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for LogError {}

/// 머리 필드 수(sha, 부모, 작성자, 메일, 시각, 요약)
const HEADER_FIELDS: usize = 6;

enum State {
    /// 첫 커밋 머리를 기다린다.
    Start,
    /// 머리의 n번째 필드를 기다린다.
    Header(usize),
    /// 상태 또는 다음 커밋 머리를 기다린다.
    Files,
    /// 상태 다음의 경로. rename·copy면 원래 경로를 먼저 받는다.
    Path {
        status: Option<FileStatus>,
        two: bool,
        from: Option<Vec<u8>>,
    },
}

/// 바이트를 받는 대로 커밋을 내놓는 파서. 청크 경계는 아무 데나 와도 된다.
///
/// 커밋은 요청 경로의 상태를 읽는 즉시 내놓는다(다음 커밋이 한참 뒤에 나와도 기다리지 않는다).
/// 요청 경로의 상태가 없는 커밋(merge, 디렉터리 히스토리)은 다음 커밋 머리나 끝에서 내놓는다.
pub struct LogParser {
    path: Vec<u8>,
    state: State,
    /// NUL을 아직 못 받은 토큰
    pending: Vec<u8>,
    /// 만드는 중인 커밋. 이미 내놓았으면 `emitted`.
    current: Option<LogEntry>,
    emitted: bool,
}

impl LogParser {
    /// `path`는 git log에 준 경로(저장소 기준). 커밋의 상태는 이 경로의 것을 쓴다.
    pub fn new(path: &str) -> LogParser {
        LogParser {
            path: path.as_bytes().to_vec(),
            state: State::Start,
            pending: Vec::new(),
            current: None,
            emitted: false,
        }
    }

    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<LogEntry>) -> Result<(), ParseError> {
        let mut rest = bytes;
        while let Some(end) = rest.iter().position(|&b| b == 0) {
            if self.pending.is_empty() {
                self.token(&rest[..end], out)?;
            } else {
                let mut token = std::mem::take(&mut self.pending);
                token.extend_from_slice(&rest[..end]);
                self.token(&token, out)?;
            }
            rest = &rest[end + 1..];
        }
        self.pending.extend_from_slice(rest);
        Ok(())
    }

    /// 출력이 끝났다. 마지막 커밋을 내놓는다.
    pub fn finish(&mut self, out: &mut Vec<LogEntry>) -> Result<(), ParseError> {
        if !self.pending.is_empty() {
            return Err(ParseError("출력이 중간에 끊겼다".into()));
        }
        match self.state {
            State::Start | State::Files => {
                self.emit(out);
                Ok(())
            }
            State::Header(_) | State::Path { .. } => {
                Err(ParseError("커밋 출력이 중간에 끊겼다".into()))
            }
        }
    }

    fn emit(&mut self, out: &mut Vec<LogEntry>) {
        if let Some(entry) = self.current.take()
            && !self.emitted
        {
            out.push(entry);
        }
        self.emitted = false;
    }

    fn token(&mut self, token: &[u8], out: &mut Vec<LogEntry>) -> Result<(), ParseError> {
        match &mut self.state {
            State::Start | State::Files => {
                let token = token.strip_prefix(b"\n").unwrap_or(token);
                if let Some(sha) = token.strip_prefix(b"\x01") {
                    self.emit(out);
                    if sha.is_empty() {
                        return Err(ParseError("sha가 없다".into()));
                    }
                    self.current = Some(LogEntry {
                        sha: text(sha),
                        parents: Vec::new(),
                        author: String::new(),
                        author_mail: String::new(),
                        author_time: 0,
                        author_tz: String::new(),
                        summary: String::new(),
                        status: None,
                        path: text(&self.path),
                        old_path: None,
                    });
                    self.state = State::Header(1);
                    return Ok(());
                }
                if matches!(self.state, State::Start) {
                    return Err(ParseError(format!(
                        "커밋 머리가 아니다: {}",
                        preview(token)
                    )));
                }
                let (status, two) = parse_status(token)?;
                self.state = State::Path {
                    status,
                    two,
                    from: None,
                };
            }
            State::Header(n) => {
                let n = *n;
                let entry = self.current.as_mut().expect("머리를 읽는 중");
                match n {
                    1 => {
                        entry.parents = text(token).split_whitespace().map(str::to_owned).collect()
                    }
                    2 => entry.author = text(token),
                    3 => entry.author_mail = text(token),
                    4 => {
                        // --date=raw: "<초> <시간대>"
                        let raw = text(token);
                        let mut parts = raw.split_whitespace();
                        entry.author_time = parts
                            .next()
                            .and_then(|t| t.parse().ok())
                            .ok_or_else(|| ParseError(format!("시각이 아니다: {raw}")))?;
                        entry.author_tz = parts.next().unwrap_or("+0000").to_owned();
                    }
                    _ => entry.summary = text(token),
                }
                self.state = if n + 1 < HEADER_FIELDS {
                    State::Header(n + 1)
                } else {
                    State::Files
                };
            }
            State::Path { status, two, from } => {
                if *two && from.is_none() {
                    *from = Some(token.to_vec());
                    return Ok(());
                }
                let (status, from) = (*status, from.take());
                self.state = State::Files;
                // 요청 경로의 첫 상태만 쓴다(디렉터리면 맞는 경로가 없다).
                if !self.emitted
                    && token == self.path.as_slice()
                    && let Some(entry) = self.current.as_mut()
                {
                    entry.status = status;
                    entry.old_path = from.as_deref().map(text);
                    out.push(entry.clone());
                    self.emitted = true;
                }
            }
        }
        Ok(())
    }
}

/// `M`, `A`, `D`, `T`, `R067`, `C100` … 모르는 글자(U, X, B)는 상태 없음으로 둔다.
fn parse_status(token: &[u8]) -> Result<(Option<FileStatus>, bool), ParseError> {
    let Some((&letter, score)) = token.split_first() else {
        return Err(ParseError("상태가 비었다".into()));
    };
    if !score.iter().all(u8::is_ascii_digit) || !letter.is_ascii_uppercase() {
        return Err(ParseError(format!("상태가 아니다: {}", preview(token))));
    }
    let status = match letter {
        b'A' => Some(FileStatus::Added),
        b'D' => Some(FileStatus::Deleted),
        b'M' => Some(FileStatus::Modified),
        b'T' => Some(FileStatus::TypeChanged),
        b'R' => Some(FileStatus::Renamed),
        b'C' => Some(FileStatus::Copied),
        _ => None,
    };
    Ok((status, matches!(letter, b'R' | b'C')))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn preview(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.chars().take(40).collect()
}

/// `git log <starts> -- <path>` 실행. 읽는 쪽은 [`LogStream::read`]로 커밋을 받는다.
pub struct LogStream {
    git: git::Streaming,
    parser: LogParser,
    buf: Vec<u8>,
    done: bool,
}

impl LogStream {
    /// `starts`(커밋 sha)에서 거슬러 올라가며 `path`(저장소 기준)를 바꾼 커밋을 찾는다.
    ///
    /// 사용자 설정이 결과를 바꾸지 않게 막는다: `log.follow`(경로 하나면 `--follow`처럼 동작),
    /// `log.showSignature`(서명 확인 출력이 끼어든다), `log.showRoot`(루트 커밋의 상태가 빠진다),
    /// `i18n.logOutputEncoding`(메시지를 다른 인코딩으로 낸다), 색.
    pub fn spawn(repo_dir: &Path, starts: &[String], path: &str) -> io::Result<LogStream> {
        let mut command = git::command(repo_dir);
        command.args([
            "-c",
            "log.showRoot=true",
            "log",
            "-z",
            FORMAT,
            "--date=raw",
            "--name-status",
            "--no-follow",
            "--no-show-signature",
            "--no-color",
            "--encoding=UTF-8",
        ]);
        command.args(starts).arg("--").arg(path);
        Ok(LogStream {
            git: git::Streaming::spawn(command, "git log")?,
            parser: LogParser::new(path),
            buf: vec![0; 64 * 1024],
            done: false,
        })
    }

    pub fn canceller(&self) -> Canceller {
        self.git.canceller()
    }

    /// git이 내놓은 만큼 커밋을 `out`에 더한다(없을 수도 있다). 출력이 끝났으면 `false`.
    pub fn read(&mut self, out: &mut Vec<LogEntry>) -> Result<bool, LogError> {
        if self.done {
            return Ok(false);
        }
        let n = self.git.read(&mut self.buf)?;
        if n == 0 {
            self.done = true;
            self.parser.finish(out).map_err(|e| self.fail(e))?;
            return Ok(false);
        }
        self.parser
            .push(&self.buf[..n], out)
            .map_err(|e| self.fail(e))?;
        Ok(true)
    }

    /// 출력을 끝까지 읽은 뒤 git이 성공했는지 본다.
    pub fn finish(self) -> Result<(), LogError> {
        self.git.finish().map_err(LogError::from)
    }

    fn fail(&self, error: ParseError) -> LogError {
        if self.git.abort() {
            LogError::Cancelled
        } else {
            LogError::Parse(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SHA_M: &str = "cccccccccccccccccccccccccccccccccccccccc";

    fn sample() -> Vec<u8> {
        let mut out = Vec::new();
        // merge: 바뀐 파일 없음
        out.extend_from_slice(
            format!("\x01{SHA_M}\0{SHA_A} {SHA_B}\0김개발\0kim@example.com\01700000000 +0900\0Merge branch 'x'\0")
                .as_bytes(),
        );
        out.extend_from_slice(
            format!("\x01{SHA_A}\0{SHA_B}\0B\0b@example.com\01700000100 -0130\0한글 요약\0\nM\0문서/설명.md\0")
                .as_bytes(),
        );
        // 루트 커밋: 부모가 비었다
        out.extend_from_slice(
            format!("\x01{SHA_B}\0\0C\0c@e\01600000000 +0000\0root\0\nA\0문서/설명.md\0")
                .as_bytes(),
        );
        out
    }

    fn parse(chunks: &[&[u8]], path: &str) -> Vec<LogEntry> {
        let mut parser = LogParser::new(path);
        let mut out = Vec::new();
        for chunk in chunks {
            parser.push(chunk, &mut out).unwrap();
        }
        parser.finish(&mut out).unwrap();
        out
    }

    #[test]
    fn parses_merges_statuses_and_roots() {
        let bytes = sample();
        let entries = parse(&[&bytes], "문서/설명.md");
        assert_eq!(entries.len(), 3);
        let merge = &entries[0];
        assert_eq!(merge.sha, SHA_M);
        assert_eq!(merge.parents, [SHA_A, SHA_B]);
        assert_eq!(merge.author, "김개발");
        assert_eq!(merge.author_mail, "kim@example.com");
        assert_eq!(
            (merge.author_time, merge.author_tz.as_str()),
            (1_700_000_000, "+0900")
        );
        assert_eq!(merge.status, None);
        assert_eq!(merge.path, "문서/설명.md");
        assert_eq!(entries[1].summary, "한글 요약");
        assert_eq!(entries[1].author_tz, "-0130");
        assert_eq!(entries[1].status, Some(FileStatus::Modified));
        assert_eq!(entries[2].parents, Vec::<String>::new());
        assert_eq!(entries[2].status, Some(FileStatus::Added));
    }

    #[test]
    fn chunk_boundaries_do_not_matter() {
        let bytes = sample();
        let whole = parse(&[&bytes], "문서/설명.md");
        for size in [1, 2, 7, 64] {
            let chunks: Vec<&[u8]> = bytes.chunks(size).collect();
            assert_eq!(parse(&chunks, "문서/설명.md"), whole, "{size}바이트씩");
        }
    }

    #[test]
    fn emits_an_entry_as_soon_as_its_status_arrives() {
        let bytes = sample();
        let mut parser = LogParser::new("문서/설명.md");
        let mut out = Vec::new();
        // 두 번째 커밋의 상태·경로까지만 받았다: merge와 두 번째 커밋은 이미 나왔다.
        let second_end = bytes
            .windows("설명.md\0".len())
            .position(|w| w == "설명.md\0".as_bytes())
            .unwrap()
            + "설명.md\0".len();
        parser.push(&bytes[..second_end], &mut out).unwrap();
        assert_eq!(
            out.iter().map(|e| e.sha.as_str()).collect::<Vec<_>>(),
            [SHA_M, SHA_A]
        );
        parser.push(&bytes[second_end..], &mut out).unwrap();
        parser.finish(&mut out).unwrap();
        assert_eq!(out.len(), 3, "같은 커밋을 두 번 내놓지 않는다");
    }

    #[test]
    fn renames_and_directory_paths() {
        let bytes = format!(
            "\x01{SHA_A}\0{SHA_B}\0a\0a@e\01 +0000\0s\0\nR087\0src/old.rs\0src/new.rs\0M\0src/other.rs\0"
        );
        let entries = parse(&[bytes.as_bytes()], "src/new.rs");
        assert_eq!(entries[0].status, Some(FileStatus::Renamed));
        assert_eq!(entries[0].old_path.as_deref(), Some("src/old.rs"));
        // 디렉터리 히스토리: 맞는 경로가 없으면 상태가 없고 경로는 요청 경로다.
        let entries = parse(&[bytes.as_bytes()], "src");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, None);
        assert_eq!(entries[0].path, "src");
    }

    #[test]
    fn truncated_or_garbage_output_is_an_error() {
        let bytes = sample();
        let mut parser = LogParser::new("x");
        let mut out = Vec::new();
        parser.push(&bytes[..30], &mut out).unwrap();
        assert!(parser.finish(&mut out).is_err());

        let mut parser = LogParser::new("x");
        assert!(parser.push(b"not a header\0", &mut out).is_err());
        let mut parser = LogParser::new("x");
        let bad = format!("\x01{SHA_A}\0\0a\0a@e\0soon\0s\0");
        assert!(parser.push(bad.as_bytes(), &mut out).is_err());
        let mut parser = LogParser::new("x");
        let bad = format!("\x01{SHA_A}\0\0a\0a@e\01 +0000\0s\0\nmodified\0x\0");
        assert!(parser.push(bad.as_bytes(), &mut out).is_err());
    }

    #[test]
    fn empty_output_has_no_entries() {
        assert_eq!(parse(&[], "x"), []);
    }
}
