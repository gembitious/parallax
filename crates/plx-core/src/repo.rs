//! 저장소 읽기(gix). refs·커밋·트리·blob을 프로세스를 띄우지 않고 읽는다.

use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use gix::ObjectId;
use gix::bstr::{BStr, ByteSlice};
use serde::Serialize;

/// 저장소 작업의 오류. 화면에 그대로 보여 줄 문장이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn err(context: &str, e: impl fmt::Display) -> Error {
    Error(format!("{context}: {e}"))
}

/// 작업트리가 있는 저장소. 여러 스레드에서 나눠 쓴다.
pub struct Repo {
    sync: gix::ThreadSafeRepository,
    /// 심볼릭 링크를 푼 작업트리 경로. 사용자 경로와 비교할 때 기준이 된다(Windows는 `\\?\` 형식).
    canonical: PathBuf,
    /// 화면과 git 작업 디렉터리에 쓰는 경로. Windows에서 `\\?\`를 뗄 수 있으면 뗀다
    /// (Git for Windows 같은 자식 프로세스가 그 형식의 작업 디렉터리를 다루지 못할 수 있다).
    workdir: PathBuf,
}

impl fmt::Debug for Repo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Repo")
            .field("workdir", &self.workdir)
            .finish()
    }
}

impl Repo {
    /// `path`(파일이나 디렉터리, 지금은 없는 경로도 된다)를 담은 저장소를 찾는다.
    pub fn discover(path: &Path) -> Result<Repo> {
        let start = existing_ancestor(path)
            .ok_or_else(|| Error(format!("경로가 없다: {}", path.display())))?;
        let start = if start.is_dir() {
            start
        } else {
            start.parent().unwrap_or(start)
        };
        let repo = gix::discover(start)
            .map_err(|e| err(&format!("git 저장소가 아니다 ({})", start.display()), e))?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| Error("작업트리가 없는(bare) 저장소는 지원하지 않는다".to_owned()))?;
        let canonical = std::fs::canonicalize(workdir).map_err(|e| err("작업트리 경로", e))?;
        Ok(Repo {
            sync: repo.into_sync(),
            workdir: dunce::simplified(&canonical).to_path_buf(),
            canonical,
        })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    pub fn local(&self) -> gix::Repository {
        self.sync.to_thread_local()
    }

