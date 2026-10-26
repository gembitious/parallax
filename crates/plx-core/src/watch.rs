//! 저장소·파일 변화 감시: `.git`의 HEAD·refs·packed-refs, 작업트리 blame으로 연 파일이 바뀌면 알린다(M4).
//!
//! 에이전트가 다른 터미널·워크트리에서 커밋하거나 브랜치를 옮기면 열린 화면(그래프 등)이 다시 그린다.
//! 파일만 고치고 커밋하지 않아도 작업트리 blame은 바뀐 줄을 미커밋으로 다시 보인다.
//! 파일 이름으로 먼저 거른다(objects/, index, *.lock 같은 것은 무시). git은 참조를 lock 파일에 쓰고 rename하므로
//! 이벤트가 몰려 오니 [`DEBOUNCE`]만큼 조용해질 때까지 모은 뒤, HEAD와 참조의 지문이 실제로 달라졌을 때만 알린다.
//!
//! 작업트리 변경 화면([`crate::changes`])은 작업트리 전체를 재귀로 본다([`Watchers::watch_tree`]). `.git` 안은 빼고,
//! 파일 하나하나의 지문을 재지 않고 [`TREE_DEBOUNCE`]만큼 조용하면 알린다(화면이 다시 견준다).
//!
//! 링크된 워크트리는 HEAD가 `<공통>/worktrees/<이름>/HEAD`에 있고 참조는 공통 디렉터리에 있다. 둘 다 본다.
//! 그래프 이름표는 다른 워크트리가 체크아웃한 것에 달라지므로 다른 워크트리의 HEAD와 워크트리가 생기고 없어지는 것도
//! 본다(`<공통>/worktrees`를 재귀로, 아직 없으면 생길 때부터). 이것은 참조 변화로 알린다.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use notify::{EventKind, RecursiveMode, Watcher};
use serde::Serialize;

use crate::repo::{Repo, RepoCache};

/// 마지막 이벤트 뒤 이만큼 조용하면 알린다.
pub const DEBOUNCE: Duration = Duration::from_millis(120);

/// 작업트리 감시: 마지막 이벤트 뒤 이만큼 조용하면 알린다(에이전트가 파일 여럿을 잇달아 쓴다).
pub const TREE_DEBOUNCE: Duration = Duration::from_millis(300);

/// 작업트리 안의 파일이 바뀌었다(`.git` 밖)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeChange {
    /// [`Watchers::watch_tree`]가 돌려준 작업트리 경로
    pub repo: String,
}

/// 화면으로 보내는 변화
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoChange {
    /// 작업트리 경로([`Watchers::watch`]가 돌려준 값과 같다)
    pub repo: String,
    /// 지금 HEAD가 가리키는 커밋
    pub head: Option<String>,
    /// HEAD(커밋이나 가리키는 브랜치)가 바뀌었다
    pub head_changed: bool,
    /// HEAD 말고 다른 참조(브랜치·태그·원격)가 바뀌었다
    pub refs_changed: bool,
}

/// 파일 내용이 바뀌었다(크기나 수정 시각, 지워지거나 다시 생김)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// [`Watchers::watch_file`]이 돌려준 경로
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Repo(RepoChange),
    File(FileChange),
    Tree(TreeChange),
}

/// 감시 대상: 저장소(작업트리 경로) 또는 파일(canonical 경로)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Repo(PathBuf),
    File(PathBuf),
    /// 작업트리 전체(canonical 작업트리 경로)
    Tree(PathBuf),
}

/// HEAD와 참조의 지문
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    head: Option<String>,
    /// HEAD가 가리키는 참조 이름(detached면 없다)
    head_ref: Option<String>,
    refs: u64,
    /// 다른 워크트리들(id와 HEAD 파일 내용)
    others: u64,
}

fn fingerprint(repo: &Repo) -> Fingerprint {
    let local = repo.local();
    let head = repo.head().ok().flatten().map(|id| id.to_string());
    let head_ref = local
        .head_name()
        .ok()
        .flatten()
        .map(|name| name.as_bstr().to_string());
    let mut hasher = DefaultHasher::new();
    if let Ok(platform) = local.references()
        && let Ok(all) = platform.all()
    {
        for reference in all.flatten() {
            reference.name().as_bstr().hash(&mut hasher);
            match reference.target() {
                gix::refs::TargetRef::Object(id) => id.as_bytes().hash(&mut hasher),
                gix::refs::TargetRef::Symbolic(name) => name.as_bstr().hash(&mut hasher),
            }
        }
    }
    Fingerprint {
        head,
        head_ref,
        refs: hasher.finish(),
        others: other_worktrees(local.git_dir(), local.common_dir()),
    }
}

