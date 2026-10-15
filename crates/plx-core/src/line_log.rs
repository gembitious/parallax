//! 라인 히스토리: `git log -L <a>,<b>:<경로>` 스트리밍 파서와 실행기.
//!
//! 커밋마다 머리([`FORMAT`], 필드는 NUL로 끊은 한 줄) 다음에 그 줄 범위의 patch가 온다. patch는 다음 커밋 머리
//! (줄 맨 앞의 `\x01`)까지다. patch의 줄은 `diff`·`---`·`+++`·`@@` 머리이거나 ` `·`-`·`+`·`\`로 시작하므로
//! 머리와 헷갈리지 않는다.
//! ```text
//! \x01<sha>\0<부모들>\0<작성자>\0<메일>\0<시각> <시간대>\0<요약>\0
//! (빈 줄)
//! diff --git a/<옛 경로> b/<경로>
//! --- a/<옛 경로>          ← 새로 생긴 파일이면 /dev/null
//! +++ b/<경로>
//! @@ -<a>,<n> +<c>,<m> @@
//!  같은 줄 / -뺀 줄 / +더한 줄
//! ```
//! git이 rename을 따라 옛 경로로 이어 간다. merge 커밋은 patch 없이 머리만 나올 수 있다.
//! `-z`가 없어 특수 문자가 든 경로는 C 문자열로 감싸 나온다([`git::unquote_path`]).

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::commit::FileStatus;
use crate::git::{self, Canceller};
use crate::log::{EntrySource, LogEntry, LogError, ParseError};
use crate::text;

const FORMAT: &str = "--format=%x01%H%x00%P%x00%aN%x00%aE%x00%ad%x00%s%x00";

/// 줄 범위의 변경 한 덩어리
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// 줄마다 첫 글자가 ` `(같음)·`-`(뺌)·`+`(더함)·`\`(끝 개행 없음). 파일 인코딩(UTF-8, 아니면 EUC-KR)으로 읽었다.
    pub lines: Vec<String>,
}

/// 머리 필드 수(sha, 부모, 작성자, 메일, 시각, 요약)
const HEADER_FIELDS: usize = 6;

/// 줄 단위로 받는 대로 커밋을 내놓는 파서. 청크 경계는 아무 데나 와도 된다.
/// 커밋은 다음 커밋 머리나 끝에서 내놓는다(patch가 어디서 끝나는지 그때 안다).
pub struct LineLogParser {
    path: String,
    /// 줄 끝(`\n`)을 아직 못 받은 바이트
    pending: Vec<u8>,
    current: Option<Building>,
}

struct Building {
    entry: LogEntry,
    hunks: Vec<RawHunk>,
    /// `--- /dev/null`을 봤다
    added: bool,
    old_path: Option<String>,
    new_path: Option<String>,
}

struct RawHunk {
    header: (u32, u32, u32, u32),
    /// 줄마다 (첫 글자, 내용 바이트)
    lines: Vec<(u8, Vec<u8>)>,
    /// 아직 받지 않은 옛·새 쪽 줄 수. 둘 다 0이면 hunk가 끝났다(그다음 `---`·`+++`는 파일 머리다).
    old_left: u32,
    new_left: u32,
}