    /// 절대 경로를 저장소 기준 상대 경로(`/`로 구분)로 바꾼다.
    /// 심볼릭 링크·대소문자(Windows·macOS)·`/private/var` 같은 별칭은 실제 경로로 맞춘 뒤 비교한다.
    pub fn relative_path(&self, path: &Path) -> Result<String> {
        let canonical = canonicalize_lenient(path)?;
        let relative = canonical.strip_prefix(&self.canonical).map_err(|_| {
            Error(format!(
                "저장소({}) 밖의 경로다: {}",
                self.workdir.display(),
                path.display()
            ))
        })?;
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => {
                    parts.push(part.to_str().ok_or_else(|| {
                        Error(format!("경로가 UTF-8이 아니다: {}", path.display()))
                    })?)
                }
                other => return Err(Error(format!("예상하지 못한 경로 구성 요소: {other:?}"))),
            }
        }
        Ok(parts.join("/"))
    }

    /// HEAD가 가리키는 커밋. 커밋이 하나도 없는 저장소면 `None`.
    pub fn head(&self) -> Result<Option<ObjectId>> {
        let repo = self.local();
        let mut head = repo.head().map_err(|e| err("HEAD", e))?;
        match head.try_peel_to_id() {
            Ok(Some(id)) => Ok(Some(id.detach())),
            Ok(None) => Ok(None),
            Err(e) => Err(err("HEAD", e)),
        }
    }

    /// 리비전 표현(`HEAD~2`, 태그, sha 일부 등)을 커밋으로 푼다.
    pub fn resolve_commit(&self, rev: &str) -> Result<ObjectId> {
        let repo = self.local();
        let id = repo
            .rev_parse_single(rev)
            .map_err(|e| err(&format!("리비전을 찾을 수 없다 ({rev})"), e))?;
        let commit = id
            .object()
            .and_then(|object| object.peel_to_commit())
            .map_err(|e| err(&format!("커밋이 아니다 ({rev})"), e))?;
        Ok(commit.id)
    }

    /// `commit`의 `path` 파일 내용. 그 커밋에 파일이 없으면 `None`.
    pub fn blob_at(&self, commit: ObjectId, path: &str) -> Result<Option<Vec<u8>>> {
        let repo = self.local();
        let tree = repo
            .find_commit(commit)
            .and_then(|commit| commit.tree())
            .map_err(|e| err("커밋 트리", e))?;
        let Some(entry) = tree
            .lookup_entry_by_path(path)
            .map_err(|e| err("트리 조회", e))?
        else {
            return Ok(None);
        };
        if !entry.mode().is_blob() {
            return Err(Error(format!("파일이 아니다: {path}")));
        }
        let blob = entry.object().map_err(|e| err("blob 읽기", e))?;
        Ok(Some(blob.detach().data))
    }

    /// `commit`에 `path` 파일(blob·실행 파일·심볼릭 링크)이 있는가. 내용은 읽지 않는다.
    pub fn has_file(&self, commit: ObjectId, path: &str) -> Result<bool> {
        let repo = self.local();
        let tree = repo
            .find_commit(commit)
            .and_then(|commit| commit.tree())
            .map_err(|e| err("커밋 트리", e))?;
        let entry = tree
            .lookup_entry_by_path(path)
            .map_err(|e| err("트리 조회", e))?;
        Ok(entry.is_some_and(|entry| !entry.mode().is_tree() && !entry.mode().is_commit()))
    }

    /// 커밋 상세(hover 카드). 변경 파일 수는 첫 부모(없으면 빈 트리)와 비교한다.
    pub fn commit_info(&self, id: ObjectId) -> Result<CommitInfo> {
        self.commit_changes(id).map(|(info, _)| info)
    }

    /// 커밋 정보와 첫 부모(없으면 빈 트리)와 비교한 파일 변경. 디렉터리(트리) 항목은 뺀다.
    /// rename은 사용자 설정과 관계없이 git 기본값(유사도 50%)으로 찾는다.
    pub(crate) fn commit_changes(&self, id: ObjectId) -> Result<(CommitInfo, Vec<TreeChange>)> {
        let repo = self.local();
        let commit = repo.find_commit(id).map_err(|e| err("커밋 읽기", e))?;
        let decoded = commit.decode().map_err(|e| err("커밋 해석", e))?;
        let encoding = decoded.encoding;
        let message = decode_text(decoded.message, encoding);
        let author = person(commit.author().map_err(|e| err("작성자", e))?, encoding);
        let committer = person(commit.committer().map_err(|e| err("커미터", e))?, encoding);
        let parents: Vec<ObjectId> = commit.parent_ids().map(|id| id.detach()).collect();

        let tree = commit.tree().map_err(|e| err("커밋 트리", e))?;
        // 얕은 클론의 경계 커밋은 부모 객체가 없다. git처럼 루트 커밋으로 보고 빈 트리와 비교한다.
        let parent_tree = match parents.first() {
            Some(parent) => match repo.try_find_object(*parent) {
                Ok(Some(object)) => Some(object.peel_to_tree().map_err(|e| err("부모 트리", e))?),
                Ok(None) => None,
                Err(e) => return Err(err("부모 커밋", e)),
            },
            None => None,
        };
        let options =
            gix::diff::Options::default().with_rewrites(Some(gix::diff::Rewrites::default()));
        // 트리 비교는 바뀐 파일을 담은 디렉터리(트리) 항목도 변경으로 돌려준다. 파일만 남긴다.
        let changes: Vec<TreeChange> = repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), options)
            .map_err(|e| err("트리 비교", e))?
            .into_iter()
            .filter(|change| !change.entry_mode().is_tree())
            .collect();

        let info = CommitInfo {
            sha: id.to_string(),
            parents: parents.iter().map(ToString::to_string).collect(),
            summary: message.lines().next().unwrap_or("").trim().to_owned(),
            message: message.trim_end().to_owned(),
            author,
            committer,
            changed_files: changes.len(),
        };
        Ok((info, changes))
    }
}

/// rename 원본 후보를 이만큼만 내용까지 비교한다(크기로 거른 뒤, 이름이 비슷한 것부터).
const MAX_RENAME_CANDIDATES: usize = 4000;

