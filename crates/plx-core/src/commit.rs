//! 커밋 상세: 첫 부모(루트 커밋은 빈 트리)와 비교한 파일 목록, 파일별 더한·뺀 줄 수, 파일 하나의 두 버전.
//!
//! gix로 비교한다(프로세스를 띄우지 않는다). rename은 git 기본값(유사도 50%)으로 찾고,
//! merge 커밋은 첫 부모와 비교한다(GitHub·GitLens와 같다). 줄 수는 Myers diff로 세어 `git diff --numstat`과 같다.

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
    /// 바뀌기 전 blob(추가면 없다)
    pub old_id: Option<String>,
    /// 바뀐 뒤 blob(삭제면 없다)
    pub new_id: Option<String>,
    pub binary: bool,
    /// 하위 모듈(커밋을 가리키는 항목). 내용 diff가 없다.
    pub submodule: bool,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub info: CommitInfo,
    /// 경로 순
    pub files: Vec<FileChange>,
    pub additions: u32,
    pub deletions: u32,
}

/// 커밋 상세. `id`는 커밋이어야 한다.
///
/// `focus`(저장소 기준 경로)는 화면이 보려는 파일이다. 그 파일이 이 커밋에서 새로 생겼는데 rename으로
/// 짝지어지지 않았으면(파일을 많이 옮긴 커밋은 gix가 유사도 비교를 건너뛴다) 그 파일 하나만 git처럼
/// 원본을 찾아([`Repo::rename_source`]) rename으로 보인다. 파일 히스토리의 rename 지점과 맞춘다.
pub fn commit_detail(repo: &Repo, id: ObjectId, focus: Option<&str>) -> Result<CommitDetail> {
    let (info, mut changes) = repo.commit_changes(id)?;
    if let Some(focus) = focus {
        pair_rename(repo, id, focus, &mut changes)?;
    }
    let local = repo.local();
    let mut files = changes
        .into_iter()
        .map(|change| file_change(&local, change))
        .collect::<Result<Vec<_>>>()?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(CommitDetail {
        additions: files.iter().map(|f| f.additions).sum(),
        deletions: files.iter().map(|f| f.deletions).sum(),
        info,
        files,
    })
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
    let Some(source) = repo.rename_source(id, focus)? else {
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

fn file_change(repo: &gix::Repository, change: TreeChange) -> Result<FileChange> {
    let path = |location: &gix::bstr::BStr| location.to_str_lossy().into_owned();
    let (status, path, old_path, old, new) = match change {
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
            Some((entry_mode.kind(), id)),
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
            Some((entry_mode.kind(), id)),
            None,
        ),
        TreeChange::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => {
            let type_changed = is_link(previous_entry_mode.kind()) != is_link(entry_mode.kind())
                || is_submodule(previous_entry_mode.kind()) != is_submodule(entry_mode.kind());
            (
                if type_changed {
                    FileStatus::TypeChanged
                } else {
                    FileStatus::Modified
                },
                path(location.as_ref()),
                None,
                Some((previous_entry_mode.kind(), previous_id)),
                Some((entry_mode.kind(), id)),
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
            Some((source_entry_mode.kind(), source_id)),
            Some((entry_mode.kind(), id)),
        ),
    };

    let submodule = [old, new]
        .iter()
        .flatten()
        .any(|(kind, _)| is_submodule(*kind));
    let (mut binary, mut additions, mut deletions) = (false, 0, 0);
    if !submodule {
        let read = |side: Option<(EntryKind, ObjectId)>| -> Result<Vec<u8>> {
            match side {
                Some((_, id)) => blob(repo, id),
                None => Ok(Vec::new()),
            }
        };
        let (before, after) = (read(old)?, read(new)?);
        binary = text::is_binary(&before) || text::is_binary(&after);
        if !binary {
            (additions, deletions) = line_stats(&before, &after);
        }
    }
    Ok(FileChange {
        status,
        path,
        old_path,
        old_id: old.map(|(_, id)| id.to_string()),
        new_id: new.map(|(_, id)| id.to_string()),
        binary,
        submodule,
        additions,
        deletions,
    })
}

fn is_link(kind: EntryKind) -> bool {
    kind == EntryKind::Link
}

fn is_submodule(kind: EntryKind) -> bool {
    kind == EntryKind::Commit
}

fn blob(repo: &gix::Repository, id: ObjectId) -> Result<Vec<u8>> {
    let blob = repo
        .find_blob(id)
        .map_err(|e| Error(format!("blob 읽기 ({id}): {e}")))?;
    Ok(blob.detach().data)
}

/// (더한 줄, 뺀 줄). git처럼 끝 개행이 없는 마지막 줄도 한 줄로 센다.
pub fn line_stats(before: &[u8], after: &[u8]) -> (u32, u32) {
    let input = InternedInput::new(before, after);
    let diff = Diff::compute(Algorithm::Myers, &input);
    (diff.count_additions(), diff.count_removals())
}

/// 파일 하나의 두 버전(diff 화면용).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileVersions {
    /// 바뀌기 전 내용. 추가된 파일이거나 바이너리면 없다.
    pub old: Option<Decoded>,
    pub new: Option<Decoded>,
    pub binary: bool,
}

/// blob 두 개(없으면 빈 파일)를 화면에 보일 문자열로 읽는다.
pub fn file_versions(
    repo: &Repo,
    old_id: Option<&str>,
    new_id: Option<&str>,
) -> Result<FileVersions> {
    let local = repo.local();
    let read = |id: Option<&str>| -> Result<Option<Vec<u8>>> {
        id.map(|id| {
            let id = ObjectId::from_hex(id.as_bytes())
                .map_err(|e| Error(format!("blob id가 아니다 ({id}): {e}")))?;
            blob(&local, id)
        })
        .transpose()
    };
    let (old, new) = (read(old_id)?, read(new_id)?);
    let binary = [&old, &new]
        .iter()
        .any(|side| side.as_ref().is_some_and(|b| text::is_binary(b)));
    if binary {
        return Ok(FileVersions {
            old: None,
            new: None,
            binary,
        });
    }
    Ok(FileVersions {
        old: old.map(|b| text::decode(&b)),
        new: new.map(|b| text::decode(&b)),
        binary,
    })
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