/// 다른 워크트리들의 지문: 주 작업트리의 HEAD(지금 것이 아니면)와 `<공통>/worktrees/<id>/HEAD`의 내용.
/// 워크트리가 생기거나 없어지거나 다른 브랜치·커밋을 체크아웃하면 달라진다.
fn other_worktrees(git_dir: &Path, common_dir: &Path) -> u64 {
    let canonical = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    let here = canonical(git_dir);
    let mut heads: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |dir: &Path, id: String| {
        if canonical(dir) != here {
            heads.push((id, std::fs::read(dir.join("HEAD")).unwrap_or_default()));
        }
    };
    add(common_dir, String::new());
    if let Ok(entries) = std::fs::read_dir(common_dir.join("worktrees")) {
        for entry in entries.flatten() {
            add(
                &entry.path(),
                entry.file_name().to_string_lossy().into_owned(),
            );
        }
    }
    heads.sort();
    let mut hasher = DefaultHasher::new();
    heads.hash(&mut hasher);
    hasher.finish()
}

/// 파일의 지문: 없으면 None
fn file_fingerprint(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()))
}

/// 감시하는 파일 하나(부모 디렉터리를 본다: 편집기는 임시 파일에 쓰고 rename하기도 한다)
struct WatchedFile {
    count: usize,
    last: Option<(u64, Option<SystemTime>)>,
    _watcher: notify::RecommendedWatcher,
}

/// 재귀로 보는 작업트리 하나
struct WatchedTree {
    count: usize,
    /// 작업트리 경로(알릴 때 쓴다: 화면이 받은 값)
    workdir: PathBuf,
    /// 이 아래의 이벤트는 뺀다(`.git` 디렉터리, 링크된 워크트리면 그 git 디렉터리)
    git_dirs: Vec<PathBuf>,
    _watcher: notify::RecommendedWatcher,
}

/// 감시하는 저장소 하나
struct Watched {
    repo: Arc<Repo>,
    /// worktree별 git 디렉터리(HEAD가 있는 곳)
    git_dir: PathBuf,
    /// 공통 디렉터리(refs/, packed-refs가 있는 곳)
    common_dir: PathBuf,
    /// 링크된 워크트리들의 git 디렉터리가 있는 곳(`<공통>/worktrees`)
    worktrees_dir: PathBuf,
    /// `worktrees_dir`를 재귀로 보고 있다(없던 것이 생기면 그때부터 본다)
    watching_worktrees: bool,
    /// 이 저장소를 보는 구독 수
    count: usize,
    last: Fingerprint,
    watcher: notify::RecommendedWatcher,
}

impl Watched {
    /// 이 저장소에 의미 있는 파일인가(HEAD, packed-refs, refs/ 아래, 워크트리가 생기고 없어지는 것과 그 HEAD).
    /// lock 파일은 뺀다.
    fn relevant(&self, path: &Path) -> bool {
        if path.extension().is_some_and(|e| e == "lock") {
            return false;
        }
        let name = path.file_name().and_then(|n| n.to_str());
        (path.parent() == Some(self.git_dir.as_path()) && name == Some("HEAD"))
            || (path.parent() == Some(self.common_dir.as_path())
                && matches!(name, Some("HEAD" | "packed-refs")))
            || path.starts_with(self.common_dir.join("refs"))
            || path == self.worktrees_dir
            || path.parent() == Some(self.worktrees_dir.as_path())
            || (name == Some("HEAD")
                && path.parent().and_then(Path::parent) == Some(self.worktrees_dir.as_path()))
    }

    /// `<공통>/worktrees`가 생겼으면 그때부터 재귀로 본다(첫 `git worktree add`).
    fn watch_new_worktrees(&mut self) {
        if !self.watching_worktrees
            && self.worktrees_dir.is_dir()
            && self
                .watcher
                .watch(&self.worktrees_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            self.watching_worktrees = true;
        }
    }
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    /// 구독 id → 대상
    subscriptions: HashMap<u64, Key>,
    /// 작업트리 → 감시
    repos: HashMap<PathBuf, Watched>,
    /// 파일 → 감시
    files: HashMap<PathBuf, WatchedFile>,
    /// canonical 작업트리 → 감시
    trees: HashMap<PathBuf, WatchedTree>,
}

