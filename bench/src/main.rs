//! git baseline 측정.
//!
//! 앱이 git을 띄우는 방식 그대로(`-c core.quotepath=false`, Windows는 `CREATE_NO_WINDOW`) 스폰해
//! 첫 출력, 첫 N개 레코드, 전체 시간을 잰다. hyperfine 같은 도구는 전체 시간만 재므로 직접 잰다.
//! 조건은 두 가지다: commit-graph 없음(`-c core.commitGraph=false`), commit-graph + changed-path Bloom 필터 있음.
//! `plx-blame`은 git이 아니라 앱의 blame 요청(`plx_core::blame_session`)을 그대로 돌려 우리 오버헤드를 잰다.
//!
//! 사용법: plx-bench --repos <디렉터리> --cases <파일> [--runs 5] [--warmup 1]
//! 결과는 Markdown으로 표준 출력에 쓴다. bench/run.sh가 bench/results.md에 넣는다.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

use plx_core::blame_session::{SessionEvent, Sessions};
use plx_core::repo::RepoCache;

struct Options {
    repos: PathBuf,
    cases: PathBuf,
    runs: usize,
    warmup: usize,
}

/// 측정 항목 하나. bench/cases.txt의 한 줄.
struct Case {
    repo: String,
    kind: Kind,
    target: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// `git --version`: 프로세스 생성 비용.
    Spawn,
    /// `git blame --incremental`: 첫 블록까지.
    Blame,
    /// `git log -- <path>`: 첫 50개까지.
    Log,
    /// `git log --follow -- <path>`: 첫 50개까지 (M2의 rename 추적 비교용).
    Follow,
    /// `git log -L a,b:path`: 첫 커밋까지.
    LineLog,
    /// `git show --stat --patch`: 커밋 상세 (M2에서 gix로 대체할 기준).
    Show,
    /// `git log --topo-order HEAD`: 그래프 첫 화면 100개까지.
    /// 브랜치 목록은 클론 시점마다 달라지므로 고정 태그(HEAD)의 이력만 잰다.
    Graph,
    /// 앱의 작업트리 blame 요청: 저장소 찾기·git blame·파일 읽기·파싱·JSON 직렬화(채널로 보낼 모양).
    /// 첫 출력은 내용(meta), 첫 N은 첫 묶음(거터 첫 줄), 전체는 끝(done) 시각이다. 저장소 캐시는 매번 비운다.
    PlxBlame,
    /// 앱의 커밋 상세: gix 트리 비교 + 파일별 줄 수(프로세스 없음). `show`(git CLI)와 비교한다.
    /// 첫 출력 = 결과 전체, 출력 = 화면으로 보내는 JSON 양. 저장소는 매번 새로 연다.
    PlxShow,
}