impl LineLogParser {
    /// `path`는 범위를 준 파일(저장소 기준). patch에 경로가 없으면 이 경로를 쓴다.
    pub fn new(path: &str) -> LineLogParser {
        LineLogParser {
            path: path.to_owned(),
            pending: Vec::new(),
            current: None,
        }
    }

    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<LogEntry>) -> Result<(), ParseError> {
        let mut rest = bytes;
        while let Some(end) = rest.iter().position(|&b| b == b'\n') {
            if self.pending.is_empty() {
                self.line(&rest[..end], out)?;
            } else {
                let mut line = std::mem::take(&mut self.pending);
                line.extend_from_slice(&rest[..end]);
                self.line(&line, out)?;
            }
            rest = &rest[end + 1..];
        }
        self.pending.extend_from_slice(rest);
        Ok(())
    }

    pub fn finish(&mut self, out: &mut Vec<LogEntry>) -> Result<(), ParseError> {
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.line(&line, out)?;
        }
        self.emit(out);
        Ok(())
    }

    fn emit(&mut self, out: &mut Vec<LogEntry>) {
        let Some(building) = self.current.take() else {
            return;
        };
        let mut entry = building.entry;
        if let Some(path) = building.new_path {
            entry.path = path;
        }
        entry.status = if building.hunks.is_empty() {
            None
        } else if building.added {
            Some(FileStatus::Added)
        } else if building
            .old_path
            .as_ref()
            .is_some_and(|old| *old != entry.path)
        {
            entry.old_path = building.old_path;
            Some(FileStatus::Renamed)
        } else {
            Some(FileStatus::Modified)
        };
        // 내용 줄은 한꺼번에 디코드한다(EUC-KR 파일도 한 줄씩 끊어 읽으면 결과가 같다: 줄바꿈은 ASCII다).
        let mut bytes = Vec::new();
        for hunk in &building.hunks {
            for (_, line) in &hunk.lines {
                bytes.extend_from_slice(line);
                bytes.push(b'\n');
            }
        }
        let decoded = text::decode(&bytes).text;
        let mut texts = decoded.split('\n');
        entry.hunks = Some(
            building
                .hunks
                .into_iter()
                .map(|hunk| {
                    let (old_start, old_lines, new_start, new_lines) = hunk.header;
                    let lines = hunk
                        .lines
                        .iter()
                        .map(|(kind, _)| {
                            let text = texts.next().unwrap_or("");
                            format!("{}{text}", *kind as char)
                        })
                        .collect();
                    Hunk {
                        old_start,
                        old_lines,
                        new_start,
                        new_lines,
                        lines,
                    }
                })
                .collect(),
        );
        out.push(entry);
    }

    fn line(&mut self, line: &[u8], out: &mut Vec<LogEntry>) -> Result<(), ParseError> {
        if let Some(header) = line.strip_prefix(b"\x01") {
            self.emit(out);
            self.current = Some(Building {
                entry: header_entry(header, &self.path)?,
                hunks: Vec::new(),
                added: false,
                old_path: None,
                new_path: None,
            });
            return Ok(());
        }
        let Some(building) = self.current.as_mut() else {
            if line.is_empty() {
                return Ok(());
            }
            return Err(ParseError(format!(
                "커밋 머리가 아니다: {}",
                String::from_utf8_lossy(&line[..line.len().min(40)])
            )));
        };
        // hunk 안의 줄: 머리의 줄 수만큼 받는다(내용이 `-- `·`++ `로 시작해도 파일 머리와 헷갈리지 않는다).
        if let Some(hunk) = building.hunks.last_mut() {
            let open = hunk.old_left > 0 || hunk.new_left > 0;
            let taken = match line.first() {
                Some(b' ') if open => {
                    hunk.old_left = hunk.old_left.saturating_sub(1);
                    hunk.new_left = hunk.new_left.saturating_sub(1);
                    true
                }
                Some(b'-') if hunk.old_left > 0 => {
                    hunk.old_left -= 1;
                    true
                }
                Some(b'+') if hunk.new_left > 0 => {
                    hunk.new_left -= 1;
                    true
                }
                // "\ No newline at end of file": 바로 앞 줄에 붙는다.
                Some(b'\\') if !hunk.lines.is_empty() => true,
                _ => false,
            };
            if taken {
                // CRLF 파일의 줄 끝 CR은 보이지 않는 글자라 뗀다(blame·diff 화면과 같다).
                let content = &line[1..];
                let content = content.strip_suffix(b"\r").unwrap_or(content);
                hunk.lines.push((line[0], content.to_vec()));
                return Ok(());
            }
        }
        if line.starts_with(b"@@ ") {
            let header = hunk_header(line)?;
            building.hunks.push(RawHunk {
                header,
                lines: Vec::new(),
                old_left: header.1,
                new_left: header.3,
            });
        } else if let Some(path) = line.strip_prefix(b"--- ") {
            if path == b"/dev/null" {
                building.added = true;
            } else {
                building.old_path = Some(prefixed_path(path, b"a/"));
            }
        } else if let Some(path) = line.strip_prefix(b"+++ ") {
            building.new_path = Some(prefixed_path(path, b"b/"));
        }
        // 빈 줄, diff --git, index, similarity, rename from/to, new file mode … 는 쓰지 않는다.
        Ok(())
    }
}

/// `@@ -a,b +c,d @@ …` (`,b`가 없으면 1줄)
fn hunk_header(line: &[u8]) -> Result<(u32, u32, u32, u32), ParseError> {
    let bad = || {
        ParseError(format!(
            "hunk 머리가 아니다: {}",
            String::from_utf8_lossy(line)
        ))
    };
    let text = std::str::from_utf8(line).map_err(|_| bad())?;
    let mut parts = text.split_whitespace().skip(1);
    let range = |part: Option<&str>, sign: char| -> Option<(u32, u32)> {
        let part = part?.strip_prefix(sign)?;
        let (start, count) = part.split_once(',').unwrap_or((part, "1"));
        Some((start.parse().ok()?, count.parse().ok()?))
    };
    let (old_start, old_lines) = range(parts.next(), '-').ok_or_else(bad)?;
    let (new_start, new_lines) = range(parts.next(), '+').ok_or_else(bad)?;
    Ok((old_start, old_lines, new_start, new_lines))
}