impl Repo {
    /// `id` 커밋이 첫 부모의 다른 파일을 옮겨(rename) `path`를 만들었다면 그 원래 경로.
    ///
    /// `git log --follow`처럼 이 파일 하나의 원본만 찾는다: 그 커밋에서 지운 파일 가운데 내용이 같은 것,
    /// 없으면 git과 같은 유사도([`crate::similarity`])가 50% 이상인 것 중 가장 비슷한 것(같으면 파일 이름이
    /// 같은 것). 커밋 전체의 rename을 찾지 않으므로 파일 수백 개를 옮긴 커밋에서도 빠르다.
    /// 복사(copy)는 따라가지 않는다. 얕은 클론의 경계처럼 부모가 없으면 `None`.
    pub fn rename_source(&self, id: ObjectId, path: &str) -> Result<Option<String>> {
        use crate::similarity::{self, MINIMUM_SCORE, Spans};

        let repo = self.local();
        let commit = repo.find_commit(id).map_err(|e| err("커밋 읽기", e))?;
        let Some(parent) = commit.parent_ids().next().map(|id| id.detach()) else {
            return Ok(None);
        };
        let Some(parent) = repo
            .try_find_object(parent)
            .map_err(|e| err("부모 커밋", e))?
        else {
            return Ok(None);
        };
        let parent_tree = parent.peel_to_tree().map_err(|e| err("부모 트리", e))?;
        let tree = commit.tree().map_err(|e| err("커밋 트리", e))?;
        let Some(dest) = tree
            .lookup_entry_by_path(path)
            .map_err(|e| err("트리 조회", e))?
        else {
            return Ok(None);
        };
        // git처럼 일반 파일만 내용으로 비교한다(심볼릭 링크는 내용이 같을 때만 rename).
        let dest_regular = dest.mode().is_blob();
        let dest_id = dest.object_id();
        if parent_tree
            .lookup_entry_by_path(path)
            .map_err(|e| err("트리 조회", e))?
            .is_some()
        {
            return Ok(None);
        }

        let options = gix::diff::Options::default().with_rewrites(None);
        let deleted: Vec<(String, ObjectId, bool)> = repo
            .diff_tree_to_tree(Some(&parent_tree), Some(&tree), options)
            .map_err(|e| err("트리 비교", e))?
            .into_iter()
            .filter_map(|change| match change {
                TreeChange::Deletion {
                    location,
                    entry_mode,
                    id,
                    ..
                } if !entry_mode.is_tree() && !entry_mode.is_commit() => Some((
                    location.to_str_lossy().into_owned(),
                    id,
                    entry_mode.is_blob(),
                )),
                _ => None,
            })
            .collect();
        let name = file_name(path);
        let same_name = |candidate: &str| file_name(candidate) == name;

        // 1. 내용이 같은 파일(파일 이름이 같은 것을 먼저)
        let mut exact = deleted.iter().filter(|(_, id, _)| *id == dest_id);
        if let Some(first) = exact.next() {
            let best = std::iter::once(first)
                .chain(exact)
                .find(|(p, ..)| same_name(p))
                .unwrap_or(first);
            return Ok(Some(best.0.clone()));
        }
        if !dest_regular {
            return Ok(None);
        }

        // 2. 비슷한 파일. 이름이 닮은 것부터, 크기로 먼저 거르고(헤더만 읽는다) 내용을 비교한다.
        let dest_data = repo
            .find_object(dest_id)
            .map_err(|e| err("blob 읽기", e))?
            .detach()
            .data;
        if dest_data.is_empty() {
            return Ok(None);
        }
        let dest_size = dest_data.len() as u64;
        let dest_spans = Spans::new(&dest_data, !crate::text::is_binary(&dest_data));
        let mut candidates: Vec<(u8, &str, ObjectId)> = deleted
            .iter()
            .filter(|(.., regular)| *regular)
            .map(|(candidate, id, _)| (affinity(candidate, path), candidate.as_str(), *id))
            .collect();
        candidates.sort_by_key(|(affinity, ..)| std::cmp::Reverse(*affinity));

        let mut best: Option<(u64, bool, &str)> = None;
        let mut compared = 0;
        for (affinity, candidate, id) in candidates {
            let size = repo
                .find_header(id)
                .map_err(|e| err("blob 크기", e))?
                .size();
            if !similarity::sizes_may_match(size, dest_size) {
                continue;
            }
            compared += 1;
            if compared > MAX_RENAME_CANDIDATES {
                break;
            }
            let data = repo
                .find_object(id)
                .map_err(|e| err("blob 읽기", e))?
                .detach()
                .data;
            let score = similarity::score(
                &Spans::new(&data, !crate::text::is_binary(&data)),
                &dest_spans,
            );
            if score < MINIMUM_SCORE {
                continue;
            }
            // 이름이 닮았고 아주 비슷하면(75% 이상) 다른 후보를 보지 않는다(git의 같은 파일 이름 지름길과 같은 기준).
            if affinity > 0 && score >= (MINIMUM_SCORE + similarity::MAX_SCORE) / 2 {
                return Ok(Some(candidate.to_owned()));
            }
            let named = affinity == AFFINITY_SAME_NAME;
            if best.is_none_or(|(s, n, _)| score > s || (score == s && named && !n)) {
                best = Some((score, named, candidate));
            }
        }
        Ok(best.map(|(.., candidate)| candidate.to_owned()))
    }
}

const AFFINITY_SAME_NAME: u8 = 2;

