//! "가속 설정": commit-graph와 changed-path Bloom 필터가 있는지 보고, 사용자가 확인하면 만든다.
//!
//! 파일·라인 히스토리(`git log -- <경로>`, `git log -L`)는 Bloom 필터가 있으면 커밋마다 트리를 비교하지 않아
//! 몇 배 빨라진다(bench/results.md). 저장소를 바꾸는 유일한 동작이다(CLAUDE.md): `.git/objects/info` 아래에
//! commit-graph 파일만 쓰고 커밋·참조·작업트리는 건드리지 않는다.
//!
//! 상태는 git이 보는 대로 본다(commit-graph.c `prepare_commit_graph`).
//! - git이 commit-graph를 아예 쓰지 않는 저장소: `core.commitGraph=false`, 얕은 클론, `info/grafts`, `git replace`로
//!   바꾼 객체(commit_graph_compatible). git은 쓰기도 조용히 건너뛰므로 권하지 않는다.
//! - 객체 디렉터리마다(저장소의 것, 다음 alternates) `info/commit-graph` 하나를 먼저, 읽지 못하면
//!   `info/commit-graphs/commit-graph-chain`에 적힌 층들(층 파일은 모든 객체 디렉터리에서 찾는다)을 보고, 처음 읽은
//!   그래프를 쓴다.
//! - 파일 형식(gitformat-commit-graph): `CGPH`, 버전, 해시 버전, 청크 수, 기반 그래프 수, 청크 표(ID 4바이트 + 위치
//!   8바이트, 청크 수 + 1개). 커밋이 그래프에 있는지는 `OIDF`(fanout)·`OIDL`(정렬된 id)로 찾는다.
//! - Bloom 필터는 `BIDX`·`BDAT` 청크이고 `BDAT` 머리 12바이트가 (버전, 해시 수, 항목당 비트)다. git은 맨 위 층과
//!   머리가 다른 층의 필터를 버린다(validate_mixed_bloom_settings). 모든 층에 같은 필터가 있어야 가속됐다고 본다.
//!
//! 필터 버전은 git에 맡긴다(`commitGraph.changedPathsVersion`, 기본은 그래프에 있는 버전, 없으면 v1). v2(git 2.46)는
//! 0x80 이상 바이트(한글 등)가 든 경로의 해시를 `char` 부호와 무관하게 만들지만, git 2.39~2.45는 v2 그래프를 다시
//! 쓰다(gc, `fetch.writeCommitGraph`) 비정상 종료한다. Finder로 띄운 macOS 앱의 `/usr/bin/git`(Apple Git 2.39),
//! Ubuntu 24.04(2.43)처럼 흔한 git이라 v2를 강제하지 않는다. v1 해시는 `char`가 부호 있는 플랫폼(x86-64, Windows·
//! macOS의 ARM64)끼리 같다. 그래프에 v2 필터가 있는데 이 git이 2.46보다 낮으면 쓰지 않는다.
//!
//! 쓰기는 `GIT_NO_LAZY_FETCH=1`로 한다(git 2.45부터): 부분 클론에서 받지 않은 객체를 받아 오지 않는다. 트리를 받지
//! 않은 부분 클론(`--filter=tree:0` 등)은 필터를 만들려면 커밋마다 트리를 받아 와야 하므로 권하지 않는다.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::SystemTime;

use gix::ObjectId;
use gix::bstr::ByteSlice;
use serde::Serialize;

use crate::git;
use crate::repo::Repo;

/// 저장소의 가속 상태
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphStatus {
    /// git이 읽는 commit-graph가 있다
    pub graph: bool,
    /// 그래프(모든 층)에 든 커밋 수
    pub commits: u64,
    /// git이 쓰는 changed-path Bloom 필터가 모든 층에 있다
    pub bloom: bool,
    /// 그래프의 Bloom 필터 버전(맨 위 층)
    pub bloom_version: Option<u32>,
    /// HEAD 커밋이 그래프에 있다(없으면 그 뒤 커밋은 필터 없이 걷는다: 다시 쓰면 좋다)
    pub head_covered: bool,
    /// 지금 쓰면 git이 만들 필터 버전
    pub write_version: u32,
    /// 가속 설정을 할 수 없거나 해도 소용없는 까닭. 있으면 권하지 않는다.
    pub blocked: Option<String>,
    /// 이 앱이 지금 이 저장소에 쓰고 있다
    pub writing: bool,
}

