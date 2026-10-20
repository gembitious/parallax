//! Quick Open 파일 목록: 인덱스의 추적 파일(gix)을 저장소마다 기억해 두고 검색어마다 퍼지 점수(`fuzzy`)로 고른다.
//!
//! 목록은 인덱스 파일의 크기·수정 시각이 바뀔 때만 다시 읽는다(검색마다 stat 한 번). 앞 검색어를 이어 친 검색어는
//! 앞 결과 안에서만 찾고, 볼 경로가 많으면 여러 스레드로 나눈다. 같은 저장소에 새 검색이 오면 하던 검색은 그만둔다.

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use gix::bstr::ByteSlice;
use serde::Serialize;

use crate::fuzzy::{self, Matcher, Query};
pub use crate::graph::CANCELLED;
use crate::repo::Repo;
use crate::requests::lock_or_reset;

/// 한 번에 돌려주는 결과 수 한도
pub const MAX_LIMIT: usize = 1000;
/// 볼 경로가 이보다 많으면 스레드로 나눈다
const PARALLEL_MIN: usize = 8192;
/// 스레드 하나가 맡는 최소 경로 수
const PER_THREAD: usize = 4096;
/// 점수 세기 하나가 등급 가리기 몇 개만큼 무거운가(스레드 나누기 기준)
const SCORE_COST: usize = 8;
/// 이만큼 볼 때마다 새 검색이 왔는지 본다
const CHECK_EVERY: usize = 2048;
/// 기억하는 최근 연 파일 수
const RECENT: usize = 50;

/// 추적 파일 목록. 인덱스 순서(경로 바이트 순서)다.
#[derive(Default)]
pub struct FileList {
    paths: Vec<Box<str>>,
    meta: Vec<Meta>,
    /// NFD 한글을 합친 경로(번호)의 글자마다 원래 글자 시작 위치(`fuzzy::compose_jamo`). 맞은 위치를 원래
    /// 글자 위치로 돌려 화면이 자모 전부를 칠한다. 드물어 경로마다 두지 않는다.
    origins: HashMap<u32, Box<[u32]>>,
}

struct Meta {
    /// 파일 이름이 시작하는 글자 위치
    base: u32,
    /// 글자 수(같은 점수면 짧은 경로가 위)
    len: u32,
    /// ASCII가 아닌 경로의 글자 배열(ASCII는 바이트를 그대로 쓴다). NFD 한글은 합친 글자다.
    chars: Option<Box<[char]>>,
}

impl FileList {
    /// 경로(저장소 기준, `/` 구분)들로 목록을 만든다. 정렬돼 있어야 `find`가 맞다(인덱스 순서는 정렬돼 있다).
    pub fn from_paths(paths: impl IntoIterator<Item = Box<str>>) -> FileList {
        let paths: Vec<Box<str>> = paths.into_iter().collect();
        let mut origins = HashMap::new();
        let meta = paths
            .iter()
            .enumerate()
            .map(|(i, path)| {
                if path.is_ascii() {
                    return Meta {
                        base: fuzzy::base_of(path.as_bytes()) as u32,
                        len: path.len() as u32,
                        chars: None,
                    };
                }
                let mut chars: Vec<char> = path.chars().collect();
                if let Some((composed, starts)) = fuzzy::compose_jamo(&chars) {
                    chars = composed;
                    origins.insert(i as u32, starts.into_boxed_slice());
                }
                Meta {
                    base: fuzzy::base_of(&chars[..]) as u32,
                    len: chars.len() as u32,
                    chars: Some(chars.into_boxed_slice()),
                }
            })
            .collect();
        FileList {
            paths,
            meta,
            origins,
        }
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.paths.iter().map(|path| &**path)
    }

    fn find(&self, path: &str) -> Option<usize> {
        self.paths
            .binary_search_by(|p| p.as_bytes().cmp(path.as_bytes()))
            .ok()
    }

