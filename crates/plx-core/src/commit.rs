//! 커밋 상세: 첫 부모(루트 커밋은 빈 트리)와 비교한 파일 목록, 파일별 더한·뺀 줄 수, 파일 하나의 두 버전.
//!
//! gix로 비교한다(프로세스를 띄우지 않는다). rename은 git과 같은 단계·기준(유사도 50%)으로 찾고([`crate::renames`]),
//! merge 커밋은 첫 부모와 비교한다(GitHub·GitLens와 같다). 줄 수는 Myers diff로 세어 대개 `git diff --numstat`과
//! 같다([`line_stats`]). 큰 커밋은 목록을 먼저 보내고 줄 수는 나중에 센다([`file_stats`]).

use std::time::{Duration, Instant};

use gix::ObjectId;
use gix::bstr::ByteSlice;
use gix::diff::blob::{Algorithm, Diff, InternedInput};
use gix::object::tree::EntryKind;
use serde::{Deserialize, Serialize};

use crate::repo::{CommitInfo, Error, Repo, Result, TreeChange};
use crate::text::{self, Decoded};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    /// 파일 ↔ 심볼릭 링크 같은 종류 변경
    TypeChanged,
}

impl FileStatus {
    /// `git diff --name-status`의 글자
    pub fn letter(self) -> char {
        match self {
            FileStatus::Added => 'A',
            FileStatus::Deleted => 'D',
            FileStatus::Modified => 'M',
            FileStatus::Renamed => 'R',
            FileStatus::Copied => 'C',
            FileStatus::TypeChanged => 'T',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub status: FileStatus,
    /// 커밋 뒤 경로(지운 파일이면 지우기 전 경로)
    pub path: String,
    /// rename·copy의 원래 경로
    pub old_path: Option<String>,
    /// 경로가 UTF-8이 아니라 깨진 글자로 바꿔 읽었다. 이 경로로는 blame·히스토리를 열 수 없다.
    pub path_lossy: bool,
    /// 바뀌기 전 blob(추가면 없다)
    pub old_id: Option<String>,
    /// 바뀐 뒤 blob(삭제면 없다)
    pub new_id: Option<String>,
    /// git 파일 모드(`100644`, `100755`, `120000`, `160000`). 추가면 `old_mode`, 삭제면 `new_mode`가 없다.
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    /// 하위 모듈(커밋을 가리키는 항목). 내용 diff가 없다.
    pub submodule: bool,
    /// 줄 수. `None`이면 아직 세지 않았다(큰 커밋은 목록을 먼저 보이고 [`file_stats`]로 나머지를 센다).
    pub stat: Option<FileStat>,
    /// 바뀐 뒤가 작업트리 파일이다([`crate::changes`]). `new_id`는 그 내용을 git처럼 해시한 값이라 저장소에 없다.
    pub worktree: bool,
}

/// 파일 하나의 줄 수
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStat {
    /// 내용을 줄로 비교하지 않는다(앞 8000바이트에 NUL, `binary`·`-diff` 속성). git의 numstat `-`.
    pub binary: bool,
    /// blob이 없다(부분 클론에서 받지 않았다). 줄 수를 모른다.
    pub missing: bool,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub info: CommitInfo,
    /// 경로 순
    pub files: Vec<FileChange>,
    /// 센 파일들의 합계([`CommitDetail::counted`]가 아니면 일부다)
    pub additions: u32,
    pub deletions: u32,
    /// 모든 파일의 줄 수를 셌다.
    pub counted: bool,
    /// blob이 없어 rename을 다 견주지 못했다(부분 클론).
    pub renames_incomplete: bool,
    /// 지운 파일 × 새 파일이 git의 한도(1,000²)를 넘어 유사도로 rename을 찾지 않았다(git도 건너뛴다).
    pub renames_limited: bool,
}

/// 상세를 보내기 전에 줄 수를 세는 시간. 넘으면 나머지는 화면이 따로 받는다(목록을 먼저 그린다).
const STATS_BUDGET: Duration = Duration::from_millis(25);

/// 커밋 상세. `id`는 커밋이어야 한다.
///
/// `focus`(저장소 기준 경로)는 화면이 보려는 파일이다. 그 파일이 이 커밋에서 새로 생겼는데 rename으로
/// 짝지어지지 않았으면 그 파일 하나만 git의 `--follow`처럼 원본을 찾아([`Repo::rename_source`]) rename으로
/// 보인다. 파일 히스토리의 rename 지점과 맞춘다.
pub fn commit_detail(repo: &Repo, id: ObjectId, focus: Option<&str>) -> Result<CommitDetail> {
    let started = Instant::now();
    let (info, changes) = repo.commit_changes(id, crate::renames::RENAME_LIMIT)?;
    let mut list = changes.list;
    if let Some(focus) = focus {
        pair_rename(repo, id, focus, &mut list)?;
    }
    let mut files: Vec<FileChange> = list.into_iter().map(describe).collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let local = repo.local();
    let mut stats = Stats::new(repo, &local);
    for file in &mut files {
        if started.elapsed() >= STATS_BUDGET {
            break;
        }
        file.stat = Some(stats.of(&StatRequest::of(file))?);
    }
    let counted = files.iter().all(|f| f.stat.is_some());
    Ok(CommitDetail {
        additions: files
            .iter()
            .filter_map(|f| f.stat)
            .map(|s| s.additions)
            .sum(),
        deletions: files
            .iter()
            .filter_map(|f| f.stat)
            .map(|s| s.deletions)
            .sum(),
        info,
        files,
        counted,
        renames_incomplete: changes.renames_incomplete,
        renames_limited: changes.renames_limited,
    })
}

/// 줄 수를 셀 파일 하나([`file_stats`]의 요청)
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatRequest {
    pub path: String,
    pub old_path: Option<String>,
    pub old_id: Option<String>,
    pub new_id: Option<String>,
    pub submodule: bool,
    /// 바뀐 뒤를 작업트리의 `path`에서 읽는다.
    #[serde(default)]
    pub worktree: bool,
}

impl StatRequest {
    pub(crate) fn of(file: &FileChange) -> StatRequest {
        StatRequest {
            path: file.path.clone(),
            old_path: file.old_path.clone(),
            old_id: file.old_id.clone(),
            new_id: file.new_id.clone(),
            submodule: file.submodule,
            worktree: file.worktree,
        }
    }
}

/// [`commit_detail`]이 시간 안에 세지 못한 파일들의 줄 수(요청 순서대로).
pub fn file_stats(repo: &Repo, files: &[StatRequest]) -> Result<Vec<FileStat>> {
    let local = repo.local();
    let mut stats = Stats::new(repo, &local);
    files.iter().map(|file| stats.of(file)).collect()
}

/// 줄 수 세기. `.gitattributes`(작업트리, 없으면 인덱스)의 `diff` 속성을 git처럼 본다.
pub(crate) struct Stats<'r> {
    owner: &'r Repo,
    repo: &'r gix::Repository,
    attributes: Option<Attributes<'r>>,
}

impl<'r> Stats<'r> {
    pub(crate) fn new(owner: &'r Repo, repo: &'r gix::Repository) -> Stats<'r> {
        Stats {
            owner,
            repo,
            attributes: Attributes::new(repo),
        }
    }

