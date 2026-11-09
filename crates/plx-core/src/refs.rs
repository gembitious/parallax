//! 브랜치·태그 목록(M14). 로컬 브랜치, 원격 브랜치, 태그를 gix로 읽어(git을 띄우지 않는다) 끝 커밋의 요약·시각과
//! 그 브랜치를 체크아웃한 워크트리를 붙인다. 최근 커밋부터.
//!
//! 로컬 브랜치의 upstream과 앞섬·뒤처짐은 [`tracking`]이 `git for-each-ref`를 한 번 띄워 받는다(목록은 먼저 그린다).
//! git이 세는 그대로다(`git status`·`git branch -vv`의 수와 같다): upstream 설정(`branch.<이름>.merge`, `remote.<이름>.fetch`
//! 매핑)을 다시 풀거나 두 커밋의 공통 조상을 우리가 따로 세면 git과 어긋날 수 있다. 읽기만 한다.

use std::collections::HashMap;

use gix::ObjectId;
use gix::prelude::Find;
use serde::{Deserialize, Serialize};

use crate::repo::{Error, Repo, Result, decode_text};
use crate::worktree;

/// 한 번에 보일 참조 수의 한도(태그가 아주 많은 저장소). 넘으면 최근 것만.
pub const LIMIT: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// `refs/heads/*`
    Branch,
    /// `refs/remotes/*` (`origin/HEAD` 같은 기호 참조는 뺀다)
    Remote,
    /// `refs/tags/*`
    Tag,
}