    /// `dir`(저장소 기준, 루트는 빈 문자열) 바로 아래의 폴더와 파일 이름. 인덱스 순서(바이트 순서)다.
    ///
    /// 한 폴더 아래 경로는 정렬된 목록에서 한 구간이라 접두어 구간만 보고, 하위 폴더의 경로는 그 구간 끝까지
    /// 한 번에 건너뛴다(큰 저장소의 루트도 하위 폴더 수만큼만 찾는다).
    pub fn children(&self, dir: &str) -> (Vec<&str>, Vec<&str>) {
        let dir = dir.trim_matches('/');
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let mut i = self
            .paths
            .partition_point(|p| p.as_bytes() < prefix.as_bytes());
        while let Some(rest) = self.paths.get(i).and_then(|p| p.strip_prefix(&prefix)) {
            match rest.find('/') {
                Some(slash) => {
                    let name = &rest[..slash];
                    dirs.push(name);
                    // `<prefix><name>/`로 시작하는 경로는 `<prefix><name>0`보다 앞이다('/' 다음 바이트가 '0').
                    let end = format!("{prefix}{name}0");
                    i += self.paths[i..].partition_point(|p| p.as_bytes() < end.as_bytes());
                }
                None => {
                    files.push(rest);
                    i += 1;
                }
            }
        }
        (dirs, files)
    }

    fn tier(&self, matcher: &mut Matcher, query: &Query, i: usize) -> Option<i32> {
        let meta = &self.meta[i];
        match &meta.chars {
            None => matcher.tier(query, self.paths[i].as_bytes(), meta.base as usize),
            Some(chars) => matcher.tier(query, &chars[..], meta.base as usize),
        }
    }

    fn score(
        &self,
        matcher: &mut Matcher,
        query: &Query,
        i: usize,
        positions: Option<&mut Vec<u32>>,
    ) -> Option<i32> {
        let meta = &self.meta[i];
        match &meta.chars {
            None => matcher.score(
                query,
                self.paths[i].as_bytes(),
                meta.base as usize,
                positions,
            ),
            Some(chars) => matcher.score(query, &chars[..], meta.base as usize, positions),
        }
    }

    /// `i` 경로의 결과 한 줄. `positions`는 점수를 셀 때의 글자 위치이고, NFD 한글을 합친 경로면 원래 글자
    /// 위치(합친 글자 하나 = 자모 여럿)로 돌린다.
    fn item(&self, i: usize, positions: Vec<u32>) -> FileMatch {
        let positions = match self.origins.get(&(i as u32)) {
            None => positions,
            Some(starts) => positions
                .iter()
                .flat_map(|&p| starts[p as usize]..starts[p as usize + 1])
                .collect(),
        };
        FileMatch {
            path: self.paths[i].to_string(),
            positions,
            missing: false,
        }
    }
}

/// 인덱스의 추적 파일. 작업트리에 없는 항목(sparse checkout의 skip-worktree, sparse 디렉터리)과 서브모듈은 빼고
/// 충돌 단계(1~3)는 경로 하나로 모은다. UTF-8이 아닌 경로는 열 수 없어 뺀다. 인덱스가 없으면 빈 목록이다.
pub fn tracked(repo: &Repo) -> Result<FileList, String> {
    let local = repo.local();
    if !local.index_path().exists() {
        return Ok(FileList::default());
    }
    read_index(&local)
}

fn read_index(repo: &gix::Repository) -> Result<FileList, String> {
    let index = repo
        .open_index()
        .map_err(|e| format!("인덱스를 읽지 못했다: {e}"))?;
    let mut paths: Vec<Box<str>> = Vec::with_capacity(index.entries().len());
    let mut previous: Option<&[u8]> = None;
    for entry in index.entries() {
        if entry
            .flags
            .contains(gix::index::entry::Flags::SKIP_WORKTREE)
            || entry.mode.is_sparse()
            || entry.mode.is_submodule()
        {
            continue;
        }
        let path = entry.path(&index).as_bytes();
        if previous == Some(path) {
            continue;
        }
        previous = Some(path);
        if let Ok(path) = path.to_str() {
            paths.push(path.into());
        }
    }
    Ok(FileList::from_paths(paths))
}