/// `a/경로` 또는 `"a/특수\"경로"`에서 접두어를 뗀 경로
fn prefixed_path(raw: &[u8], prefix: &[u8]) -> String {
    let unquoted = git::unquote_path(raw);
    let path = unquoted.strip_prefix(prefix).unwrap_or(&unquoted);
    String::from_utf8_lossy(path).into_owned()
}

fn header_entry(header: &[u8], path: &str) -> Result<LogEntry, ParseError> {
    let fields: Vec<&[u8]> = header.split(|&b| b == 0).collect();
    if fields.len() < HEADER_FIELDS {
        return Err(ParseError(format!(
            "커밋 머리 필드가 모자라다: {}",
            String::from_utf8_lossy(header)
        )));
    }
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let date = text(fields[4]);
    let mut date_parts = date.split_whitespace();
    let author_time = date_parts
        .next()
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| ParseError(format!("시각이 아니다: {date}")))?;
    Ok(LogEntry {
        sha: text(fields[0]),
        parents: text(fields[1])
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
        author: text(fields[2]),
        author_mail: text(fields[3]),
        author_time,
        author_tz: date_parts.next().unwrap_or("+0000").to_owned(),
        summary: text(fields[5]),
        status: None,
        path: path.to_owned(),
        old_path: None,
        hunks: None,
    })
}

/// `git log -L <start>,<end>:<path> <rev>` 실행.
pub struct LineLogStream {
    git: git::Streaming,
    parser: LineLogParser,
    buf: Vec<u8>,
    done: bool,
}

impl LineLogStream {
    /// `rev`(커밋 sha)의 `path` 파일 `start`..=`end`줄(1부터)을 바꾼 커밋을 거슬러 올라가며 찾는다.
    ///
    /// patch 모양이 사용자 설정에 흔들리지 않게 접두어(`a/`·`b/`)를 정하고 외부 diff·textconv·색을 끈다.
    pub fn spawn(
        repo_dir: &Path,
        rev: &str,
        path: &str,
        start: u32,
        end: u32,
    ) -> io::Result<LineLogStream> {
        let mut command = git::command(repo_dir);
        command.args([
            "log",
            FORMAT,
            "--date=raw",
            "--no-show-signature",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--encoding=UTF-8",
        ]);
        command.arg(format!("-L{start},{end}:{path}")).arg(rev);
        Ok(LineLogStream {
            git: git::Streaming::spawn(command, "git log -L")?,
            parser: LineLogParser::new(path),
            buf: vec![0; 64 * 1024],
            done: false,
        })
    }

    pub fn canceller(&self) -> Canceller {
        self.git.canceller()
    }

    /// git이 내놓은 만큼 커밋을 `out`에 더한다. 출력이 끝났으면 `false`.
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

impl EntrySource for LineLogStream {
    fn canceller(&self) -> Canceller {
        LineLogStream::canceller(self)
    }

    fn read(&mut self, out: &mut Vec<LogEntry>) -> Result<bool, LogError> {
        LineLogStream::read(self, out)
    }