/// 그 브랜치를 체크아웃한 워크트리
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefWorktree {
    pub name: String,
    /// 작업트리 경로
    pub path: String,
    /// 지금 보고 있는 작업트리
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefItem {
    pub kind: Kind,
    /// 짧은 이름(`main`, `origin/main`, `v1.0`)
    pub name: String,
    /// 전체 이름(`refs/heads/main`)
    pub full: String,
    /// 가리키는 커밋(태그는 벗긴 커밋). 커밋까지 벗기지 못하면(트리·blob을 가리키는 태그, 없는 객체) 참조가 가리키는 객체
    pub sha: String,
    /// 지금 워크트리의 브랜치(HEAD가 가리킨다)
    pub current: bool,
    /// 이 브랜치를 체크아웃한 워크트리들(지금 워크트리 포함)
    pub worktrees: Vec<RefWorktree>,
    /// 태그 객체가 있는 태그(`git tag -a`)
    pub annotated: bool,
    /// 끝 커밋의 요약. 커밋이 아니거나 없으면 없다
    pub summary: Option<String>,
    pub author: Option<String>,
    /// 끝 커밋의 커미터 시각(초). 없으면 0
    pub time: i64,
    /// 그 시각의 UTC와의 차이(초)
    pub tz_offset: i32,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefList {
    /// 작업트리 경로
    pub repo: String,
    /// HEAD 커밋(빈 저장소면 없다)
    pub head: Option<String>,
    /// HEAD가 브랜치에 붙어 있지 않다
    pub detached: bool,
    /// 종류마다 최근 커밋부터(지금 브랜치는 맨 위)
    pub refs: Vec<RefItem>,
    /// [`LIMIT`]에서 멈췄다
    pub truncated: bool,
}

/// 참조를 모두 읽는다.
pub fn list(repo: &Repo) -> Result<RefList> {
    let local = repo.local();
    let current = repo.head_branch()?;
    let head = local.head_id().ok().map(|id| id.detach().to_string());
    let detached = head.is_some() && current.is_none();

    // 브랜치 → 체크아웃한 워크트리들
    let mut checked: HashMap<String, Vec<RefWorktree>> = HashMap::new();
    for tree in worktree::list(repo).unwrap_or_default() {
        if let Some(branch) = tree.branch {
            checked.entry(branch).or_default().push(RefWorktree {
                name: tree.name,
                path: tree.path,
                current: tree.current,
            });
        }
    }

    let platform = local
        .references()
        .map_err(|e| Error(format!("참조: {e}")))?;
    let mut found: Vec<(Kind, String, String, ObjectId)> = Vec::new();
    for reference in platform.all().map_err(|e| Error(format!("참조: {e}")))? {
        let Ok(reference) = reference else { continue };
        let gix::refs::TargetRef::Object(id) = reference.target() else {
            continue;
        };
        let full = reference.name().as_bstr();
        let (kind, short) = if let Some(short) = full.strip_prefix(b"refs/heads/") {
            (Kind::Branch, short)
        } else if let Some(short) = full.strip_prefix(b"refs/remotes/") {
            (Kind::Remote, short)
        } else if let Some(short) = full.strip_prefix(b"refs/tags/") {
            (Kind::Tag, short)
        } else {
            continue;
        };
        // UTF-8이 아닌 이름은 그 이름으로 git에 넘길 수 없다(다른 화면처럼 뺀다).
        let (Ok(name), Ok(full)) = (std::str::from_utf8(short), std::str::from_utf8(full)) else {
            continue;
        };
        found.push((kind, name.to_owned(), full.to_owned(), id.to_owned()));
    }

    let mut buf = Vec::new();
    let mut commits: HashMap<ObjectId, Option<(String, String, i64, i32)>> = HashMap::new();
    let mut refs: Vec<RefItem> = found
        .into_iter()
        .map(|(kind, name, full, id)| {
            let (target, annotated) = peel(&local, id);
            let commit = commits
                .entry(target)
                .or_insert_with(|| commit_summary(&local, target, &mut buf))
                .clone();
            let is_current = kind == Kind::Branch && current.as_deref() == Some(name.as_str());
            RefItem {
                worktrees: if kind == Kind::Branch {
                    checked.remove(&name).unwrap_or_default()
                } else {
                    Vec::new()
                },
                kind,
                current: is_current,
                full,
                name,
                sha: target.to_string(),
                annotated,
                exists: commit.is_some(),
                time: commit.as_ref().map_or(0, |c| c.2),
                tz_offset: commit.as_ref().map_or(0, |c| c.3),
                author: commit.as_ref().map(|c| c.1.clone()),
                summary: commit.map(|c| c.0),
            }
        })
        .collect();
    refs.sort_by(|a, b| {
        let rank = |r: &RefItem| match r.kind {
            Kind::Branch => u8::from(!r.current),
            Kind::Remote => 2,
            Kind::Tag => 3,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| b.time.cmp(&a.time))
            .then_with(|| a.name.cmp(&b.name))
    });
    let truncated = refs.len() > LIMIT;
    refs.truncate(LIMIT);
    Ok(RefList {
        repo: repo.workdir().to_string_lossy().into_owned(),
        head,
        detached,
        refs,
        truncated,
    })
}

/// 태그 객체를 커밋(끝)까지 벗긴다: (벗긴 객체, 태그 객체였나)
fn peel(repo: &gix::Repository, id: ObjectId) -> (ObjectId, bool) {
    match repo.find_header(id).map(|h| h.kind()) {
        Ok(gix::object::Kind::Tag) => match repo
            .find_object(id)
            .ok()
            .and_then(|o| o.peel_tags_to_end().ok())
        {
            Some(object) => (object.id, true),
            None => (id, true),
        },
        _ => (id, false),
    }
}

/// (요약, 작성자 이름, 커미터 시각, UTC와의 차이). 커밋이 아니거나 없으면 `None`
fn commit_summary(
    repo: &gix::Repository,
    id: ObjectId,
    buf: &mut Vec<u8>,
) -> Option<(String, String, i64, i32)> {
    let data = repo.objects.try_find(&id, buf).ok()??;
    let gix::objs::ObjectRef::Commit(commit) = data.decode().ok()? else {
        return None;
    };
    let encoding = commit.encoding;
    let message = decode_text(commit.message, encoding);
    let summary = message.lines().next().unwrap_or("").trim().to_owned();
    let author = commit
        .author()
        .map(|a| decode_text(a.trim().name, encoding))
        .unwrap_or_default();
    let time = commit
        .committer()
        .ok()
        .and_then(|c| c.time().ok())
        .unwrap_or_default();
    Some((summary, author, time.seconds, time.offset))
}

// ---------------------------------------------------------------- upstream·앞섬/뒤처짐

/// 로컬 브랜치 하나의 upstream과 앞섬·뒤처짐
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracking {
    /// 로컬 브랜치 전체 이름(`refs/heads/main`)
    pub full: String,
    /// upstream 전체 이름(`refs/remotes/origin/main`, 다른 로컬 브랜치면 `refs/heads/…`)
    pub upstream: String,
    /// upstream에 없는 이 브랜치의 커밋 수
    pub ahead: u64,
    /// 이 브랜치에 없는 upstream의 커밋 수
    pub behind: u64,
    /// upstream을 설정했지만 그 참조가 없다(원격에서 지웠다: `git branch -vv`의 `gone`)
    pub gone: bool,
}

/// `git for-each-ref`의 형식. 줄마다 `<브랜치>\0<upstream>\0<track>`. 참조 이름에는 줄바꿈·NUL이 없다.
const FORMAT: &str = "--format=%(refname)%00%(upstream)%00%(upstream:track,nobracket)";

/// 로컬 브랜치마다 upstream과 앞섬·뒤처짐(upstream이 없는 브랜치는 뺀다). git을 한 번 띄운다.
pub fn tracking(repo: &Repo) -> Result<Vec<Tracking>> {
    let out = crate::git::output(repo.workdir(), ["for-each-ref", FORMAT, "refs/heads"])
        .map_err(|e| Error(format!("upstream 읽기: {e}")))?;
    Ok(parse_tracking(&out))
}

/// [`FORMAT`] 출력을 읽는다. for-each-ref는 track 글을 번역하지 않는다(`ahead 1, behind 2`, `gone`).
/// 모르는 모양의 줄은 건너뛴다.
pub fn parse_tracking(out: &[u8]) -> Vec<Tracking> {
    let mut list = Vec::new();
    for line in out.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let mut fields = line.split(|&b| b == 0);
        let (Some(full), Some(upstream), Some(track), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if full.is_empty() || upstream.is_empty() {
            continue;
        }
        let (Ok(full), Ok(upstream), Ok(track)) = (
            std::str::from_utf8(full),
            std::str::from_utf8(upstream),
            std::str::from_utf8(track),
        ) else {
            continue;
        };
        let Some((ahead, behind, gone)) = parse_track(track) else {
            continue;
        };
        list.push(Tracking {
            full: full.to_owned(),
            upstream: upstream.to_owned(),
            ahead,
            behind,
            gone,
        });
    }
    list
}

/// `""`(같다), `ahead 3`, `behind 2`, `ahead 3, behind 2`, `gone` → (앞섬, 뒤처짐, gone)
fn parse_track(track: &str) -> Option<(u64, u64, bool)> {
    let track = track.trim();
    if track.is_empty() {
        return Some((0, 0, false));
    }
    if track == "gone" {
        return Some((0, 0, true));
    }
    let (mut ahead, mut behind) = (None, None);
    for part in track.split(", ") {
        let (word, count) = part.split_once(' ')?;
        let count: u64 = count.parse().ok()?;
        let slot = match word {
            "ahead" => &mut ahead,
            "behind" => &mut behind,
            _ => return None,
        };
        if slot.replace(count).is_some() {
            return None;
        }
    }
    Some((ahead.unwrap_or(0), behind.unwrap_or(0), false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_track() {
        assert_eq!(parse_track(""), Some((0, 0, false)));
        assert_eq!(parse_track("gone"), Some((0, 0, true)));
        assert_eq!(parse_track("ahead 3"), Some((3, 0, false)));
        assert_eq!(parse_track("behind 12"), Some((0, 12, false)));
        assert_eq!(parse_track("ahead 1, behind 2"), Some((1, 2, false)));
        assert_eq!(parse_track("ahead x"), None);
        assert_eq!(parse_track("앞섬 1"), None);
        assert_eq!(parse_track("ahead 1, ahead 2"), None);
    }

    #[test]
    fn parses_lines() {
        let out = b"refs/heads/main\0refs/remotes/origin/main\0ahead 1, behind 2\n\
refs/heads/solo\0\0\n\
refs/heads/old\0refs/remotes/origin/old\0gone\n\
refs/heads/same\0refs/heads/main\0\n\
broken line\n";
        let list = parse_tracking(out);
        assert_eq!(list.len(), 3);
        assert_eq!(
            list[0],
            Tracking {
                full: "refs/heads/main".into(),
                upstream: "refs/remotes/origin/main".into(),
                ahead: 1,
                behind: 2,
                gone: false
            }
        );
        assert!(list[1].gone);
        assert_eq!((list[2].ahead, list[2].behind), (0, 0));
    }
}