/// 결과 한 줄
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    /// 저장소 기준 경로
    pub path: String,
    /// 맞은 글자 위치(유니코드 코드 포인트 단위, 오름차순)
    pub positions: Vec<u32>,
    /// 인덱스에는 있지만 작업트리에 없다(지우고 아직 커밋하지 않았다). 작업트리 blame을 열 수 없다.
    pub missing: bool,
}

/// 파일 트리의 폴더 하나: 바로 아래 폴더와 파일(인덱스 순서. 화면이 이름 순으로 늘어놓는다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    /// 작업트리 경로
    pub repo: String,
    /// 저장소 기준 폴더(`/` 구분, 루트는 빈 문자열)
    pub dir: String,
    pub dirs: Vec<String>,
    pub files: Vec<DirFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirFile {
    pub name: String,
    /// 인덱스에는 있지만 작업트리에 없다(지우고 아직 커밋하지 않았다)
    pub missing: bool,
}

/// 검색 결과
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSearch {
    /// 작업트리 경로(고른 파일을 열 때 쓴다)
    pub repo: String,
    /// 추적 파일 수
    pub files: usize,
    /// 맞은 파일 수
    pub matched: usize,
    /// 점수 순(같으면 짧은 경로, 인덱스 순서) 앞부분. 빈 검색어면 최근 연 파일부터 인덱스 순서로.
    pub items: Vec<FileMatch>,
    /// 검색어 끝의 `:줄`
    pub line: Option<u32>,
    /// 이번에 목록을 (다시) 읽었으면 걸린 ms
    pub load_ms: Option<f64>,
    /// 찾는 데 걸린 ms(목록 읽기 제외)
    pub search_ms: f64,
    /// 앞 검색 결과 안에서만 찾았다
    pub narrowed: bool,
}

/// 저장소마다의 파일 목록과 검색 상태. 앱 전체가 하나를 나눠 쓴다.
#[derive(Default)]
pub struct FileLists {
    repos: Mutex<HashMap<PathBuf, Arc<RepoFiles>>>,
}

#[derive(Default)]
struct RepoFiles {
    /// 검색마다 하나씩 는다. 하던 검색은 이 값이 바뀌면 그만둔다.
    generation: AtomicU64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    loaded: Option<(Option<Stamp>, Arc<FileList>)>,
    last: Option<Last>,
    /// 최근 연 파일(앞이 최근)
    recent: VecDeque<String>,
}

/// 인덱스 파일의 지문. 없으면 `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

/// 앞 검색: 이어 친 검색어는 이 결과 안에서만 찾는다.
struct Last {
    list: Arc<FileList>,
    query: Query,
    matched: Arc<Vec<u32>>,
}