impl Kind {
    fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "spawn" => Kind::Spawn,
            "blame" => Kind::Blame,
            "log" => Kind::Log,
            "follow" => Kind::Follow,
            "line-log" => Kind::LineLog,
            "show" => Kind::Show,
            "graph" => Kind::Graph,
            "plx-blame" => Kind::PlxBlame,
            "plx-show" => Kind::PlxShow,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Spawn => "git 스폰",
            Kind::Blame => "blame --incremental",
            Kind::Log => "log -- <path>",
            Kind::Follow => "log --follow",
            Kind::LineLog => "log -L",
            Kind::Show => "show",
            Kind::Graph => "log --topo-order",
            Kind::PlxBlame => "plx blame 세션",
            Kind::PlxShow => "plx 커밋 상세",
        }
    }

    /// "첫 N" 열에서 세는 레코드 수와 그 이름.
    fn first_n(self) -> Option<(usize, &'static str)> {
        match self {
            Kind::Blame => Some((1, "첫 블록")),
            Kind::PlxBlame => Some((1, "첫 묶음")),
            Kind::Log | Kind::Follow => Some((50, "첫 50개")),
            Kind::Graph => Some((100, "첫 100개")),
            Kind::Spawn | Kind::LineLog | Kind::Show | Kind::PlxShow => None,
        }
    }

    fn args(self, target: &str) -> Vec<String> {
        // 앱이 쓸 만한 필드를 고른다. -z로 레코드를 NUL로 끊고 필드는 0x1f로 나눈다.
        const LOG_FORMAT: &str = "--format=%H%x1f%an%x1f%at%x1f%s";
        let v: Vec<&str> = match self {
            Kind::Spawn => vec!["--version"],
            Kind::Blame => vec!["blame", "--incremental", "--", target],
            Kind::Log => vec!["log", "-z", LOG_FORMAT, "--", target],
            Kind::Follow => vec!["log", "-z", "--follow", LOG_FORMAT, "--", target],
            Kind::LineLog => {
                return vec!["log".into(), "--format=%H".into(), format!("-L{target}")];
            }
            Kind::Show => vec![
                "show",
                "--stat",
                "--patch",
                "--format=%H%n%an%n%at%n%B",
                target,
            ],
            Kind::Graph => vec![
                "log",
                "--topo-order",
                "-z",
                "--format=%H%x1f%P%x1f%an%x1f%at%x1f%s",
                "HEAD",
            ],
            Kind::PlxBlame | Kind::PlxShow => unreachable!("git을 직접 띄우지 않는다"),
        };
        v.into_iter().map(String::from).collect()
    }
}

/// 레코드 경계 찾기. blame --incremental은 블록마다 `filename <path>` 줄로 끝나고, -z 출력은 NUL로 끝난다.
fn count_records(kind: Kind, chunk: &[u8], carry: &mut Vec<u8>) -> usize {
    match kind {
        Kind::Blame => {
            // 청크 경계에 걸친 "\nfilename "도 찾도록 앞 청크의 꼬리를 붙여 본다.
            const NEEDLE: &[u8] = b"\nfilename ";
            let mut buf = std::mem::take(carry);
            buf.extend_from_slice(chunk);
            let count = buf.windows(NEEDLE.len()).filter(|w| *w == NEEDLE).count();
            let keep = buf.len().saturating_sub(NEEDLE.len() - 1);
            *carry = buf.split_off(keep);
            count
        }
        Kind::Log | Kind::Follow | Kind::Graph => chunk.iter().filter(|&&b| b == 0).count(),
        Kind::Spawn | Kind::LineLog | Kind::Show | Kind::PlxBlame | Kind::PlxShow => 0,
    }
}

#[derive(Clone, Copy)]
enum Condition {
    /// commit-graph를 읽지 않는다 = 클론 직후 상태.
    NoGraph,
    /// commit-graph + changed-path Bloom 필터 (= 앱의 "가속 설정" 이후).
    Bloom,
}

impl Condition {
    /// 앱 코드가 띄우는 git에는 `-c`를 붙일 수 없어 환경 변수(`GIT_CONFIG_COUNT` 등)로 넘긴다.
    fn env(self) -> [(&'static str, &'static str); 2] {
        let [_, a, _, b] = self.config();
        [a, b].map(|kv| kv.split_once('=').expect("key=value"))
    }

    fn config(self) -> [&'static str; 4] {
        match self {
            Condition::NoGraph => [
                "-c",
                "core.commitGraph=false",
                "-c",
                "commitGraph.readChangedPaths=false",
            ],
            Condition::Bloom => [
                "-c",
                "core.commitGraph=true",
                "-c",
                "commitGraph.readChangedPaths=true",
            ],
        }
    }
}

/// 한 번 실행한 결과(ms).
struct Sample {
    first_byte: Option<f64>,
    first_n: Option<f64>,
    total: f64,
    records: usize,
    bytes: usize,
}