    fn finish(self) -> Result<(), LogError> {
        LineLogStream::finish(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(chunks: &[&[u8]], path: &str) -> Vec<LogEntry> {
        let mut parser = LineLogParser::new(path);
        let mut out = Vec::new();
        for chunk in chunks {
            parser.push(chunk, &mut out).unwrap();
        }
        parser.finish(&mut out).unwrap();
        out
    }

    /// 줄들을 `\n`으로 잇는다(문자열 리터럴의 줄 이음 `\`는 다음 줄 앞 공백을 지워 hunk의 같은 줄을 망가뜨린다).
    fn lines(lines: &[&str]) -> String {
        lines.iter().map(|l| format!("{l}\n")).collect()
    }

    fn rename() -> String {
        lines(&[
            "\x01aaaa\0pppp\0김개발\0kim@example.com\x001700000000 +0900\0rename\0",
            "",
            "diff --git a/old.rs b/new.rs",
            "--- a/old.rs",
            "+++ b/new.rs",
            "@@ -1,3 +1,3 @@",
            "-fn old() {",
            "+fn new() {",
            "     body",
            " }",
            "\x01pppp\0\0김개발\0kim@example.com\x001600000000 -0130\0add\0",
            "",
            "diff --git a/old.rs b/old.rs",
            "--- /dev/null",
            "+++ b/old.rs",
            "@@ -0,0 +1,3 @@",
            "+fn old() {",
            "+    body",
            "+}",
        ])
    }

    #[test]
    fn parses_renames_and_additions() {
        let entries = parse(&[rename().as_bytes()], "new.rs");
        assert_eq!(entries.len(), 2);
        let rename = &entries[0];
        assert_eq!(rename.sha, "aaaa");
        assert_eq!(rename.parents, ["pppp"]);
        assert_eq!(rename.author, "김개발");
        assert_eq!(
            (rename.author_time, rename.author_tz.as_str()),
            (1_700_000_000, "+0900")
        );
        assert_eq!(rename.status, Some(FileStatus::Renamed));
        assert_eq!(rename.path, "new.rs");
        assert_eq!(rename.old_path.as_deref(), Some("old.rs"));
        let hunks = rename.hunks.as_ref().unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(
            (
                hunks[0].old_start,
                hunks[0].old_lines,
                hunks[0].new_start,
                hunks[0].new_lines
            ),
            (1, 3, 1, 3)
        );
        assert_eq!(
            hunks[0].lines,
            ["-fn old() {", "+fn new() {", "     body", " }"]
        );
        let added = &entries[1];
        assert_eq!(added.status, Some(FileStatus::Added));
        assert_eq!(added.path, "old.rs");
        assert_eq!(added.old_path, None);
        assert_eq!(added.author_tz, "-0130");
        assert_eq!(added.hunks.as_ref().unwrap()[0].lines.len(), 3);
    }

    #[test]
    fn chunk_boundaries_do_not_matter() {
        let text = rename();
        let whole = parse(&[text.as_bytes()], "new.rs");
        for size in [1, 3, 17] {
            let chunks: Vec<&[u8]> = text.as_bytes().chunks(size).collect();
            assert_eq!(parse(&chunks, "new.rs"), whole, "{size}바이트씩");
        }
    }

    #[test]
    fn content_that_looks_like_headers_stays_in_the_hunk() {
        // 뺀 줄 "-- 주석"과 더한 줄 "++ x"는 hunk의 줄 수 안에 있으므로 파일 머리가 아니다.
        let text = lines(&[
            "\x01aaaa\0pppp\0a\0a@e\x001 +0000\0s\0",
            "",
            "diff --git a/f.sql b/f.sql",
            "--- a/f.sql",
            "+++ b/f.sql",
            "@@ -2,2 +2,2 @@",
            "--- 주석",
            "+++ x",
            " 끝",
            "\\ No newline at end of file",
        ]);
        let entries = parse(&[text.as_bytes()], "f.sql");
        let hunk = &entries[0].hunks.as_ref().unwrap()[0];
        assert_eq!(
            hunk.lines,
            ["--- 주석", "+++ x", " 끝", "\\ No newline at end of file"]
        );
        assert_eq!(entries[0].path, "f.sql");
        assert_eq!(entries[0].status, Some(FileStatus::Modified));
    }

    #[test]
    fn merges_without_patch_and_quoted_paths() {
        let text = lines(&[
            "\x01mmmm\0aaaa bbbb\0a\0a@e\x001 +0000\0Merge\0",
            "",
            "\x01aaaa\0pppp\0a\0a@e\x001 +0000\0s\0",
            "",
            "diff --git \"a/a\\\"b.rs\" \"b/a\\\"b.rs\"",
            "--- \"a/a\\\"b.rs\"",
            "+++ \"b/a\\\"b.rs\"",
            "@@ -1 +1 @@",
            "-x\r",
            "+y\r",
        ]);
        let entries = parse(&[text.as_bytes()], "a\"b.rs");
        assert_eq!(entries[0].status, None);
        assert_eq!(entries[0].hunks.as_deref(), Some(&[][..]));
        assert_eq!(entries[1].path, "a\"b.rs");
        assert_eq!(entries[1].status, Some(FileStatus::Modified));
        // CRLF 파일의 CR은 뗀다
        assert_eq!(entries[1].hunks.as_ref().unwrap()[0].lines, ["-x", "+y"]);
    }

    #[test]
    fn euc_kr_content_is_decoded() {
        let mut bytes = b"\x01aaaa\0\0a\0a@e\x001 +0000\0s\0\n\ndiff --git a/e.txt b/e.txt\n--- /dev/null\n+++ b/e.txt\n@@ -0,0 +1 @@\n+".to_vec();
        // EUC-KR "안녕하세요"
        bytes.extend_from_slice(b"\xbe\xc8\xb3\xe7\xc7\xcf\xbc\xbc\xbf\xe4\n");
        let entries = parse(&[&bytes], "e.txt");
        assert_eq!(entries[0].hunks.as_ref().unwrap()[0].lines, ["+안녕하세요"]);
    }

    #[test]
    fn garbage_before_the_first_header_is_an_error() {
        let mut parser = LineLogParser::new("x");
        assert!(parser.push(b"fatal: nope\n", &mut Vec::new()).is_err());
        let mut parser = LineLogParser::new("x");
        assert!(
            parser
                .push(b"\x01aaaa\0only two\0\n", &mut Vec::new())
                .is_err()
        );
    }
}