impl State {
    /// 인덱스가 바뀌었으면(처음이면) 다시 읽는다. 읽었으면 걸린 ms.
    fn refresh(&mut self, repo: &Repo) -> Result<Option<f64>, String> {
        let local = repo.local();
        let path = local.index_path();
        let stamp = match std::fs::metadata(&path) {
            Ok(meta) => Some(Stamp {
                len: meta.len(),
                modified: meta.modified().ok(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("인덱스 파일 ({}): {e}", path.display())),
        };
        if self.loaded.as_ref().is_some_and(|(old, _)| *old == stamp) {
            return Ok(None);
        }
        let started = Instant::now();
        let list = match stamp {
            Some(_) => read_index(&local)?,
            None => FileList::default(),
        };
        self.loaded = Some((stamp, Arc::new(list)));
        self.last = None;
        Ok(Some(ms(started)))
    }
}

impl FileLists {
    pub fn new() -> FileLists {
        FileLists::default()
    }

    fn files(&self, repo: &Repo) -> Arc<RepoFiles> {
        self.repos
            .lock()
            .unwrap()
            .entry(repo.workdir().to_path_buf())
            .or_default()
            .clone()
    }

    /// `raw` 검색어로 추적 파일을 찾아 점수 순 앞 `limit`개를 돌려준다. 같은 저장소에 새 검색이 오면
    /// [`CANCELLED`] 오류로 끝난다. 작업트리 절대 경로를 붙여 넣으면 저장소 기준 경로로 바꿔 찾는다.
    pub fn search(&self, repo: &Repo, raw: &str, limit: usize) -> Result<FileSearch, String> {
        let files = self.files(repo);
        let generation = files.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let stale = || files.generation.load(Ordering::Relaxed) != generation;
        let query = Query::parse(&strip_workdir(raw, repo.workdir()));
        let limit = limit.clamp(1, MAX_LIMIT);

        let (list, load_ms, previous, recent) = {
            let mut state = lock_or_reset(&files.state);
            let load_ms = state.refresh(repo)?;
            let list = state.loaded.as_ref().expect("방금 읽었다").1.clone();
            let previous = state
                .last
                .as_ref()
                .filter(|last| Arc::ptr_eq(&last.list, &list) && query.narrows(&last.query))
                .map(|last| last.matched.clone());
            let recent: Vec<String> = if query.is_empty() {
                // 검색어를 다 지웠다: 다음 검색어는 처음부터 찾는다(앞 결과를 들고 있을 까닭이 없다).
                state.last = None;
                state.recent.iter().cloned().collect()
            } else {
                Vec::new()
            };
            (list, load_ms, previous, recent)
        };

        let started = Instant::now();
        let workdir = repo.workdir().display().to_string();
        if query.is_empty() {
            let mut items = first_files(&list, &recent, limit);
            mark_missing(repo.workdir(), &mut items);
            return Ok(FileSearch {
                repo: workdir,
                files: list.len(),
                matched: list.len(),
                items,
                line: query.line,
                load_ms,
                search_ms: ms(started),
                narrowed: false,
            });
        }

        let mut found = search_list(
            &list,
            &query,
            previous.as_deref().map(Vec::as_slice),
            limit,
            &stale,
        )?;
        mark_missing(repo.workdir(), &mut found.items);
        let matched = found.matched.len();
        if !stale() {
            lock_or_reset(&files.state).last = Some(Last {
                list: list.clone(),
                query: query.clone(),
                matched: Arc::new(found.matched),
            });
        }
        Ok(FileSearch {
            repo: workdir,
            files: list.len(),
            matched,
            items: found.items,
            line: query.line,
            load_ms,
            search_ms: ms(started),
            narrowed: previous.is_some(),
        })
    }

    /// 파일 트리: `dir`(저장소 기준, 루트는 빈 문자열) 바로 아래의 추적 폴더·파일. 목록은 Quick Open과 같다
    /// (인덱스가 바뀌었으면 다시 읽는다).
    pub fn list_dir(&self, repo: &Repo, dir: &str) -> Result<DirListing, String> {
        let files = self.files(repo);
        let list = {
            let mut state = lock_or_reset(&files.state);
            state.refresh(repo)?;
            state.loaded.as_ref().expect("방금 읽었다").1.clone()
        };
        let dir = dir.trim_matches('/');
        let (dirs, names) = list.children(dir);
        let missing = missing_in_dir(&repo.workdir().join(dir), &names);
        Ok(DirListing {
            repo: repo.workdir().display().to_string(),
            dir: dir.to_owned(),
            dirs: dirs.into_iter().map(str::to_owned).collect(),
            files: names
                .into_iter()
                .zip(missing)
                .map(|(name, missing)| DirFile {
                    name: name.to_owned(),
                    missing,
                })
                .collect(),
        })
    }

    /// 최근 연 파일로 기억한다(빈 검색어일 때 앞에 보인다).
    pub fn touch(&self, repo: &Repo, path: &str) {
        let files = self.files(repo);
        let mut state = lock_or_reset(&files.state);
        state.recent.retain(|p| p != path);
        state.recent.push_front(path.to_owned());
        state.recent.truncate(RECENT);
    }
}

/// 돌려줄 결과 가운데 작업트리에 없는(지운) 파일을 표시한다. 결과 수만큼 stat한다(인덱스의 stat 정보와
/// 견주지 않는다: 지웠는지만 알면 된다). 디렉터리로 바뀐 경로도 파일로 열 수 없어 없는 것으로 본다.
fn mark_missing(workdir: &Path, items: &mut [FileMatch]) {
    for item in items {
        item.missing = missing_in_worktree(&workdir.join(&item.path));
    }
}

/// 폴더 `dir` 안의 `names` 가운데 작업트리에 없는 것. 폴더를 한 번 읽어 견주고(파일마다 열어 보지 않는다. Windows는
/// 파일 하나 여는 데도 비싸다), 이름이 그대로 맞지 않는 것(대소문자를 가리지 않는 파일 시스템, NFD로 저장한 한글
/// 이름)만 따로 확인한다.
fn missing_in_dir(dir: &Path, names: &[&str]) -> Vec<bool> {
    let mut on_disk = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            // 심볼릭 링크는 따라가지 않는다(`missing_in_worktree`의 symlink_metadata와 같다).
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            on_disk.insert(entry.file_name(), is_dir);
        }
    }
    names
        .iter()
        .map(|name| match on_disk.get(std::ffi::OsStr::new(name)) {
            Some(is_dir) => *is_dir,
            None => missing_in_worktree(&dir.join(name)),
        })
        .collect()
}