impl GraphStatus {
    /// git이 Bloom 필터로 히스토리를 걷는다
    pub fn accelerated(&self) -> bool {
        self.graph && self.bloom
    }
}

/// commit-graph 층 하나
#[derive(Debug, Clone, PartialEq, Eq)]
struct Layer {
    commits: u64,
    /// `BDAT` 머리(버전, 해시 수, 항목당 비트). `BIDX`와 `BDAT`가 다 있어야 필터다.
    bloom: Option<[u8; 12]>,
    /// 커밋 id 길이(SHA-1 20, SHA-256 32)
    hash_len: usize,
    /// OIDF 청크 위치(256 × 4바이트), OIDL 청크 위치
    fanout: u64,
    lookup: u64,
    path: PathBuf,
}

/// Bloom 필터 머리의 버전
fn bloom_version(header: &[u8; 12]) -> u32 {
    u32::from_be_bytes(header[..4].try_into().expect("4바이트"))
}

/// git이 읽을 그래프의 층들(기반 층부터). `dirs`는 객체 디렉터리(저장소의 것, 다음 alternates). 없으면 빈 목록.
fn load_graph(dirs: &[PathBuf]) -> Vec<Layer> {
    for dir in dirs {
        let info = dir.join("info");
        if let Some(layer) = try_layer(&info.join("commit-graph")) {
            return vec![layer];
        }
        let Ok(chain) = fs::read_to_string(info.join("commit-graphs").join("commit-graph-chain"))
        else {
            continue;
        };
        let mut layers = Vec::new();
        for hash in chain.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let name = format!("graph-{hash}.graph");
            let layer = dirs
                .iter()
                .find_map(|d| try_layer(&d.join("info").join("commit-graphs").join(&name)));
            // 층 하나를 읽지 못하면 git은 그 아래 층까지만 쓴다.
            match layer {
                Some(layer) => layers.push(layer),
                None => break,
            }
        }
        if !layers.is_empty() {
            return layers;
        }
    }
    Vec::new()
}

/// 층 파일을 읽는다. 없으면 조용히, 깨졌으면 알리고 `None`(git도 그 파일을 건너뛴다).
fn try_layer(path: &Path) -> Option<Layer> {
    match read_layer(path) {
        Ok(layer) => Some(layer),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("plx: commit-graph를 읽지 못했다({}): {e}", path.display());
            None
        }
    }
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
    let entries: Vec<([u8; 4], u64)> = table
        .chunks_exact(12)
        .map(|entry| {
            (
                entry[..4].try_into().expect("4바이트"),
                u64::from_be_bytes(entry[4..12].try_into().expect("8바이트")),
            )
        })
        .collect();
    // 청크 위치와 크기(다음 항목의 위치까지. 마지막 항목은 끝 표시다)
    let chunk = |id: &[u8; 4]| -> Option<(u64, u64)> {
        let i = entries[..chunks]
            .iter()
            .position(|(entry, _)| entry == id)?;
        let (start, end) = (entries[i].1, entries[i + 1].1);
        Some((start, end.saturating_sub(start)))
    };
    let (fanout, _) = chunk(b"OIDF").ok_or_else(|| bad("OIDF 청크가 없다"))?;
    let (lookup, _) = chunk(b"OIDL").ok_or_else(|| bad("OIDL 청크가 없다"))?;
    let commits = u64::from(read_u32(&mut file, fanout + 255 * 4)?);
    let bloom = match (chunk(b"BIDX"), chunk(b"BDAT")) {
        // git은 머리보다 작은 BDAT를 무시한다.
        (Some(_), Some((data, size))) if size >= 12 => {
            let mut header = [0u8; 12];
            file.seek(SeekFrom::Start(data))?;
            file.read_exact(&mut header)?;
            Some(header)
        }
        _ => None,
    };
    Ok(Layer {
        commits,
        bloom,
        hash_len,
        fanout,
        lookup,
        path: path.to_owned(),
    })
}