fn git_command(repo: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(repo)
        .arg("-c")
        .arg("core.quotepath=false");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn run_once(repo: &Path, kind: Kind, target: &str, condition: Condition) -> io::Result<Sample> {
    let mut command = git_command(repo);
    command
        .args(condition.config())
        .args(kind.args(target))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let start = Instant::now();
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    // stderr가 파이프 버퍼를 채워 git이 멈추지 않게 따로 읽는다.
    let stderr_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = { stderr }.read_to_string(&mut text);
        text
    });

    let ms = |t: Instant| t.duration_since(start).as_secs_f64() * 1000.0;
    let want = kind.first_n().map(|(n, _)| n);
    let mut buf = vec![0u8; 64 * 1024];
    let mut carry = Vec::new();
    let (mut first_byte, mut first_n) = (None, None);
    let (mut records, mut bytes) = (0usize, 0usize);
    loop {
        let n = stdout.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let now = Instant::now();
        first_byte.get_or_insert_with(|| ms(now));
        bytes += n;
        records += count_records(kind, &buf[..n], &mut carry);
        if first_n.is_none() && want.is_some_and(|w| records >= w) {
            first_n = Some(ms(now));
        }
    }
    let status = child.wait()?;
    let total = ms(Instant::now());
    let stderr = stderr_reader.join().unwrap_or_default();
    if !status.success() {
        return Err(io::Error::other(format!(
            "git 실패 ({status}): {}",
            stderr.trim()
        )));
    }
    Ok(Sample {
        first_byte,
        first_n,
        total,
        records,
        bytes,
    })
}

fn run_session_once(repo: &Path, target: &str, condition: Condition) -> io::Result<Sample> {
    let vars = condition.env();
    // SAFETY: plx-bench는 측정 사이에 다른 스레드를 두지 않는다. blame의 stderr 스레드는
    // 끝까지 읽고 join하므로 여기서는 이 스레드만 환경을 읽고 쓴다.
    unsafe {
        std::env::set_var("GIT_CONFIG_COUNT", vars.len().to_string());
        for (i, (key, value)) in vars.iter().enumerate() {
            std::env::set_var(format!("GIT_CONFIG_KEY_{i}"), key);
            std::env::set_var(format!("GIT_CONFIG_VALUE_{i}"), value);
        }
    }
    let sessions = Sessions::new();
    let repos = RepoCache::new();
    let id = sessions.register();
    let path = repo.join(target);

    let start = Instant::now();
    let ms = |t: Instant| t.duration_since(start).as_secs_f64() * 1000.0;
    let (mut first_byte, mut first_n) = (None, None);
    let (mut records, mut bytes) = (0usize, 0usize);
    let mut end = None;
    sessions.run(id, &repos, &path, None, |event| {
        // 앱의 Channel::send처럼 JSON으로 만든다. 출력 열은 화면으로 보내는 양이다.
        bytes += serde_json::to_vec(&event).map_or(0, |json| json.len());
        let now = Instant::now();
        match event {
            SessionEvent::Meta(_) => first_byte = Some(ms(now)),
            SessionEvent::Chunk(chunk) => {
                first_n.get_or_insert_with(|| ms(now));
                records += chunk.blocks.len();
            }
            other => end = Some(other),
        }
        true
    });
    let total = ms(Instant::now());
    match end {
        Some(SessionEvent::Done { .. }) => Ok(Sample {
            first_byte,
            first_n,
            total,
            records,
            bytes,
        }),
        Some(SessionEvent::Error { message }) => Err(io::Error::other(message)),
        other => Err(io::Error::other(format!("예상하지 못한 끝: {other:?}"))),
    }
}

fn run_show_once(repo: &Path, rev: &str) -> io::Result<Sample> {
    let start = Instant::now();
    let opened = plx_core::repo::Repo::discover(repo).map_err(io::Error::other)?;
    let id = opened.resolve_commit(rev).map_err(io::Error::other)?;
    let detail = plx_core::commit::commit_detail(&opened, id).map_err(io::Error::other)?;
    let total = start.elapsed().as_secs_f64() * 1000.0;
    let bytes = serde_json::to_vec(&detail).map_or(0, |json| json.len());
    Ok(Sample {
        first_byte: Some(total),
        first_n: None,
        total,
        records: detail.files.len(),
        bytes,
    })
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[values.len() / 2])
}

