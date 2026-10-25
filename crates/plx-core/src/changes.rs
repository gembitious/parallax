//! 두 지점의 비교: 리비전 ↔ 작업트리(아직 커밋하지 않은 변경), 리비전 ↔ 리비전(M7).
//!
//! 화면은 커밋 상세와 같다(파일 목록, 줄 수, 고른 파일의 두 버전). 리비전끼리는 두 트리를 견주고, 작업트리는
//! gix status로 바뀌었을 수 있는 경로(기준 트리 ↔ 인덱스 ↔ 작업트리, 추적하지 않는 파일)를 모은 뒤 그 경로마다
//! 기준 트리의 항목과 작업트리 파일을 git처럼(clean 변환 뒤 blob 해시) 견준다. 인덱스에 올린 것과 아닌 것을 나누지
//! 않는다(`git diff <기준>`에 추적하지 않는 파일을 더한 것). 하위 모듈은 보지 않는다.
//!
//! 저장소를 바꾸지 않는다: 작업트리 파일은 해시만 하고 객체로 쓰지 않으며, status가 고친 인덱스 stat도 쓰지 않는다.
//! rename은 커밋 상세와 같은 단계·기준으로 찾는다([`crate::renames`], 작업트리 파일은 읽은 내용으로 견준다).

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use gix::ObjectId;
use gix::bstr::{BString, ByteSlice};
use gix::object::tree::{EntryKind, EntryMode};
use serde::Serialize;

use crate::commit::{FileChange, StatRequest, Stats};
use crate::repo::{CommitInfo, Error, Repo, Result, TreeChange};

/// 추적하지 않는 파일을 이만큼만 보인다(경로 순). 무시하지 않은 빌드 결과 같은 것이 수만 개여도 목록이 바로 뜨게.
pub const UNTRACKED_LIMIT: usize = 2000;

/// 이보다 큰 작업트리 파일은 읽지 않고 크기·수정 시각으로 id를 만든다(내용이 같아도 바뀐 것으로 보인다).
const MAX_HASHED_BYTES: u64 = 64 << 20;

/// 줄 수를 세는 시간. 넘으면 나머지는 화면이 `file_stats`로 받는다(커밋 상세와 같다).
const STATS_BUDGET: Duration = Duration::from_millis(25);

/// 비교의 한쪽
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Commit(ObjectId),
    Worktree,
}

/// 비교 결과. 파일은 [`crate::commit::CommitDetail`]과 같은 모양이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// 바뀌기 전 커밋. 커밋이 없는 저장소(첫 커밋 전)의 작업트리 비교면 없다(빈 트리와 견준다).
    pub base: Option<CommitInfo>,
    /// 바뀐 뒤 커밋. 작업트리면 없다.
    pub target: Option<CommitInfo>,
    /// `base`는 두 리비전이 갈라진 지점(merge-base)이다.
    pub merge_base: bool,
    /// 경로 순
    pub files: Vec<FileChange>,
    pub additions: u32,
    pub deletions: u32,
    pub counted: bool,
    pub renames_incomplete: bool,
    pub renames_limited: bool,
    /// 보이지 않은 추적하지 않는 파일 수([`UNTRACKED_LIMIT`]를 넘은 것)
    pub untracked_omitted: usize,
}