impl Inner {
    fn relevant(&self, key: &Key, path: &Path) -> bool {
        match key {
            Key::Repo(workdir) => self.repos.get(workdir).is_some_and(|w| w.relevant(path)),
            Key::File(file) => self.files.contains_key(file) && path == file,
            Key::Tree(root) => self.trees.get(root).is_some_and(|tree| {
                path.starts_with(root) && !tree.git_dirs.iter().any(|dir| path.starts_with(dir))
            }),
        }
    }
}

/// 앱 상태로 하나 둔다. 화면이 저장소를 볼 때 [`Watchers::watch`], 떠날 때 [`Watchers::unwatch`].
pub struct Watchers {
    inner: Arc<Mutex<Inner>>,
    events: Sender<(Key, PathBuf)>,
}

impl Watchers {
    /// 변화가 있으면 다른 스레드에서 `on_change`를 부른다.
    pub fn new(on_change: impl Fn(Change) + Send + 'static) -> Watchers {
        let inner = Arc::new(Mutex::new(Inner::default()));
        let (events, rx) = mpsc::channel::<(Key, PathBuf)>();
        let state = inner.clone();
        std::thread::Builder::new()
            .name("plx-watch".to_owned())
            .spawn(move || debounce(&state, &rx, &on_change))
            .expect("감시 스레드");
        Watchers { inner, events }
    }

    /// `path`(저장소 안의 아무 경로)가 든 저장소를 본다. (구독 id, 작업트리 경로)를 돌려준다.
    /// 감시를 새로 만들 때(OS 감시 등록, 참조 지문)는 잠금 밖에서 한다(다른 구독·알림을 막지 않게).
    pub fn watch(&self, repos: &RepoCache, path: &Path) -> Result<(u64, String), String> {
        let repo = repos.for_path(path).map_err(|e| e.to_string())?;
        let workdir = repo.workdir().to_path_buf();
        let fresh = if self.inner.lock().unwrap().repos.contains_key(&workdir) {
            None
        } else {
            Some(self.start(repo)?)
        };
        let mut inner = self.inner.lock().unwrap();
        match (inner.repos.get_mut(&workdir), fresh) {
            (Some(watched), _) => watched.count += 1,
            (None, Some(watched)) => {
                inner.repos.insert(workdir.clone(), watched);
            }
            // 그사이 다른 구독이 감시를 끝냈다: 이번에는 잠금 안에서 만든다(드물다).
            (None, None) => {
                let repo = repos.for_path(path).map_err(|e| e.to_string())?;
                let watched = self.start(repo)?;
                inner.repos.insert(workdir.clone(), watched);
            }
        }
        inner.next_id += 1;
        let id = inner.next_id;
        inner.subscriptions.insert(id, Key::Repo(workdir.clone()));
        Ok((id, workdir.to_string_lossy().into_owned()))
    }

    /// 파일 하나를 본다(지금 없어도 된다: 생기면 알린다). (구독 id, canonical 경로)를 돌려준다.
    pub fn watch_file(&self, path: &Path) -> Result<(u64, String), String> {
        let parent = path
            .parent()
            .filter(|p| p.is_dir())
            .ok_or_else(|| format!("디렉터리가 없다: {}", path.display()))?;
        let name = path
            .file_name()
            .ok_or_else(|| format!("파일 이름이 없다: {}", path.display()))?;
        let parent = dunce::canonicalize(parent).map_err(|e| format!("경로: {e}"))?;
        let file = parent.join(name);
        // OS 감시는 잠금 밖에서 만든다.
        let fresh = if self.inner.lock().unwrap().files.contains_key(&file) {
            None
        } else {
            Some(self.start_file(&parent, &file)?)
        };
        let mut inner = self.inner.lock().unwrap();
        match (inner.files.get_mut(&file), fresh) {
            (Some(watched), _) => watched.count += 1,
            (None, Some(watched)) => {
                inner.files.insert(file.clone(), watched);
            }
            (None, None) => {
                let watched = self.start_file(&parent, &file)?;
                inner.files.insert(file.clone(), watched);
            }
        }
        inner.next_id += 1;
        let id = inner.next_id;
        inner.subscriptions.insert(id, Key::File(file.clone()));
        Ok((id, file.to_string_lossy().into_owned()))
    }