    pub(crate) fn of(&mut self, file: &StatRequest) -> Result<FileStat> {
        let side = |id: &Option<String>| id.as_deref().map(parse_id).transpose();
        let (old, new) = (side(&file.old_id)?, side(&file.new_id)?);
        if file.submodule {
            // git은 하위 모듈의 바뀜을 `Subproject commit <sha>` 한 줄로 센다.
            return Ok(FileStat {
                binary: false,
                missing: false,
                additions: u32::from(new.is_some()),
                deletions: u32::from(old.is_some()),
            });
        }
        let no_diff = self.attributes.as_mut().is_some_and(|a| {
            a.no_diff(&file.path) || file.old_path.as_deref().is_some_and(|p| a.no_diff(p))
        });
        let after = if file.worktree {
            // 지운 파일은 빈 내용이다(삭제는 `new_id`가 없다).
            match new {
                Some(_) => Some(
                    crate::changes::worktree_content(self.owner, &file.path)?.unwrap_or_default(),
                ),
                None => Some(Vec::new()),
            }
        } else {
            read_blob(self.repo, new)?
        };
        let (before, after) = match (read_blob(self.repo, old)?, after) {
            (Some(before), Some(after)) => (before, after),
            _ => {
                return Ok(FileStat {
                    binary: no_diff,
                    missing: true,
                    additions: 0,
                    deletions: 0,
                });
            }
        };
        if no_diff || text::is_binary(&before) || text::is_binary(&after) {
            return Ok(FileStat {
                binary: true,
                missing: false,
                additions: 0,
                deletions: 0,
            });
        }
        let (additions, deletions) = line_stats(&before, &after);
        Ok(FileStat {
            binary: false,
            missing: false,
            additions,
            deletions,
        })
    }
}