fn missing_in_worktree(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.is_dir(),
        Err(e) => matches!(
            e.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        ),
    }
}

/// 빈 검색어: 목록에 있는 최근 연 파일, 그 뒤로 인덱스 순서
fn first_files(list: &FileList, recent: &[String], limit: usize) -> Vec<FileMatch> {
    let mut seen = HashSet::new();
    let recent = recent.iter().filter_map(|path| list.find(path));
    recent
        .chain(0..list.len())
        .filter(|&i| seen.insert(i))
        .take(limit)
        .map(|i| list.item(i, Vec::new()))
        .collect()
}

/// 목록 검색 결과: 맞은 경로(인덱스 순서, 다음 좁히기용)와 점수 순 앞부분
pub struct Found {
    pub matched: Vec<u32>,
    pub items: Vec<FileMatch>,
}

/// `candidates`(없으면 목록 전체) 가운데 `query`에 맞는 경로를 찾는다. `stale`이 참이 되면 [`CANCELLED`].
///
/// 먼저 맞는 경로와 등급(파일 이름·앞부분·경로)만 가리고, 앞 `limit`개에 들 수 있는 등급의 경로만 점수를 센다
/// (등급이 다르면 점수도 언제나 그 순서다). 한두 글자 검색어는 거의 모든 경로가 맞지만 점수는 일부만 센다.
pub fn search_list(
    list: &FileList,
    query: &Query,
    candidates: Option<&[u32]>,
    limit: usize,
    stale: &(dyn Fn() -> bool + Sync),
) -> Result<Found, String> {
    let count = candidates.map_or(list.len(), <[u32]>::len);
    let ranked = parallel(count, threads_for(count), stale, |matcher, k| {
        let i = candidates.map_or(k as u32, |c| c[k]);
        list.tier(matcher, query, i as usize).map(|tier| (tier, i))
    })?;
    let matched: Vec<u32> = ranked.iter().map(|&(_, i)| i).collect();

    let cutoff = if ranked.len() > limit {
        let mut tiers: Vec<i32> = ranked.iter().map(|&(tier, _)| tier).collect();
        *tiers
            .select_nth_unstable_by_key(limit - 1, |&tier| Reverse(tier))
            .1
    } else {
        i32::MIN
    };
    let contenders: Vec<u32> = ranked
        .iter()
        .filter(|&&(tier, _)| tier >= cutoff)
        .map(|&(_, i)| i)
        .collect();
    // 점수 세기는 등급 가리기보다 훨씬 무거워 더 적은 수에서도 나눈다.
    let mut scored = parallel(
        contenders.len(),
        threads_for(contenders.len() * SCORE_COST),
        stale,
        |matcher, k| {
            let i = contenders[k];
            list.score(matcher, query, i as usize, None)
                .map(|score| (score, i))
        },
    )?;

    let key = |&(score, i): &(i32, u32)| (Reverse(score), list.meta[i as usize].len, i);
    if scored.len() > limit {
        scored.select_nth_unstable_by_key(limit - 1, key);
        scored.truncate(limit);
    }
    scored.sort_unstable_by_key(key);
    let mut matcher = Matcher::new();
    let items = scored
        .iter()
        .map(|&(_, i)| {
            let mut positions = Vec::new();
            list.score(&mut matcher, query, i as usize, Some(&mut positions));
            list.item(i as usize, positions)
        })
        .collect();
    Ok(Found { matched, items })
}