fn read_u32(file: &mut fs::File, offset: u64) -> io::Result<u32> {
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

impl Layer {
    /// 이 층에 `id` 커밋이 있는가(fanout으로 범위를 좁히고 정렬된 id를 이진 탐색한다).
    fn contains(&self, id: &ObjectId) -> io::Result<bool> {
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

/// 객체 디렉터리: 저장소의 것, 다음 alternates(git이 찾는 순서). alternates를 읽지 못하면 저장소의 것만.
fn object_dirs(repo: &Repo, local: &gix::Repository) -> Vec<PathBuf> {
    let objects = local.common_dir().join("objects");
    let mut dirs = vec![objects.clone()];
    match gix::odb::alternate::resolve(objects, repo.workdir()) {
        Ok(alternates) => dirs.extend(alternates),
        Err(e) => eprintln!("plx: alternates를 읽지 못했다: {e}"),
    }
    dirs
}

/// git(`git --version`)의 (주, 부) 버전. 앱이 떠 있는 동안 한 번만 묻는다(상태를 볼 때마다 git을 띄우지 않게).
/// 모르면 (0, 0): 가장 오래된 git으로 본다.
fn git_version(dir: &Path) -> (u32, u32) {
    static VERSION: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
            git::output(dir, ["--version"])
                .ok()
                .and_then(|out| parse_version(&String::from_utf8_lossy(&out)))
        })
        .unwrap_or((0, 0))
}

/// `git version 2.43.0`, `git version 2.56.0.windows.2` → (2, 43)
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let version = text.split_whitespace().nth(2)?;
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// `commitGraph.changedPathsVersion`이 생긴 git(그 전 git은 v1만 읽고 쓴다)
const V2_GIT: (u32, u32) = (2, 46);

/// git이 Bloom 필터를 읽고 쓰는 규칙(repo-settings.c, bloom.c `get_bloom_filter`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Versions {
    /// 그래프에 있는 버전을 읽고, 쓸 때도 그 버전(없으면 v1)이다(-1, 기본값)
    Existing,
    /// 이 버전만 읽고 쓴다
    Only(u32),
    /// 필터를 읽지 않는다(0, 또는 `commitGraph.readChangedPaths=false`)
    Off,
    /// git이 모르는 값이라 쓰지 않는다
    Unknown(i64),
}

fn versions(config: &gix::config::Snapshot<'_>, git: (u32, u32)) -> Versions {
    let read = config.boolean("commitGraph.readChangedPaths") != Some(false);
    if git < V2_GIT {
        return if read {
            Versions::Only(1)
        } else {
            Versions::Off
        };
    }
    match config.integer("commitGraph.changedPathsVersion") {
        None if read => Versions::Existing,
        None | Some(0) => Versions::Off,
        Some(-1) => Versions::Existing,
        Some(v @ (1 | 2)) => Versions::Only(v as u32),
        Some(v) => Versions::Unknown(v),
    }
}

/// git이 commit-graph를 쓰지 않는 까닭(prepare_commit_graph, commit_graph_compatible)
fn ignored_reason(repo: &Repo, local: &gix::Repository) -> Option<String> {
    if local.config_snapshot().boolean("core.commitGraph") == Some(false) {
        return Some("core.commitGraph=false라 git이 commit-graph를 쓰지 않는다.".to_owned());
    }
    if repo.is_shallow() {
        return Some(
            "얕은 클론이라 git이 commit-graph를 쓰지 않는다(git fetch --unshallow로 이력을 모두 받으면 쓴다)."
                .to_owned(),
        );
    }
    if !crate::repo::read_grafts(local.common_dir()).is_empty() {
        return Some("info/grafts가 있어 git이 commit-graph를 쓰지 않는다.".to_owned());
    }
    if crate::repo::has_replace_refs(local) {
        return Some("git replace로 바꾼 객체가 있어 git이 commit-graph를 쓰지 않는다.".to_owned());
    }
    None
}

