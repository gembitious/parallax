//! "가속 설정": commit-graph와 changed-path Bloom 필터가 있는지 보고, 사용자가 확인하면 만든다.
//!
//! 파일·라인 히스토리(`git log -- <경로>`, `git log -L`)는 Bloom 필터가 있으면 커밋마다 트리를 비교하지 않아
//! 몇 배 빨라진다(bench/results.md). 저장소를 바꾸는 유일한 동작이다(CLAUDE.md): `.git/objects/info` 아래에
//! commit-graph 파일만 쓰고 커밋·참조·작업트리는 건드리지 않는다.
//!
//! 읽기는 git 형식(gitformat-commit-graph)의 머리만 본다: `CGPH`, 버전, 해시 버전, 청크 수, 기반 그래프 수,
//! 청크 표(ID 4바이트 + 위치 8바이트, 청크 수 + 1개). Bloom 필터는 `BIDX`·`BDAT` 청크이고 `BDAT`의 첫 4바이트가
//! 필터 버전이다. 그래프는 `objects/info/commit-graph` 하나이거나 `objects/info/commit-graphs/commit-graph-chain`에
//! 적힌 층들이다. 커밋이 그래프에 있는지는 `OIDF`(fanout)·`OIDL`(정렬된 id)로 찾는다.
//!
//! 필터 v1은 경로에 0x80 이상 바이트(한글 등)가 있으면 `char` 부호에 따라 해시가 달라지는 문제가 있어 git 2.46에
//! v2가 생겼다. git이 v2를 쓸 수 있으면 v2로 쓴다.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use gix::ObjectId;
use serde::Serialize;

use crate::git;
use crate::repo::Repo;

/// 저장소의 가속 상태
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphStatus {
    /// commit-graph가 있다
    pub graph: bool,
    /// 그래프(모든 층)에 든 커밋 수
    pub commits: u64,
    /// 모든 층에 changed-path Bloom 필터가 있다
    pub bloom: bool,
    /// Bloom 필터 버전. 층마다 다르면 가장 낮은 것
    pub bloom_version: Option<u32>,
    /// HEAD 커밋이 그래프에 있다(없으면 그 뒤 커밋은 필터 없이 걷는다: 다시 쓰면 좋다)
    pub head_covered: bool,
    /// 쓸 때 필터 버전(git 2.46부터 2)
    pub write_version: u32,
}

impl GraphStatus {
    /// 다시 쓸 필요가 없다
    pub fn accelerated(&self) -> bool {
        self.graph && self.bloom && self.head_covered
    }
}

/// commit-graph 층 하나
#[derive(Debug, Clone, PartialEq, Eq)]
struct Layer {
    commits: u64,
    bloom_version: Option<u32>,
    /// 커밋 id 길이(SHA-1 20, SHA-256 32)
    hash_len: usize,
    /// OIDF 청크 위치(256 × 4바이트), OIDL 청크 위치
    fanout: u64,
    lookup: u64,
    path: PathBuf,
}

/// `objects` 디렉터리(저장소의 공통 디렉터리 아래)의 commit-graph 층들. 없으면 빈 목록.
fn layers(objects: &Path) -> io::Result<Vec<Layer>> {
    let info = objects.join("info");
    let chain = info.join("commit-graphs").join("commit-graph-chain");
    let files: Vec<PathBuf> = match fs::read_to_string(&chain) {
        // 사슬 파일이 있으면 그것이 그래프다(단일 파일은 무시된다)
        Ok(text) => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|hash| {
                info.join("commit-graphs")
                    .join(format!("graph-{hash}.graph"))
            })
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let single = info.join("commit-graph");
            if single.is_file() {
                vec![single]
            } else {
                Vec::new()
            }
        }
        Err(e) => return Err(e),
    };
    files.iter().map(|path| read_layer(path)).collect()
}