fn threads_for(count: usize) -> usize {
    if count < PARALLEL_MIN {
        return 1;
    }
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(count / PER_THREAD)
        .max(1)
}

/// `0..count`마다 `f`를 부르고 `Some` 결과를 순서대로 모은다. `threads`개 스레드로 나누고(각자 `Matcher`),
/// 중간중간 `stale`을 보아 참이면 [`CANCELLED`]로 그만둔다.
fn parallel<R: Send>(
    count: usize,
    threads: usize,
    stale: &(dyn Fn() -> bool + Sync),
    f: impl Fn(&mut Matcher, usize) -> Option<R> + Sync,
) -> Result<Vec<R>, String> {
    let run = |range: Range<usize>| -> Result<Vec<R>, String> {
        let mut matcher = Matcher::new();
        let mut out = Vec::new();
        for (n, k) in range.enumerate() {
            if n % CHECK_EVERY == 0 && stale() {
                return Err(CANCELLED.to_owned());
            }
            out.extend(f(&mut matcher, k));
        }
        Ok(out)
    };
    if threads <= 1 {
        return run(0..count);
    }
    let chunk = count.div_ceil(threads);
    let run = &run;
    let parts: Vec<Result<Vec<R>, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let range = (t * chunk).min(count)..((t + 1) * chunk).min(count);
                scope.spawn(move || run(range))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("파일 검색 스레드"))
            .collect()
    });
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.as_ref().map_or(0, Vec::len)).sum());
    for part in parts {
        out.extend(part?);
    }
    Ok(out)
}