/// 트리를 받지 않은 부분 클론의 필터(원격 이름, 필터). Bloom 필터를 만들려면 커밋마다 트리를 받아 와야 한다.
fn tree_filter(config: &gix::config::Snapshot<'_>) -> Option<(String, String)> {
    config
        .plumbing()
        .sections_by_name("remote")?
        .find_map(|section| {
            let spec = section.body().value("partialclonefilter")?;
            let spec = spec.to_str_lossy().into_owned();
            omits_trees(&spec).then(|| {
                let name = section
                    .header()
                    .subsection_name()
                    .map(|n| n.to_str_lossy().into_owned())
                    .unwrap_or_default();
                (name, spec)
            })
        })
}

/// 필터(`git clone --filter`)가 트리를 빼는가: `tree:<깊이>`, 트리가 아닌 `object:type=`. `combine:`은 `+`로 나눈
/// 필터마다 %-인코딩돼 있다.
fn omits_trees(spec: &str) -> bool {
    let spec = spec.trim();
    let parts: Vec<String> = match spec.strip_prefix("combine:") {
        Some(rest) => rest.split('+').map(percent_decode).collect(),
        None => vec![spec.to_owned()],
    };
    parts.iter().any(|part| {
        part.starts_with("tree:")
            || (part.starts_with("object:type=") && part != "object:type=tree")
    })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = text.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 저장소의 가속 상태. 그래프 파일을 읽을 수 없으면(깨짐 등) 그 파일이 없는 것으로 본다(git도 그렇다).
pub fn status(repo: &Repo) -> crate::repo::Result<GraphStatus> {
    let local = repo.local();
    let layers = load_graph(&object_dirs(repo, &local));
    let git = git_version(repo.workdir());
    let config = local.config_snapshot();
    let head_covered = match repo.head()? {
        Some(head) => layers
            .iter()
            .any(|layer| layer.contains(&head).unwrap_or(false)),
        // 커밋이 없으면 걸을 이력도 없다.
        None => true,
    };
    let ignored = ignored_reason(repo, &local);
    let versions = versions(&config, git);
    // git이 기준으로 삼는 필터 설정은 맨 위 층(그 위에서 처음 필터가 있는 층)의 것이다.
    let top = layers.iter().rev().find_map(|layer| layer.bloom);
    let same_filters = !layers.is_empty() && layers.iter().all(|layer| layer.bloom == top);
    let readable = top.is_some_and(|header| match versions {
        Versions::Existing => true,
        Versions::Only(v) => bloom_version(&header) == v,
        Versions::Off | Versions::Unknown(_) => false,
    });
    let write_version = match versions {
        Versions::Existing => top.map_or(1, |header| bloom_version(&header)),
        Versions::Only(v) => v,
        Versions::Off | Versions::Unknown(_) => 1,
    };
    let write_version = if write_version == 2 { 2 } else { 1 };
    let v2_layers = layers
        .iter()
        .any(|layer| layer.bloom.is_some_and(|h| bloom_version(&h) == 2));
    let blocked = ignored.clone().or_else(|| {
        let (major, minor) = git;
        match versions {
            Versions::Off => {
                return Some(
                    "commitGraph.changedPathsVersion=0(또는 commitGraph.readChangedPaths=false)이라 git이 Bloom 필터를 읽지 않는다."
                        .to_owned(),
                );
            }
            Versions::Unknown(v) => {
                return Some(format!(
                    "commitGraph.changedPathsVersion={v}은 git이 모르는 값이라 commit-graph를 쓰지 않는다."
                ));
            }
            Versions::Existing | Versions::Only(_) => {}
        }
        if v2_layers && git < V2_GIT {
            return Some(format!(
                "commit-graph의 Bloom 필터가 v2인데 이 git({major}.{minor})은 v2를 모른다(다시 쓰면 git이 비정상 종료한다). git 2.46 이상에서 쓴다."
            ));
        }
        tree_filter(&config).map(|(remote, spec)| {
            format!(
                "트리를 받지 않은 부분 클론(remote.{remote}.partialclonefilter={spec})이라 필터를 만들려면 커밋마다 트리를 받아 와야 한다."
            )
        })
    });
    Ok(GraphStatus {
        graph: ignored.is_none() && !layers.is_empty(),
        commits: layers.iter().map(|l| l.commits).sum(),
        bloom: ignored.is_none() && same_filters && readable,
        bloom_version: top.map(|header| bloom_version(&header)),
        head_covered,
        write_version,
        blocked,
        writing: Writing::active(&writing_key(&local)),
    })
}

/// 이 앱이 지금 쓰고 있는 저장소(공통 디렉터리). 두 화면(히스토리·그래프)에서 눌러도 한 번만 쓴다.
static WRITING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// 쓰는 동안 [`WRITING`]에 저장소를 둔다.
struct Writing(PathBuf);

impl Writing {
    fn start(key: PathBuf) -> Option<Writing> {
        let mut writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
        if writing.contains(&key) {
            return None;
        }
        writing.push(key.clone());
        Some(Writing(key))
    }

    fn active(key: &Path) -> bool {
        WRITING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|k| k == key)
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        WRITING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|k| k != &self.0);
    }
}