fn read_layer(path: &Path) -> io::Result<Layer> {
    let bad = |what: &str| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {what}", path.display()),
        )
    };
    let mut file = fs::File::open(path)?;
    let mut head = [0u8; 8];
    file.read_exact(&mut head)?;
    if &head[..4] != b"CGPH" {
        return Err(bad("commit-graph 파일이 아니다"));
    }
    if head[4] != 1 {
        return Err(bad("모르는 commit-graph 버전"));
    }
    let hash_len = match head[5] {
        1 => 20,
        2 => 32,
        _ => return Err(bad("모르는 해시 버전")),
    };
    let chunks = usize::from(head[6]);
    let mut table = vec![0u8; (chunks + 1) * 12];
    file.read_exact(&mut table)?;
    let chunk = |id: &[u8; 4]| -> Option<u64> {
        table.chunks_exact(12).take(chunks).find_map(|entry| {
            (&entry[..4] == id)
                .then(|| u64::from_be_bytes(entry[4..12].try_into().expect("8바이트")))
        })
    };
    let fanout = chunk(b"OIDF").ok_or_else(|| bad("OIDF 청크가 없다"))?;
    let lookup = chunk(b"OIDL").ok_or_else(|| bad("OIDL 청크가 없다"))?;
    let commits = u64::from(read_u32(&mut file, fanout + 255 * 4)?);
    let bloom_version = match (chunk(b"BIDX"), chunk(b"BDAT")) {
        (Some(_), Some(data)) => Some(read_u32(&mut file, data)?),
        _ => None,
    };
    Ok(Layer {
        commits,
        bloom_version,
        hash_len,
        fanout,
        lookup,
        path: path.to_owned(),
    })
}

fn read_u32(file: &mut fs::File, offset: u64) -> io::Result<u32> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

impl Layer {
    /// 이 층에 `id` 커밋이 있는가(fanout으로 범위를 좁히고 정렬된 id를 이진 탐색한다).
    fn contains(&self, id: &ObjectId) -> io::Result<bool> {
        use std::io::{Seek, SeekFrom};
        let bytes = id.as_bytes();
        if bytes.len() != self.hash_len {
            return Ok(false);
        }
        let mut file = fs::File::open(&self.path)?;
        let first = bytes[0] as u64;
        let lo = if first == 0 {
            0
        } else {
            u64::from(read_u32(&mut file, self.fanout + (first - 1) * 4)?)
        };
        let hi = u64::from(read_u32(&mut file, self.fanout + first * 4)?);
        let (mut lo, mut hi) = (lo, hi);
        let mut buf = vec![0u8; self.hash_len];
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            file.seek(SeekFrom::Start(self.lookup + mid * self.hash_len as u64))?;
            file.read_exact(&mut buf)?;
            match buf.as_slice().cmp(bytes) {
                std::cmp::Ordering::Equal => return Ok(true),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        Ok(false)
    }
}

/// 저장소의 가속 상태. 그래프 파일을 읽을 수 없으면(깨짐 등) 그래프가 없는 것으로 본다.
pub fn status(repo: &Repo) -> crate::repo::Result<GraphStatus> {
    let objects = repo.local().common_dir().join("objects");
    let layers = layers(&objects).unwrap_or_else(|e| {
        eprintln!("plx: commit-graph를 읽지 못했다: {e}");
        Vec::new()
    });
    let head = repo.head()?;
    let head_covered = match head {
        Some(head) => layers
            .iter()
            .any(|layer| layer.contains(&head).unwrap_or(false)),
        // 커밋이 없으면 걸을 이력도 없다.
        None => true,
    };
    Ok(GraphStatus {
        graph: !layers.is_empty(),
        commits: layers.iter().map(|l| l.commits).sum(),
        bloom: !layers.is_empty() && layers.iter().all(|l| l.bloom_version.is_some()),
        bloom_version: layers.iter().filter_map(|l| l.bloom_version).min(),
        head_covered,
        write_version: if git_supports_v2(repo.workdir()) {
            2
        } else {
            1
        },
    })
}

/// git 2.46부터 Bloom 필터 v2를 쓴다(`commitGraph.changedPathsVersion`). 버전은 앱이 떠 있는 동안 한 번만 묻는다
/// (상태를 볼 때마다 git을 띄우지 않게).
fn git_supports_v2(dir: &Path) -> bool {
    static VERSION: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
            git::output(dir, ["--version"])
                .ok()
                .and_then(|out| parse_version(&String::from_utf8_lossy(&out)))
        })
        .is_some_and(|version| version >= (2, 46))
}

/// `git version 2.43.0`, `git version 2.56.0.windows.2` → (2, 43)
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let version = text.split_whitespace().nth(2)?;
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// 쓰는 동안의 진행(git `--progress`의 단계 이름과 퍼센트)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: String,
    pub percent: Option<u32>,
}