struct Summary {
    first_byte: Option<f64>,
    first_n: Option<f64>,
    total: f64,
    records: usize,
    bytes: usize,
}

fn measure(opts: &Options, repo: &Path, case: &Case, condition: Condition) -> io::Result<Summary> {
    let run = || match case.kind {
        Kind::PlxBlame => run_session_once(repo, &case.target, condition),
        Kind::PlxShow => run_show_once(repo, &case.target),
        kind => run_once(repo, kind, &case.target, condition),
    };
    for _ in 0..opts.warmup {
        run()?;
    }
    let mut samples = Vec::with_capacity(opts.runs);
    for _ in 0..opts.runs {
        samples.push(run()?);
    }
    let last = samples.last().expect("runs >= 1");
    Ok(Summary {
        first_byte: median(samples.iter().filter_map(|s| s.first_byte).collect()),
        first_n: median(samples.iter().filter_map(|s| s.first_n).collect()),
        total: median(samples.iter().map(|s| s.total).collect()).unwrap_or(f64::NAN),
        records: last.records,
        bytes: last.bytes,
    })
}

fn fmt_ms(v: Option<f64>) -> String {
    match v {
        Some(v) if v >= 100.0 => format!("{v:.0}"),
        Some(v) => format!("{v:.1}"),
        None => "—".to_owned(),
    }
}

fn cell(s: &Summary) -> String {
    format!(
        "{} / {} / {}",
        fmt_ms(s.first_byte),
        fmt_ms(s.first_n),
        fmt_ms(Some(s.total))
    )
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    git_command(repo)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "?".to_owned())
}

fn cpu_model() -> String {
    #[cfg(target_os = "linux")]
    if let Ok(info) = std::fs::read_to_string("/proc/cpuinfo")
        && let Some(line) = info.lines().find(|l| l.starts_with("model name"))
    {
        return line
            .split_once(':')
            .map_or("?", |(_, v)| v.trim())
            .to_owned();
    }
    #[cfg(target_os = "macos")]
    if let Ok(out) = Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
    {
        return String::from_utf8_lossy(&out.stdout).trim().to_owned();
    }
    #[cfg(windows)]
    if let Ok(id) = std::env::var("PROCESSOR_IDENTIFIER") {
        return id;
    }
    "?".to_owned()
}

fn read_cases(path: &Path) -> Result<Vec<Case>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut cases = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [repo, kind, target] = fields[..] else {
            return Err(format!(
                "{}:{}: `저장소 종류 대상` 세 칸이어야 한다",
                path.display(),
                i + 1
            ));
        };
        let kind = Kind::parse(kind)
            .ok_or_else(|| format!("{}:{}: 알 수 없는 종류 {kind}", path.display(), i + 1))?;
        cases.push(Case {
            repo: repo.to_owned(),
            kind,
            target: target.to_owned(),
        });
    }
    Ok(cases)
}

fn parse_options() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let (mut repos, mut cases, mut runs, mut warmup) = (None, None, 5, 1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{arg} 뒤에 값이 필요하다"))
        };
        match arg.as_str() {
            "--repos" => repos = Some(PathBuf::from(value()?)),
            "--cases" => cases = Some(PathBuf::from(value()?)),
            "--runs" => {
                runs = value()?
                    .parse()
                    .map_err(|_| "--runs는 1 이상의 정수".to_owned())?
            }
            "--warmup" => warmup = value()?.parse().map_err(|_| "--warmup은 정수".to_owned())?,
            other => return Err(format!("알 수 없는 인자: {other}")),
        }
    }
    if runs == 0 {
        return Err("--runs는 1 이상의 정수".to_owned());
    }
    Ok(Options {
        repos: repos.ok_or("--repos가 필요하다")?,
        cases: cases.ok_or("--cases가 필요하다")?,
        runs,
        warmup,
    })
}