    /// 작업트리 전체를 재귀로 본다(`.git` 밖의 파일이 생기고 바뀌고 없어지면 알린다). (구독 id, 작업트리 경로)를
    /// 돌려준다. 작업트리 경로는 [`Watchers::watch`]와 같다.
    pub fn watch_tree(&self, repos: &RepoCache, path: &Path) -> Result<(u64, String), String> {
        let repo = repos.for_path(path).map_err(|e| e.to_string())?;
        let workdir = repo.workdir().to_path_buf();
        let root = dunce::canonicalize(&workdir).map_err(|e| format!("경로: {e}"))?;
        let fresh = if self.inner.lock().unwrap().trees.contains_key(&root) {
            None
        } else {
            Some(self.start_tree(&repo, &root)?)
        };
        let mut inner = self.inner.lock().unwrap();
        match (inner.trees.get_mut(&root), fresh) {
            (Some(watched), _) => watched.count += 1,
            (None, Some(watched)) => {
                inner.trees.insert(root.clone(), watched);
            }
            (None, None) => {
                let watched = self.start_tree(&repo, &root)?;
                inner.trees.insert(root.clone(), watched);
            }
        }
        inner.next_id += 1;
        let id = inner.next_id;
        inner.subscriptions.insert(id, Key::Tree(root));
        Ok((id, workdir.to_string_lossy().into_owned()))
    }

    fn start_tree(&self, repo: &Repo, root: &Path) -> Result<WatchedTree, String> {
        let local = repo.local();
        let canonical = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_owned());
        let events = self.events.clone();
        let key = Key::Tree(root.to_path_buf());
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                for path in event.paths {
                    let _ = events.send((key.clone(), path));
                }
            })
            .map_err(|e| format!("작업트리 감시 실패: {e}"))?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|e| format!("작업트리 감시 실패({}): {e}", root.display()))?;
        Ok(WatchedTree {
            count: 1,
            workdir: repo.workdir().to_path_buf(),
            git_dirs: vec![
                root.join(".git"),
                canonical(local.git_dir()),
                canonical(local.common_dir()),
            ],
            _watcher: watcher,
        })
    }

    /// 구독을 끝낸다. 그 저장소를 보는 구독이 없으면 감시를 멈춘다. 모르는 id면 `false`.
    /// OS 감시는 잠금 밖에서 버린다(macOS FSEvents는 버릴 때 스레드를 기다린다).
    pub fn unwatch(&self, id: u64) -> bool {
        let mut stopped_repo = None;
        let mut stopped_file = None;
        let mut stopped_tree = None;
        let known = {
            let mut inner = self.inner.lock().unwrap();
            match inner.subscriptions.remove(&id) {
                Some(Key::Repo(workdir)) => {
                    if let Some(watched) = inner.repos.get_mut(&workdir) {
                        watched.count -= 1;
                        if watched.count == 0 {
                            stopped_repo = inner.repos.remove(&workdir);
                        }
                    }
                    true
                }
                Some(Key::File(file)) => {
                    if let Some(watched) = inner.files.get_mut(&file) {
                        watched.count -= 1;
                        if watched.count == 0 {
                            stopped_file = inner.files.remove(&file);
                        }
                    }
                    true
                }
                Some(Key::Tree(root)) => {
                    if let Some(watched) = inner.trees.get_mut(&root) {
                        watched.count -= 1;
                        if watched.count == 0 {
                            stopped_tree = inner.trees.remove(&root);
                        }
                    }
                    true
                }
                None => false,
            }
        };
        drop(stopped_repo);
        drop(stopped_file);
        drop(stopped_tree);
        known
    }

    fn start_file(&self, parent: &Path, file: &Path) -> Result<WatchedFile, String> {
        let events = self.events.clone();
        let key = Key::File(file.to_path_buf());
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                for path in event.paths {
                    let _ = events.send((key.clone(), path));
                }
            })
            .map_err(|e| format!("파일 감시 실패: {e}"))?;
        watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .map_err(|e| format!("파일 감시 실패({}): {e}", parent.display()))?;
        Ok(WatchedFile {
            count: 1,
            last: file_fingerprint(file),
            _watcher: watcher,
        })
    }

    /// 감시 중인 저장소·파일 수
    pub fn watching(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.repos.len() + inner.files.len() + inner.trees.len()
    }

    fn start(&self, repo: Arc<Repo>) -> Result<Watched, String> {
        let local = repo.local();
        let canonical = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_owned());
        let git_dir = canonical(local.git_dir());
        let common_dir = canonical(local.common_dir());
        let key = Key::Repo(repo.workdir().to_path_buf());
        let events = self.events.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                for path in event.paths {
                    let _ = events.send((key.clone(), path));
                }
            })
            .map_err(|e| format!("저장소 감시 실패: {e}"))?;
        let watch = |watcher: &mut notify::RecommendedWatcher, path: &Path, mode| {
            watcher
                .watch(path, mode)
                .map_err(|e| format!("저장소 감시 실패({}): {e}", path.display()))
        };
        watch(&mut watcher, &git_dir, RecursiveMode::NonRecursive)?;
        if common_dir != git_dir {
            watch(&mut watcher, &common_dir, RecursiveMode::NonRecursive)?;
        }
        let refs = common_dir.join("refs");
        if refs.is_dir() {
            watch(&mut watcher, &refs, RecursiveMode::Recursive)?;
        }
        let last = fingerprint(&repo);
        let mut watched = Watched {
            repo,
            git_dir,
            worktrees_dir: common_dir.join("worktrees"),
            common_dir,
            watching_worktrees: false,
            count: 1,
            last,
            watcher,
        };
        watched.watch_new_worktrees();
        Ok(watched)
    }
}