/// `diff` 속성 찾기(`-diff`, `binary`는 줄로 비교하지 않는다). 속성을 읽지 못하면 없는 것으로 본다.
struct Attributes<'r> {
    stack: gix::AttributeStack<'r>,
    outcome: gix::attrs::search::Outcome,
}

impl<'r> Attributes<'r> {
    fn new(repo: &'r gix::Repository) -> Option<Attributes<'r>> {
        let index = repo.index_or_empty().ok()?;
        let stack = repo
            .attributes_only(
                &index,
                gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
            )
            .ok()?;
        let outcome = stack.selected_attribute_matches(["diff"]);
        Some(Attributes { stack, outcome })
    }

    fn no_diff(&mut self, path: &str) -> bool {
        let Ok(platform) = self.stack.at_entry(path, None) else {
            return false;
        };
        self.outcome.reset();
        platform.matching_attributes(&mut self.outcome);
        self.outcome
            .iter_selected()
            .any(|m| matches!(m.assignment.state, gix::attrs::StateRef::Unset))
    }
}

/// `focus`의 추가와 그 rename 원본의 삭제를 rename 하나로 합친다.
fn pair_rename(
    repo: &Repo,
    id: ObjectId,
    focus: &str,
    changes: &mut Vec<TreeChange>,
) -> Result<()> {
    let is_added = |c: &TreeChange| matches!(c, TreeChange::Addition { location, .. } if location.as_slice() == focus.as_bytes());
    let Some(added) = changes.iter().position(is_added) else {
        return Ok(());
    };
    // 부분 클론에서 blob이 없으면 원본을 찾지 못한다. 상세는 그대로 보인다.
    let Ok(Some(source)) = repo.rename_source(id, focus) else {
        return Ok(());
    };
    let Some(deleted) = changes.iter().position(|c| {
        matches!(c, TreeChange::Deletion { location, .. } if location.as_slice() == source.as_bytes())
    }) else {
        return Ok(());
    };
    let (
        TreeChange::Deletion {
            location: source_location,
            entry_mode: source_entry_mode,
            relation: source_relation,
            id: source_id,
        },
        TreeChange::Addition {
            location,
            entry_mode,
            relation,
            id,
        },
    ) = (changes[deleted].clone(), changes[added].clone())
    else {
        return Ok(());
    };
    changes[added] = TreeChange::Rewrite {
        source_location,
        source_entry_mode,
        source_relation,
        source_id,
        diff: None,
        entry_mode,
        id,
        location,
        relation,
        copy: false,
    };
    changes.remove(deleted);
    Ok(())
}

/// 트리 비교 결과 하나를 화면의 파일 하나로(blob은 읽지 않는다).
pub(crate) fn describe(change: TreeChange) -> FileChange {
    let mut lossy = false;
    let mut path = |location: &gix::bstr::BStr| match location.to_str() {
        Ok(path) => path.to_owned(),
        Err(_) => {
            lossy = true;
            location.to_str_lossy().into_owned()
        }
    };
    let (status, new_path, old_path, old, new) = match change {
        TreeChange::Addition {
            location,
            entry_mode,
            id,
            ..
        } => (
            FileStatus::Added,
            path(location.as_ref()),
            None,
            None,
            Some((entry_mode, id)),
        ),
        TreeChange::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => (
            FileStatus::Deleted,
            path(location.as_ref()),
            None,
            Some((entry_mode, id)),
            None,
        ),
        TreeChange::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => {
            let (before, after) = (previous_entry_mode.kind(), entry_mode.kind());
            let type_changed =
                is_link(before) != is_link(after) || is_submodule(before) != is_submodule(after);
            (
                if type_changed {
                    FileStatus::TypeChanged
                } else {
                    FileStatus::Modified
                },
                path(location.as_ref()),
                None,
                Some((previous_entry_mode, previous_id)),
                Some((entry_mode, id)),
            )
        }
        TreeChange::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            copy,
            ..
        } => (
            if copy {
                FileStatus::Copied
            } else {
                FileStatus::Renamed
            },
            path(location.as_ref()),
            Some(path(source_location.as_ref())),
            Some((source_entry_mode, source_id)),
            Some((entry_mode, id)),
        ),
    };
    let submodule = [old, new]
        .iter()
        .flatten()
        .any(|(mode, _)| is_submodule(mode.kind()));
    let mode = |mode: gix::object::tree::EntryMode| format!("{:06o}", mode.value());
    FileChange {
        status,
        path: new_path,
        old_path,
        path_lossy: lossy,
        old_id: old.map(|(_, id)| id.to_string()),
        new_id: new.map(|(_, id)| id.to_string()),
        old_mode: old.map(|(m, _)| mode(m)),
        new_mode: new.map(|(m, _)| mode(m)),
        submodule,
        stat: None,
        worktree: false,
    }
}