/// 이름이 얼마나 닮았나: 파일 이름이 같으면 2, 확장자를 뺀 이름이 같으면 1(`git-log.txt` → `git-log.adoc`).
fn affinity(candidate: &str, path: &str) -> u8 {
    let (a, b) = (file_name(candidate), file_name(path));
    if a == b {
        AFFINITY_SAME_NAME
    } else if stem(a) == stem(b) {
        1
    } else {
        0
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// 트리 비교 결과 하나(gix).
pub(crate) type TreeChange = gix::object::tree::diff::ChangeDetached;

/// 열어 둔 저장소 캐시. 같은 저장소를 다시 열지 않아 gix의 객체 캐시를 이어 쓰고,
/// 디렉터리 → 저장소 찾기 결과도 기억한다(하위 모듈처럼 중첩된 저장소도 디렉터리마다 정확하다).
#[derive(Default)]
pub struct RepoCache {
    inner: Mutex<CacheInner>,
}

#[derive(Default)]
struct CacheInner {
    by_dir: HashMap<PathBuf, PathBuf>,
    repos: HashMap<PathBuf, Arc<Repo>>,
}

impl RepoCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// `path`(파일·디렉터리, 지금은 없는 경로도 된다)를 담은 저장소.
    pub fn for_path(&self, path: &Path) -> Result<Arc<Repo>> {
        let dir = existing_ancestor(path)
            .map(|p| {
                if p.is_dir() {
                    p
                } else {
                    p.parent().unwrap_or(p)
                }
            })
            .ok_or_else(|| Error(format!("경로가 없다: {}", path.display())))?
            .to_path_buf();
        {
            let inner = self.inner.lock().unwrap();
            if let Some(repo) = inner.by_dir.get(&dir).and_then(|w| inner.repos.get(w)) {
                return Ok(repo.clone());
            }
        }
        // 저장소 열기는 파일을 읽으므로 잠금 밖에서 한다.
        let repo = Repo::discover(&dir)?;
        let mut inner = self.inner.lock().unwrap();
        let workdir = repo.workdir().to_path_buf();
        inner.by_dir.insert(dir, workdir.clone());
        Ok(inner
            .repos
            .entry(workdir)
            .or_insert_with(|| Arc::new(repo))
            .clone())
    }
}

/// 사람(작성자·커미터)과 시각.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub email: String,
    /// Unix 시각(초)
    pub time: i64,
    /// UTC와의 차이(초)
    pub tz_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub sha: String,
    pub parents: Vec<String>,
    pub summary: String,
    pub message: String,
    pub author: Person,
    pub committer: Person,
    /// 첫 부모와 비교해 바뀐 파일 수
    pub changed_files: usize,
}

pub(crate) fn person(signature: gix::actor::SignatureRef<'_>, encoding: Option<&BStr>) -> Person {
    let signature = signature.trim();
    let time = signature.time().unwrap_or_default();
    Person {
        name: decode_text(signature.name, encoding),
        email: decode_text(signature.email, encoding),
        time: time.seconds,
        tz_offset: time.offset,
    }
}

/// 커밋의 `encoding` 헤더(예: euc-kr)에 맞춰 문자열로 바꾼다. 헤더가 없으면 UTF-8이다.
/// 디코드할 수 없는 바이트는 U+FFFD로 바꾼다.
pub fn decode_text(bytes: &BStr, encoding: Option<&BStr>) -> String {
    if let Some(label) = encoding
        && let Some(encoding) = encoding_rs::Encoding::for_label(label.trim())
        && encoding != encoding_rs::UTF_8
    {
        let (text, _, _) = encoding.decode(bytes);
        return text.into_owned();
    }
    bytes.to_str_lossy().into_owned()
}

/// `path`부터 위로 올라가며 처음 존재하는 경로.
fn existing_ancestor(path: &Path) -> Option<&Path> {
    path.ancestors().find(|p| p.exists())
}

/// 존재하는 가장 가까운 조상까지 canonicalize하고 나머지(아직 없거나 삭제된 경로)는 그대로 붙인다.
fn canonicalize_lenient(path: &Path) -> Result<PathBuf> {
    let base =
        existing_ancestor(path).ok_or_else(|| Error(format!("경로가 없다: {}", path.display())))?;
    let mut out = std::fs::canonicalize(base).map_err(|e| err("경로", e))?;
    if let Ok(rest) = path.strip_prefix(base) {
        out.push(rest);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_euc_kr_message() {
        let bytes = b"\xbe\xc8\xb3\xe7\xc7\xcf\xbc\xbc\xbf\xe4";
        assert_eq!(
            decode_text(bytes.as_bstr(), Some(b"euc-kr".as_bstr())),
            "안녕하세요"
        );
        assert_eq!(decode_text("안녕".as_bytes().as_bstr(), None), "안녕");
        assert_eq!(decode_text(b"\xff".as_bstr(), None), "\u{fffd}");
    }
}