/// 이벤트를 모아 [`DEBOUNCE`]만큼 조용하면 지문을 다시 재고, 달라졌으면 알린다.
fn debounce(state: &Mutex<Inner>, rx: &mpsc::Receiver<(Key, PathBuf)>, on_change: &dyn Fn(Change)) {
    // 대상 → 마지막으로 의미 있는 이벤트를 받은 때
    let mut dirty: HashMap<Key, Instant> = HashMap::new();
    loop {
        let timeout = dirty
            .iter()
            .map(|(key, at)| (*at + delay(key)).saturating_duration_since(Instant::now()))
            .min()
            .unwrap_or(Duration::from_secs(3600));
        match rx.recv_timeout(timeout) {
            Ok((key, path)) => {
                let path = canonical_parent(&path);
                if state.lock().unwrap().relevant(&key, &path) {
                    dirty.insert(key, Instant::now());
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let ready: Vec<Key> = dirty
            .iter()
            .filter(|(key, at)| now.duration_since(**at) >= delay(key))
            .map(|(key, _)| key.clone())
            .collect();
        for key in ready {
            dirty.remove(&key);
            let change = match &key {
                Key::Repo(workdir) => repo_change(state, workdir).map(Change::Repo),
                Key::File(file) => file_change(state, file).map(Change::File),
                Key::Tree(root) => state.lock().unwrap().trees.get(root).map(|tree| {
                    Change::Tree(TreeChange {
                        repo: tree.workdir.to_string_lossy().into_owned(),
                    })
                }),
            };
            if let Some(change) = change {
                on_change(change);
            }
        }
    }
}

fn delay(key: &Key) -> Duration {
    match key {
        Key::Tree(_) => TREE_DEBOUNCE,
        _ => DEBOUNCE,
    }
}

/// 저장소의 지문이 달라졌으면 그 변화
fn repo_change(state: &Mutex<Inner>, workdir: &Path) -> Option<RepoChange> {
    // 지문은 잠금 밖에서 잰다(참조 수천 개면 몇 ms).
    let repo = state.lock().unwrap().repos.get(workdir)?.repo.clone();
    let now = fingerprint(&repo);
    let mut inner = state.lock().unwrap();
    let watched = inner.repos.get_mut(workdir)?;
    watched.watch_new_worktrees();
    let head_changed = now.head != watched.last.head || now.head_ref != watched.last.head_ref;
    // 다른 워크트리의 HEAD는 그래프 이름표(체크아웃한 워크트리)만 바꾼다: 참조 변화로 알린다.
    let refs_changed = now.refs != watched.last.refs || now.others != watched.last.others;
    if !head_changed && !refs_changed {
        return None;
    }
    watched.last = now.clone();
    Some(RepoChange {
        repo: workdir.to_string_lossy().into_owned(),
        head: now.head,
        head_changed,
        refs_changed,
    })
}

/// 파일의 크기·수정 시각(또는 있고 없음)이 달라졌으면 그 변화
fn file_change(state: &Mutex<Inner>, file: &Path) -> Option<FileChange> {
    let now = file_fingerprint(file);
    let mut inner = state.lock().unwrap();
    let watched = inner.files.get_mut(file)?;
    if watched.last == now {
        return None;
    }
    watched.last = now;
    Some(FileChange {
        path: file.to_string_lossy().into_owned(),
    })
}

/// 이벤트 경로의 부모 디렉터리를 canonicalize한다(macOS의 /private/var 같은 별칭, 지운 파일도 부모로 맞춘다).
fn canonical_parent(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => dunce::canonicalize(parent)
            .map(|p| p.join(name))
            .unwrap_or_else(|_| path.to_owned()),
        _ => path.to_owned(),
    }
}