fn is_link(kind: EntryKind) -> bool {
    kind == EntryKind::Link
}

fn is_submodule(kind: EntryKind) -> bool {
    kind == EntryKind::Commit
}

fn parse_id(id: &str) -> Result<ObjectId> {
    ObjectId::from_hex(id.as_bytes()).map_err(|e| Error(format!("blob id가 아니다 ({id}): {e}")))
}

/// blob 내용. 없는 쪽(추가·삭제)은 빈 내용, blob이 저장소에 없으면(부분 클론) `None`.
fn read_blob(repo: &gix::Repository, id: Option<ObjectId>) -> Result<Option<Vec<u8>>> {
    let Some(id) = id else {
        return Ok(Some(Vec::new()));
    };
    match repo.try_find_object(id) {
        Ok(Some(object)) => Ok(Some(object.detach().data)),
        Ok(None) => Ok(None),
        Err(e) => Err(Error(format!("blob 읽기 ({id}): {e}"))),
    }
}

/// (더한 줄, 뺀 줄). git처럼 끝 개행이 없는 마지막 줄도 한 줄로 센다. imara-diff의 Myers라 git(xdiff)과
/// 같은 diff가 여럿일 때 다른 쪽을 골라 몇 줄 다를 수 있다(git/git 최근 3,000개 중 73개, 대개 1~10줄).
pub fn line_stats(before: &[u8], after: &[u8]) -> (u32, u32) {
    let input = InternedInput::new(before, after);
    let diff = Diff::compute(Algorithm::Myers, &input);
    (diff.count_additions(), diff.count_removals())
}

/// 이보다 큰 파일(두 쪽 합)은 내용을 보내지 않는다. 화면이 "그래도 보기"로 다시 요청한다(`full`).
pub const MAX_VERSIONS_BYTES: u64 = 4 << 20;

/// 파일 하나의 두 버전(diff 화면용).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileVersions {
    /// 바뀌기 전 내용. 추가된 파일이거나 바이너리면 없다.
    pub old: Option<Decoded>,
    pub new: Option<Decoded>,
    pub binary: bool,
    /// blob이 없다(부분 클론에서 받지 않았다).
    pub missing: bool,
    /// 너무 커서 내용을 보내지 않았다: 두 쪽의 바이트 수 합
    pub too_large: Option<u64>,
    /// 줄 diff에는 보이지 않는 바뀜(줄 끝, 끝 개행, BOM, 인코딩)
    pub notes: Vec<String>,
}