/// `base`(커밋, `None`이면 빈 트리)와 `target`을 견준다. `merge_base`면 `base` 대신 두 커밋이 갈라진 지점과
/// 견준다(GitHub의 브랜치 비교, `git diff base...target`). 작업트리 쪽이면 `merge_base`는 HEAD와의 갈라진 지점이다.
pub fn compare(
    repo: &Repo,
    base: Option<ObjectId>,
    target: Side,
    merge_base: bool,
) -> Result<Comparison> {
    let started = Instant::now();
    let local = repo.local();
    let base = match (base, merge_base) {
        (Some(base), true) => {
            let other = match target {
                Side::Commit(id) => Some(id),
                Side::Worktree => repo.head()?,
            };
            match other {
                Some(other) => Some(
                    local
                        .merge_base(base, other)
                        .map_err(|e| Error(format!("갈라진 지점을 찾지 못했다: {e}")))?
                        .ok_or_else(|| Error("두 리비전의 이력이 만나지 않는다".into()))?
                        .detach(),
                ),
                None => Some(base),
            }
        }
        (base, _) => base,
    };
    let base_tree = match base {
        Some(id) => Some(tree_of(&local, id)?),
        None => None,
    };
    let (list, renames_incomplete, renames_limited, untracked_omitted) = match target {
        Side::Commit(id) => {
            let tree = tree_of(&local, id)?;
            let changes = crate::repo::tree_changes(
                &local,
                base_tree.as_ref(),
                &tree,
                crate::renames::RENAME_LIMIT,
            )?;
            (
                changes.list,
                changes.renames_incomplete,
                changes.renames_limited,
                0,
            )
        }
        Side::Worktree => {
            let found = worktree_changes(repo, &local, base_tree.as_ref())?;
            let changes = crate::repo::pair_renames(
                &local,
                found.list,
                crate::renames::RENAME_LIMIT,
                &found.contents,
            );
            (
                changes.list,
                changes.renames_incomplete,
                changes.renames_limited,
                found.untracked_omitted,
            )
        }
    };
    let worktree = target == Side::Worktree;
    let mut files: Vec<FileChange> = list
        .into_iter()
        .map(|change| {
            let mut file = crate::commit::describe(change);
            file.worktree = worktree && file.new_id.is_some();
            file
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut stats = Stats::new(repo, &local);
    for file in &mut files {
        if started.elapsed() >= STATS_BUDGET {
            break;
        }
        file.stat = Some(stats.of(&StatRequest::of(file))?);
    }
    let counted = files.iter().all(|f| f.stat.is_some());
    let sum = |f: fn(&crate::commit::FileStat) -> u32| {
        files
            .iter()
            .filter_map(|file| file.stat.as_ref())
            .map(f)
            .sum()
    };
    Ok(Comparison {
        base: base.map(|id| repo.commit_header(id)).transpose()?,
        target: match target {
            Side::Commit(id) => Some(repo.commit_header(id)?),
            Side::Worktree => None,
        },
        merge_base,
        additions: sum(|s| s.additions),
        deletions: sum(|s| s.deletions),
        files,
        counted,
        renames_incomplete,
        renames_limited,
        untracked_omitted,
    })
}

fn tree_of(repo: &gix::Repository, commit: ObjectId) -> Result<gix::Tree<'_>> {
    repo.find_commit(commit)
        .and_then(|commit| commit.tree())
        .map_err(|e| Error(format!("커밋 트리 ({commit}): {e}")))
}

/// 작업트리 비교의 날 결과(rename을 짝짓기 전)
struct Found {
    list: Vec<TreeChange>,
    /// 작업트리 파일의 내용(해시한 id → clean 내용). rename을 견줄 때 쓴다.
    contents: HashMap<ObjectId, Vec<u8>>,
    untracked_omitted: usize,
}

fn worktree_changes(
    repo: &Repo,
    local: &gix::Repository,
    base: Option<&gix::Tree<'_>>,
) -> Result<Found> {
    use gix::status::index_worktree::Item as Worktree;
    use gix::status::{Item, UntrackedFiles};

    let empty_tree = ObjectId::empty_tree(local.object_hash());
    let mut status = local
        .status(gix::progress::Discard)
        .map_err(|e| Error(format!("작업트리 상태: {e}")))?
        .head_tree(base.map_or(empty_tree, |tree| tree.id))
        .untracked_files(UntrackedFiles::Files)
        .index_worktree_submodules(None)
        .index_worktree_rewrites(None)
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Disabled);
    // 무시한 파일은 보지 않는다(기본값). 빈 디렉터리도 보이지 않는다.
    status = status.dirwalk_options(|options| options.emit_empty_directories(false));
    let iter = status
        .into_iter(None)
        .map_err(|e| Error(format!("작업트리 상태: {e}")))?;
    let mut tracked: BTreeSet<BString> = BTreeSet::new();
    let mut untracked: BTreeSet<BString> = BTreeSet::new();
    for item in iter {
        let item = item.map_err(|e| Error(format!("작업트리 상태: {e}")))?;
        match item {
            Item::TreeIndex(change) => {
                tracked.insert(change.location().to_owned());
            }
            Item::IndexWorktree(Worktree::Modification { rela_path, .. }) => {
                tracked.insert(rela_path);
            }
            Item::IndexWorktree(Worktree::DirectoryContents { entry, .. }) => {
                if entry.status == gix::dir::entry::Status::Untracked
                    && matches!(
                        entry.disk_kind,
                        Some(gix::dir::entry::Kind::File | gix::dir::entry::Kind::Symlink)
                    )
                {
                    untracked.insert(entry.rela_path);
                }
            }
            Item::IndexWorktree(Worktree::Rewrite { .. }) => {}
        }
    }
    let untracked_omitted = untracked.len().saturating_sub(UNTRACKED_LIMIT);
    let paths = tracked
        .into_iter()
        .chain(untracked.into_iter().take(UNTRACKED_LIMIT))
        .collect::<BTreeSet<_>>();

    let file_mode = local
        .config_snapshot()
        .boolean("core.fileMode")
        .unwrap_or(true);
    let mut contents = HashMap::new();
    let mut list = Vec::new();
    for path in paths {
        let before = match base {
            Some(tree) => base_entry(tree, &path)?,
            None => None,
        };
        // 하위 모듈(기준 트리의 커밋 항목)은 보지 않는다.
        if before.is_some_and(|(mode, _)| mode.kind() == EntryKind::Commit) {
            continue;
        }
        let Ok(rel) = path.to_str() else {
            // UTF-8이 아닌 경로는 작업트리에서 그 이름으로 찾을 수 없다(화면도 열지 못한다). 기준 쪽만 보인다.
            continue;
        };
        let after = worktree_entry(repo, rel, before.map(|(mode, _)| mode), file_mode)?;
        let after = after.map(|(mode, data)| {
            let id = match &data {
                Content::Read(bytes) => blob_id(local, bytes),
                Content::Large(stamp) => blob_id(local, stamp.as_bytes()),
            };
            if let Content::Read(bytes) = data {
                contents.insert(id, bytes);
            }
            (mode, id)
        });
        let location = path.clone();
        match (before, after) {
            (Some(before), Some(after)) if before == after => {}
            (Some((previous_entry_mode, previous_id)), Some((entry_mode, id))) => {
                list.push(TreeChange::Modification {
                    location,
                    previous_entry_mode,
                    previous_id,
                    entry_mode,
                    id,
                })
            }
            (Some((entry_mode, id)), None) => list.push(TreeChange::Deletion {
                location,
                entry_mode,
                id,
                relation: None,
            }),
            (None, Some((entry_mode, id))) => list.push(TreeChange::Addition {
                location,
                entry_mode,
                id,
                relation: None,
            }),
            (None, None) => {}
        }
    }
    Ok(Found {
        list,
        contents,
        untracked_omitted,
    })
}