/// 쓰기를 구분하는 열쇠: 공통 디렉터리의 실제 경로(워크트리끼리 그래프를 나눠 쓴다)
fn writing_key(local: &gix::Repository) -> PathBuf {
    let common = local.common_dir();
    fs::canonicalize(common).unwrap_or_else(|_| common.to_owned())
}

/// 저장소의 commit-graph 파일들(git이 쓰는 곳: 저장소의 `objects/info`)의 크기·수정 시각. 쓰기 앞뒤로 견줘
/// git이 성공으로 끝났는데 파일이 그대로인지(바꾸지 못했는지) 본다.
fn graph_files_stamp(local: &gix::Repository) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
    let info = local.common_dir().join("objects").join("info");
    let mut paths = vec![info.join("commit-graph")];
    if let Ok(entries) = fs::read_dir(info.join("commit-graphs")) {
        paths.extend(entries.flatten().map(|entry| entry.path()));
    }
    let mut stamp: Vec<_> = paths
        .into_iter()
        .filter_map(|path| {
            let meta = fs::metadata(&path).ok()?;
            Some((path, meta.len(), meta.modified().ok()))
        })
        .collect();
    stamp.sort();
    stamp
}

/// 쓰는 동안의 진행(git `--progress`의 단계 이름과 퍼센트)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: String,
    pub percent: Option<u32>,
}

/// commit-graph와 Bloom 필터를 쓴다(`git commit-graph write --reachable --changed-paths`).
/// 사용자가 확인한 뒤에만 부른다. 진행은 `on_progress`로 알린다. git이 쓰지 않았으면(성공으로 끝나도) 오류다.
pub fn write(
    repo: &Repo,
    mut on_progress: impl FnMut(Progress),
) -> crate::repo::Result<GraphStatus> {
    let err = |e: String| crate::repo::Error(e);
    let local = repo.local();
    let _writing = Writing::start(writing_key(&local))
        .ok_or_else(|| err("이 저장소의 가속 설정을 이미 하고 있다.".to_owned()))?;
    if let Some(reason) = status(repo)?.blocked {
        return Err(err(reason));
    }
    let before = graph_files_stamp(&local);
    let mut command = git::command(repo.workdir());
    command
        .args([
            "commit-graph",
            "write",
            "--reachable",
            "--changed-paths",
            "--progress",
        ])
        // 부분 클론에서 받지 않은 객체를 받아 오지 않는다(받아야 하면 git이 실패한다).
        .env("GIT_NO_LAZY_FETCH", "1")
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
    let exit = child
        .wait()
        .map_err(|e| err(format!("git commit-graph 실행 실패: {e}")))?;
    let stderr = String::from_utf8_lossy(&all);
    if !exit.success() {
        return Err(err(failure_message(exit, &stderr)));
    }
    let mut after = status(repo)?;
    after.writing = false;
    // git은 쓸 수 없는 저장소(얕은 클론 등)에서 경고만 하고, 새 파일로 바꾸지 못해도(Windows에서 다른 프로그램이
    // 열어 둔 파일) 성공으로 끝난다.
    let warning = stderr
        .split(['\r', '\n'])
        .filter_map(|l| l.trim().strip_prefix("warning: "))
        .next_back()
        .map(|w| format!(" (git: {w})"))
        .unwrap_or_default();
    if !after.accelerated() {
        let why = after
            .blocked
            .clone()
            .unwrap_or_else(|| "Bloom 필터가 든 commit-graph가 없다.".to_owned());
        return Err(err(format!("git이 끝났지만 {why}{warning}")));
    }
    if graph_files_stamp(&local) == before {
        return Err(err(format!(
            "git이 끝났지만 commit-graph 파일이 그대로다(다른 프로그램이 파일을 열고 있으면 바꾸지 못한다).{warning}"
        )));
    }
    Ok(after)
}