/// blob 두 개(없으면 빈 파일)를 화면에 보일 문자열로 읽는다. `path`가 있으면 그 경로의 `diff` 속성을 본다.
/// `full`이 아니면 [`MAX_VERSIONS_BYTES`]보다 큰 파일은 내용 없이 크기만 알린다.
/// `worktree`면 바뀐 뒤(`new_id`가 있을 때)를 작업트리의 `path`에서 읽는다([`crate::changes`]).
pub fn file_versions(
    repo: &Repo,
    old_id: Option<&str>,
    new_id: Option<&str>,
    path: Option<&str>,
    full: bool,
    worktree: bool,
) -> Result<FileVersions> {
    let local = repo.local();
    let (old_id, new_id) = (
        old_id.map(parse_id).transpose()?,
        new_id.map(parse_id).transpose()?,
    );
    let worktree_path = match (worktree, path, new_id) {
        (true, Some(path), Some(_)) => Some(path),
        (true, None, Some(_)) => return Err(Error("작업트리 파일의 경로가 없다".into())),
        _ => None,
    };
    let empty = |missing: bool, binary: bool, too_large: Option<u64>| FileVersions {
        old: None,
        new: None,
        binary,
        missing,
        too_large,
        notes: Vec::new(),
    };
    if !full {
        let size = |id: Option<ObjectId>| {
            id.map_or(Some(0), |id| local.find_header(id).ok().map(|h| h.size()))
        };
        let new_size = match worktree_path {
            Some(path) => crate::changes::worktree_size(repo, path),
            None => size(new_id),
        };
        if let (Some(old), Some(new)) = (size(old_id), new_size)
            && old + new > MAX_VERSIONS_BYTES
        {
            return Ok(empty(false, false, Some(old + new)));
        }
    }
    let no_diff =
        path.is_some_and(|path| Attributes::new(&local).is_some_and(|mut a| a.no_diff(path)));
    let new = match worktree_path {
        Some(path) => Some(crate::changes::worktree_content(repo, path)?.unwrap_or_default()),
        None => read_blob(&local, new_id)?,
    };
    let old = read_blob(&local, old_id)?;
    let (Some(old_bytes), Some(new_bytes)) = (old, new) else {
        return Ok(empty(true, no_diff, None));
    };
    if no_diff || text::is_binary(&old_bytes) || text::is_binary(&new_bytes) {
        return Ok(empty(false, true, None));
    }
    let old = old_id.map(|_| text::decode(&old_bytes));
    let new = new_id.map(|_| text::decode(&new_bytes));
    let notes = match (&old, &new) {
        (Some(o), Some(n)) => invisible_changes(&old_bytes, &new_bytes, o, n),
        _ => Vec::new(),
    };
    Ok(FileVersions {
        old,
        new,
        binary: false,
        missing: false,
        too_large: None,
        notes,
    })
}

/// 화면의 줄 diff에는 보이지 않는 바뀜. diff 화면은 줄 끝을 맞추고 끝 개행·BOM을 떼고 보여서, 이것만 바뀐
/// 파일은 diff가 비어 보인다.
fn invisible_changes(
    old: &[u8],
    new: &[u8],
    old_text: &Decoded,
    new_text: &Decoded,
) -> Vec<String> {
    let mut notes = Vec::new();
    let (old_eol, new_eol) = (line_endings(old), line_endings(new));
    if let (Some(a), Some(b)) = (old_eol, new_eol)
        && a != b
    {
        notes.push(format!("줄 끝: {a} → {b}"));
    }
    let ends = |b: &[u8]| b.ends_with(b"\n");
    if !old.is_empty() && !new.is_empty() && ends(old) != ends(new) {
        notes.push(if ends(new) {
            "끝 개행을 더했다".into()
        } else {
            "끝 개행을 지웠다".into()
        });
    }
    let bom = |b: &[u8]| b.starts_with(b"\xef\xbb\xbf");
    if bom(old) != bom(new) {
        notes.push(if bom(new) {
            "UTF-8 BOM을 더했다".into()
        } else {
            "UTF-8 BOM을 지웠다".into()
        });
    }
    if old_text.encoding != new_text.encoding {
        notes.push(format!(
            "인코딩: {} → {}",
            encoding_name(old_text.encoding),
            encoding_name(new_text.encoding)
        ));
    }
    notes
}

/// 줄 끝 종류. 개행이 없으면 `None`.
fn line_endings(bytes: &[u8]) -> Option<&'static str> {
    let lf = bytes.iter().filter(|&&b| b == b'\n').count();
    if lf == 0 {
        return None;
    }
    let crlf = bytes.windows(2).filter(|w| w == b"\r\n").count();
    Some(match crlf {
        0 => "LF",
        n if n == lf => "CRLF",
        _ => "LF·CRLF 섞임",
    })
}

fn encoding_name(encoding: text::Encoding) -> &'static str {
    match encoding {
        text::Encoding::Utf8 => "UTF-8",
        text::Encoding::EucKr => "EUC-KR",
        text::Encoding::Lossy => "UTF-8(깨진 바이트)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_stats_count_like_numstat() {
        assert_eq!(line_stats(b"", b""), (0, 0));
        assert_eq!(line_stats(b"", b"a\nb\n"), (2, 0));
        assert_eq!(line_stats(b"a\nb\n", b""), (0, 2));
        assert_eq!(line_stats(b"a\nb\nc\n", b"a\nx\nc\n"), (1, 1));
        // 끝 개행만 더하면 마지막 줄이 바뀐 것으로 센다(git과 같다)
        assert_eq!(line_stats(b"a\nb", b"a\nb\n"), (1, 1));
    }

    #[test]
    fn status_letters() {
        assert_eq!(FileStatus::Renamed.letter(), 'R');
        assert_eq!(FileStatus::TypeChanged.letter(), 'T');
    }
}