/// commit-graph와 Bloom 필터를 쓴다(`git commit-graph write --reachable --changed-paths`).
/// 사용자가 확인한 뒤에만 부른다. 진행은 `on_progress`로 알린다.
pub fn write(
    repo: &Repo,
    mut on_progress: impl FnMut(Progress),
) -> crate::repo::Result<GraphStatus> {
    let err = |e: String| crate::repo::Error(e);
    let mut command = git::command(repo.workdir());
    if git_supports_v2(repo.workdir()) {
        command.args(["-c", "commitGraph.changedPathsVersion=2"]);
    }
    command
        .args([
            "commit-graph",
            "write",
            "--reachable",
            "--changed-paths",
            "--progress",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| err(format!("git commit-graph 실행 실패: {e}")))?;
    let mut stderr = child.stderr.take().expect("stderr");
    // 진행은 `\r`로 같은 줄을 고쳐 쓴다. 줄마다(`\r`·`\n`) 단계와 퍼센트를 읽는다.
    let mut all = Vec::new();
    let mut line = Vec::new();
    let mut buf = [0u8; 4096];
    let mut last: Option<Progress> = None;
    loop {
        let n = match stderr.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(err(format!("git commit-graph 출력 읽기 실패: {e}"))),
        };
        all.extend_from_slice(&buf[..n]);
        for &b in &buf[..n] {
            if b == b'\r' || b == b'\n' {
                if let Some(progress) = parse_progress(&String::from_utf8_lossy(&line))
                    && last.as_ref() != Some(&progress)
                {
                    on_progress(progress.clone());
                    last = Some(progress);
                }
                line.clear();
            } else {
                line.push(b);
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| err(format!("git commit-graph 실행 실패: {e}")))?;
    if !status.success() {
        let text = String::from_utf8_lossy(&all);
        let message = text
            .split(['\r', '\n'])
            .map(|l| l.trim_start_matches("fatal: ").trim())
            .rfind(|l| !l.is_empty() && parse_progress(l).is_none())
            .unwrap_or("");
        return Err(err(format!("git commit-graph 실패({status}): {message}")));
    }
    self::status(repo)
}

/// `Computing commit changed paths Bloom filters:  45% (1234/2745)` → 단계 이름과 45.
/// `Expanding reachable commits in commit graph: 82327, done.`처럼 퍼센트가 없을 수도 있다.
fn parse_progress(line: &str) -> Option<Progress> {
    let (stage, rest) = line.split_once(':')?;
    let stage = stage.trim();
    if stage.is_empty()
        || stage.starts_with("fatal")
        || stage.starts_with("warning")
        || stage.starts_with("error")
    {
        return None;
    }
    let rest = rest.trim();
    let percent = rest
        .split_whitespace()
        .find_map(|word| word.strip_suffix('%'))
        .and_then(|p| p.parse().ok());
    let numeric = rest.chars().next().is_some_and(|c| c.is_ascii_digit());
    (percent.is_some() || numeric).then(|| Progress {
        stage: stage.to_owned(),
        percent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("git version 2.43.0\n"), Some((2, 43)));
        assert_eq!(parse_version("git version 2.56.0.windows.2"), Some((2, 56)));
        assert_eq!(
            parse_version("git version 2.39.5 (Apple Git-154)"),
            Some((2, 39))
        );
        assert_eq!(parse_version("nonsense"), None);
    }

    #[test]
    fn progress_lines() {
        // git/git(커밋 8만 개)에서 본 줄들: 10초 동안 `\r`로 고쳐 쓰고 마지막에 ", done."
        assert_eq!(
            parse_progress(
                "Computing commit changed paths Bloom filters: 100% (82327/82327), done."
            ),
            Some(Progress {
                stage: "Computing commit changed paths Bloom filters".into(),
                percent: Some(100)
            })
        );
        assert_eq!(
            parse_progress("Computing commit changed paths Bloom filters:  45% (1234/2745)"),
            Some(Progress {
                stage: "Computing commit changed paths Bloom filters".into(),
                percent: Some(45)
            })
        );
        assert_eq!(
            parse_progress("Expanding reachable commits in commit graph: 82327, done."),
            Some(Progress {
                stage: "Expanding reachable commits in commit graph".into(),
                percent: None
            })
        );
        assert_eq!(parse_progress("fatal: not a git repository"), None);
        assert_eq!(parse_progress("no colon here"), None);
        assert_eq!(parse_progress("hint: something else"), None);
    }
}