fn main() -> ExitCode {
    let result = parse_options().and_then(|opts| {
        let cases = read_cases(&opts.cases)?;
        report(&opts, &cases).map_err(|e| e.to_string())
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("plx-bench: {e}");
            ExitCode::FAILURE
        }
    }
}

fn report(opts: &Options, cases: &[Case]) -> io::Result<()> {
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    println!(
        "- 환경: {} {}, {} ({cores}코어)",
        std::env::consts::OS,
        std::env::consts::ARCH,
        cpu_model()
    );
    println!("- {}", git_output(Path::new("."), &["--version"]));
    println!(
        "- 반복: 예열 {}회 후 {}회 측정, 중앙값(ms)",
        opts.warmup, opts.runs
    );
    let mut repos: Vec<&str> = cases.iter().map(|c| c.repo.as_str()).collect();
    repos.dedup();
    for name in &repos {
        let repo = opts.repos.join(name);
        let tag = git_output(&repo, &["describe", "--tags", "--exact-match", "HEAD"]);
        let sha = git_output(&repo, &["rev-parse", "--short=12", "HEAD"]);
        let count = git_output(&repo, &["rev-list", "--count", "HEAD"]);
        println!("- `{name}`: {tag} ({sha}), HEAD까지 커밋 {count}개");
    }
    println!();
    println!("| 저장소 | 항목 | 대상 | commit-graph 없음 | Bloom 있음 | 레코드 | 출력 |");
    println!("|---|---|---|---|---|---|---|");
    for case in cases {
        let repo = opts.repos.join(&case.repo);
        eprintln!("측정: {} {} {}", case.repo, case.kind.label(), case.target);
        let none = measure(opts, &repo, case, Condition::NoGraph)?;
        let bloom = measure(opts, &repo, case, Condition::Bloom)?;
        let target = match case.kind {
            Kind::Spawn | Kind::Graph => "—".to_owned(),
            _ => format!("`{}`", case.target),
        };
        let first_n = case.kind.first_n().map_or("", |(_, name)| name);
        println!(
            "| {} | {}{} | {} | {} | {} | {} | {} KB |",
            case.repo,
            case.kind.label(),
            if first_n.is_empty() {
                String::new()
            } else {
                format!(" ({first_n})")
            },
            target,
            cell(&none),
            cell(&bloom),
            if case.kind.first_n().is_some() {
                bloom.records.to_string()
            } else {
                "—".to_owned()
            },
            bloom.bytes / 1024,
        );
    }
    println!();
    println!(
        "셀은 `첫 출력 / 첫 N / 전체` (ms). 첫 N은 항목 이름의 괄호 안 기준이고, 해당 없으면 —."
    );
    println!(
        "`plx blame 세션`의 첫 출력은 내용(meta), 레코드는 블록 수, 출력은 화면으로 보내는 JSON 양이다."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blame_records_span_chunk_boundaries() {
        let mut carry = Vec::new();
        let stream = b"abc 1 1 1\nauthor x\nfilename a.rs\ndef 2 2 1\nfilename a.rs\n";
        let mut total = 0;
        // 한 바이트씩 넣어도 경계에 걸친 "\nfilename "을 놓치지 않아야 한다.
        for b in stream.chunks(1) {
            total += count_records(Kind::Blame, b, &mut carry);
        }
        assert_eq!(total, 2);
    }

    #[test]
    fn nul_records() {
        let mut carry = Vec::new();
        assert_eq!(count_records(Kind::Log, b"a\0b\0c", &mut carry), 2);
    }

    #[test]
    fn median_of_samples() {
        assert_eq!(median(vec![3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(vec![]), None);
    }
}