/// 기준 트리의 `path` 항목(파일·심볼릭 링크·하위 모듈). 디렉터리나 없으면 `None`.
fn base_entry(tree: &gix::Tree<'_>, path: &BString) -> Result<Option<(EntryMode, ObjectId)>> {
    let entry = tree
        .lookup_entry(path.split(|&b| b == b'/'))
        .map_err(|e| Error(format!("트리 조회 ({path}): {e}")))?;
    Ok(entry
        .filter(|entry| !entry.mode().is_tree())
        .map(|entry| (entry.mode(), entry.object_id())))
}

enum Content {
    /// git에 넣을 모양(clean)의 내용
    Read(Vec<u8>),
    /// 너무 커서 읽지 않았다: 크기·수정 시각
    Large(String),
}

/// 작업트리의 `rel`: (모드, 내용). 없거나 디렉터리면 `None`. 실행 비트는 `core.fileMode`일 때만 파일에서 읽고
/// 아니면 기준 쪽을 따른다(git과 같다).
fn worktree_entry(
    repo: &Repo,
    rel: &str,
    before: Option<EntryMode>,
    file_mode: bool,
) -> Result<Option<(EntryMode, Content)>> {
    let path = repo.workdir().join(rel);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        // 경로 가운데 디렉터리 자리에 파일이 있다(NotADirectory) 같은 것도 없는 것으로 본다.
        Err(_) if !path.exists() => return Ok(None),
        Err(e) => return Err(Error(format!("파일을 읽지 못했다 ({rel}): {e}"))),
    };
    if meta.file_type().is_symlink() {
        let bytes = link_target(&path, rel)?;
        return Ok(Some((EntryKind::Link.into(), Content::Read(bytes))));
    }
    if !meta.is_file() {
        return Ok(None);
    }
    let executable = if file_mode {
        is_executable(&meta)
    } else {
        before.is_some_and(|mode| mode.kind() == EntryKind::BlobExecutable)
    };
    let mode: EntryMode = if executable {
        EntryKind::BlobExecutable
    } else {
        EntryKind::Blob
    }
    .into();
    if meta.len() > MAX_HASHED_BYTES {
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        return Ok(Some((
            mode,
            Content::Large(format!("plx-large:{rel}:{}:{modified}", meta.len())),
        )));
    }
    Ok(repo
        .worktree_clean(rel)?
        .map(|bytes| (mode, Content::Read(bytes))))
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// 심볼릭 링크가 가리키는 경로(git이 blob에 넣는 모양: `/`로 구분)
fn link_target(path: &std::path::Path, rel: &str) -> Result<Vec<u8>> {
    let target =
        std::fs::read_link(path).map_err(|e| Error(format!("링크를 읽지 못했다 ({rel}): {e}")))?;
    let bytes =
        gix::path::into_bstr(target).map_err(|e| Error(format!("링크 경로 ({rel}): {e}")))?;
    Ok(gix::path::to_unix_separators_on_windows(bytes)
        .into_owned()
        .into())
}

fn blob_id(repo: &gix::Repository, bytes: &[u8]) -> ObjectId {
    gix::objs::compute_hash(repo.object_hash(), gix::object::Kind::Blob, bytes)
        .unwrap_or_else(|_| ObjectId::null(repo.object_hash()))
}

/// 작업트리 파일을 diff·줄 수에 쓸 내용: 심볼릭 링크면 가리키는 경로(git이 blob에 넣는 것), 아니면 clean 내용.
/// 없으면 `None`.
pub fn worktree_content(repo: &Repo, rel: &str) -> Result<Option<Vec<u8>>> {
    let path = repo.workdir().join(rel);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => Ok(Some(link_target(&path, rel)?)),
        Ok(meta) if !meta.is_file() => Ok(None),
        _ => repo.worktree_clean(rel),
    }
}

/// 작업트리 파일의 크기(없으면 0). 모르면 `None`.
pub fn worktree_size(repo: &Repo, rel: &str) -> Option<u64> {
    match std::fs::symlink_metadata(repo.workdir().join(rel)) {
        Ok(meta) => Some(meta.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(0),
        Err(_) => None,
    }
}