/// 작업트리 절대 경로로 시작하는 검색어는 저장소 기준 경로로 바꾼다.
fn strip_workdir<'a>(raw: &'a str, workdir: &Path) -> Cow<'a, str> {
    let Some(root) = workdir.to_str() else {
        return Cow::Borrowed(raw);
    };
    let root = root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    let text = raw.trim_start().replace('\\', "/");
    let same = |a: &str, b: &str| {
        if cfg!(any(windows, target_os = "macos")) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    match (text.get(..root.len()), text.get(root.len()..)) {
        (Some(head), Some(rest))
            if !root.is_empty() && same(head, root) && rest.starts_with('/') =>
        {
            Cow::Owned(rest[1..].to_owned())
        }
        _ => Cow::Borrowed(raw),
    }
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(paths: &[&str]) -> FileList {
        let mut paths: Vec<Box<str>> = paths.iter().map(|&p| p.into()).collect();
        paths.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        FileList::from_paths(paths)
    }

    fn names(found: &Found) -> Vec<&str> {
        found.items.iter().map(|item| item.path.as_str()).collect()
    }

    const NEVER: &(dyn Fn() -> bool + Sync) = &|| false;

    #[test]
    fn lists_one_directory() {
        let files = list(&[
            "README.md",
            "a.txt",
            "a/b.txt",
            "a/c/d.txt",
            "a/c/e/f.txt",
            "a-b/x.txt",
            "a0.txt",
            "b/한글 파일.md",
            "src/main.rs",
        ]);
        // '-'·'.'은 '/'보다 앞이라 `a-b/`·`a.txt`가 `a/` 구간 앞에, `a0.txt`가 뒤에 온다.
        assert_eq!(
            files.children(""),
            (
                vec!["a-b", "a", "b", "src"],
                vec!["README.md", "a.txt", "a0.txt"]
            )
        );
        assert_eq!(files.children("a"), (vec!["c"], vec!["b.txt"]));
        assert_eq!(files.children("a/"), (vec!["c"], vec!["b.txt"]));
        assert_eq!(files.children("a/c"), (vec!["e"], vec!["d.txt"]));
        assert_eq!(files.children("b"), (vec![], vec!["한글 파일.md"]));
        // 파일이나 없는 폴더 아래는 비었다.
        assert_eq!(files.children("a.txt"), (vec![], vec![]));
        assert_eq!(files.children("없음"), (vec![], vec![]));
        assert_eq!(FileList::default().children(""), (vec![], vec![]));
    }

    #[test]
    fn walking_the_tree_lists_every_path_once() {
        // 서로 접두어인 이름('a', 'a-b', 'a.b', 'a0')이 많은 작은 이름 집합으로 경로를 만든다.
        const NAMES: [&str; 6] = ["a", "a-b", "a.b", "a0", "b", "한"];
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        let mut below = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..300 {
            let mut paths: Vec<String> = (0..below(30))
                .map(|_| {
                    (0..1 + below(3))
                        .map(|_| NAMES[below(NAMES.len())])
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .collect();
            paths.sort();
            paths.dedup();
            // 인덱스에는 파일과 같은 이름의 폴더가 함께 있을 수 없다.
            let all = paths.clone();
            paths.retain(|p| !all.iter().any(|q| q.starts_with(&format!("{p}/"))));
            let files = list(&paths.iter().map(String::as_str).collect::<Vec<_>>());
            let mut walked = Vec::new();
            walk(&files, "", &mut walked);
            walked.sort();
            assert_eq!(walked, paths);
        }
    }

    fn walk(files: &FileList, dir: &str, out: &mut Vec<String>) {
        let (dirs, names) = files.children(dir);
        let join = |name: &str| {
            if dir.is_empty() {
                name.to_owned()
            } else {
                format!("{dir}/{name}")
            }
        };
        out.extend(names.into_iter().map(join));
        for sub in dirs {
            walk(files, &join(sub), out);
        }
    }

    #[test]
    fn ranks_and_highlights() {
        let files = list(&[
            "src/graph/draw.ts",
            "src/graph/GraphView.tsx",
            "crates/plx-core/src/graph.rs",
            "README.md",
        ]);
        let found = search_list(&files, &Query::parse("graph"), None, 10, NEVER).unwrap();
        assert_eq!(
            names(&found),
            [
                "crates/plx-core/src/graph.rs",
                "src/graph/GraphView.tsx",
                "src/graph/draw.ts"
            ]
        );
        assert_eq!(found.items[0].positions, [20, 21, 22, 23, 24]);
        // 맞은 경로는 인덱스 순서로 남긴다(다음 좁히기)
        let order: Vec<&str> = found
            .matched
            .iter()
            .map(|&i| &*files.paths[i as usize])
            .collect();
        assert_eq!(
            order,
            [
                "crates/plx-core/src/graph.rs",
                "src/graph/GraphView.tsx",
                "src/graph/draw.ts"
            ]
        );
        assert_eq!(
            names(&search_list(&files, &Query::parse("graph"), None, 1, NEVER).unwrap()),
            ["crates/plx-core/src/graph.rs"]
        );
    }

    #[test]
    fn narrowed_search_equals_full_search() {
        let files = list(&[
            "a/bc.rs",
            "ab/c.rs",
            "abc.rs",
            "b/a/c.rs",
            "x.rs",
            "소스/파일 이름.rs",
        ]);
        let first = search_list(&files, &Query::parse("a"), None, 100, NEVER).unwrap();
        for next in ["ab", "abc", "a c", "a/c"] {
            let full = search_list(&files, &Query::parse(next), None, 100, NEVER).unwrap();
            let narrow = search_list(
                &files,
                &Query::parse(next),
                Some(&first.matched),
                100,
                NEVER,
            )
            .unwrap();
            assert_eq!(full.items, narrow.items, "{next}");
            assert_eq!(full.matched, narrow.matched, "{next}");
        }
    }

    #[test]
    fn threads_give_the_same_result() {
        let paths: Vec<String> = (0..40_000)
            .map(|i| {
                format!(
                    "dir{}/sub{}/file_{i}.{}",
                    i % 97,
                    i % 13,
                    ["rs", "ts", "md"][i % 3]
                )
            })
            .collect();
        let mut sorted: Vec<&str> = paths.iter().map(String::as_str).collect();
        sorted.sort();
        let files = list(&sorted);
        for query in ["f12", "d9 s1 rs", "file_399", "f"] {
            let q = Query::parse(query);
            let score =
                |matcher: &mut Matcher, i: usize| files.score(matcher, &q, i, None).map(|s| (s, i));
            let one = parallel(files.len(), 1, NEVER, score).unwrap();
            let many = parallel(files.len(), 4, NEVER, score).unwrap();
            assert_eq!(one, many, "{query}");
            // 등급으로 거른 앞 20개 = 모두 점수를 센 앞 20개
            let mut all = one.clone();
            all.sort_by_key(|&(s, i)| (Reverse(s), files.meta[i].len, i));
            let expected: Vec<&str> = all
                .iter()
                .take(20)
                .map(|&(_, i)| &*files.paths[i])
                .collect();
            let found = search_list(&files, &q, None, 20, NEVER).unwrap();
            assert_eq!(found.matched.len(), one.len());
            assert_eq!(names(&found), expected, "{query}");
        }
    }

    #[test]
    fn nfd_paths_match_and_highlight_every_jamo() {
        // 이름만 NFD로 저장된 경로(문서/파일.txt): 맞은 음절 하나가 자모 둘·셋이다
        let nfd = "문서/\u{1111}\u{1161}\u{110B}\u{1175}\u{11AF}.txt";
        let files = list(&[nfd, "파이.txt", "문서/파.txt"]);
        let found = search_list(&files, &Query::parse("파일"), None, 10, NEVER).unwrap();
        assert_eq!(names(&found), [nfd]);
        assert_eq!(found.items[0].positions, [3, 4, 5, 6, 7]);
        let found = search_list(&files, &Query::parse("ㅍㅇ.txt"), None, 10, NEVER).unwrap();
        let item = found.items.iter().find(|item| item.path == nfd).unwrap();
        assert_eq!(item.positions, [3, 4, 5, 6, 7, 8, 9, 10, 11]);
        // 정렬 길이도 합친 글자 수다(문서/파일.txt = 9자)
        let i = files.find(nfd).unwrap();
        assert_eq!((files.meta[i].len, files.meta[i].base), (9, 3));
    }

    #[test]
    fn stale_searches_stop() {
        let files = list(&["a.rs", "b.rs"]);
        let stop: &(dyn Fn() -> bool + Sync) = &|| true;
        assert_eq!(
            search_list(&files, &Query::parse("a"), None, 10, stop)
                .err()
                .as_deref(),
            Some(CANCELLED)
        );
    }

    #[test]
    fn empty_query_lists_recent_files_first() {
        let files = list(&["a.rs", "b.rs", "c.rs", "d.rs"]);
        let recent = ["c.rs".to_owned(), "gone.rs".to_owned(), "a.rs".to_owned()];
        let first: Vec<String> = first_files(&files, &recent, 3)
            .into_iter()
            .map(|m| m.path)
            .collect();
        assert_eq!(first, ["c.rs", "a.rs", "b.rs"]);
    }

    #[test]
    fn strips_the_worktree_prefix() {
        let root = if cfg!(windows) {
            Path::new(r"C:\work\repo")
        } else {
            Path::new("/work/repo")
        };
        let pasted = if cfg!(windows) {
            r"C:\work\repo\src\a.rs:12"
        } else {
            "/work/repo/src/a.rs:12"
        };
        assert_eq!(strip_workdir(pasted, root), "src/a.rs:12");
        assert_eq!(strip_workdir("src/a.rs", root), "src/a.rs");
        let sibling = if cfg!(windows) {
            r"C:\work\repo2\a.rs"
        } else {
            "/work/repo2/a.rs"
        };
        assert_eq!(strip_workdir(sibling, root), sibling);
    }
}