/// 실패한 git의 오류 문장. git의 오류 줄(`fatal:`·`error:`)을 먼저 쓴다: 잠금 파일 충돌은 마지막 줄이
/// "remove the file manually to continue."라 파일 이름이 앞 줄에만 있다. 시그널로 끝나면 출력이 없을 수 있다.
fn failure_message(exit: ExitStatus, stderr: &str) -> String {
    let lines = || {
        stderr
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|l| !l.is_empty())
    };
    let message = lines()
        .find_map(|l| {
            l.strip_prefix("fatal: ")
                .or_else(|| l.strip_prefix("error: "))
        })
        .or_else(|| lines().rfind(|l| parse_progress(l).is_none()));
    match message {
        Some(message) => format!("git commit-graph 실패({exit}): {message}"),
        None => format!("git commit-graph가 비정상 종료했다({exit})."),
    }
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
    fn versions_of_git() {
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

    #[test]
    fn filters_that_omit_trees() {
        assert!(omits_trees("tree:0"));
        assert!(omits_trees("tree:1"));
        assert!(omits_trees("combine:tree%3A0+blob%3Anone"));
        assert!(omits_trees("combine:blob%3Anone+tree%3a2"));
        assert!(omits_trees("object:type=commit"));
        assert!(!omits_trees("object:type=tree"));
        assert!(!omits_trees("blob:none"));
        assert!(!omits_trees("blob:limit=1m"));
        assert!(!omits_trees("sparse:oid=main:.sparse"));
        assert!(!omits_trees("combine:blob%3Anone+blob%3Alimit%3D1k"));
    }

    #[cfg(unix)]
    #[test]
    fn failure_messages_prefer_git_errors() {
        use std::os::unix::process::ExitStatusExt;

        // 잠금 파일 충돌: 마지막 줄에는 파일 이름이 없다.
        let lock = "fatal: Unable to create '/r/.git/objects/info/commit-graphs/commit-graph-chain.lock': File exists.\n\nAnother git process seems to be running in this repository, e.g.\nremove the file manually to continue.\n";
        let message = failure_message(ExitStatus::from_raw(128 << 8), lock);
        assert!(
            message.contains("commit-graph-chain.lock': File exists."),
            "{message}"
        );
        // 시그널로 끝나 출력이 없다(git 2.39~2.45가 v2 그래프를 다시 쓸 때).
        let crashed = failure_message(ExitStatus::from_raw(11), "");
        assert!(crashed.contains("비정상 종료"), "{crashed}");
        assert!(crashed.contains("SIGSEGV"), "{crashed}");
        // 오류 줄이 없으면 진행이 아닌 마지막 줄
        let other = failure_message(
            ExitStatus::from_raw(1 << 8),
            "Expanding reachable commits in commit graph: 5, done.\nsomething odd\n",
        );
        assert!(other.ends_with("something odd"), "{other}");
    }

    #[test]
    fn one_write_per_repository() {
        let key = PathBuf::from("/저장소/.git");
        let first = Writing::start(key.clone()).expect("처음은 쓴다");
        assert!(Writing::active(&key));
        assert!(
            Writing::start(key.clone()).is_none(),
            "쓰는 동안 또 쓰지 않는다"
        );
        assert!(Writing::start(PathBuf::from("/다른/.git")).is_some());
        drop(first);
        assert!(!Writing::active(&key));
        assert!(Writing::start(key).is_some(), "끝나면 다시 쓸 수 있다");
    }
}
