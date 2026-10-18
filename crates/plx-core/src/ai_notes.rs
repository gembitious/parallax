//! AI 출처(M5): git-ai가 커밋마다 `refs/notes/ai`에 붙이는 authorship 로그(Git AI Standard v3.0.0,
//! `authorship/3.0.0`)를 읽어 blame 줄을 AI 에이전트가 썼는지 알려 준다.
//!
//! 로그는 그 커밋 시점 파일의 줄 번호로 적힌다. 그래서 `git-ai blame`과 같이 blame이 준 원래 커밋·원래 경로·원래 줄로
//! 찾는다. 한 줄을 덮는 항목이 여럿이면 뒤의 항목이 이긴다. 메타데이터에 없는 키는 건너뛰어 그 앞 항목을 보고,
//! 옛 형식 키(16자 hex)는 다른 커밋의 노트(cherry-pick으로 옮겨진 줄)에서도 찾는다. 키 종류:
//! - `s_<14hex>::t_<14hex>`: 세션(`metadata.sessions`, `::` 앞이 세션 id)
//! - `h_<14hex>`: IDE 확장이 본 사람(`metadata.humans`)
//! - 접두어 없음: 옛 형식 프롬프트(`metadata.prompts`)
//!
//! 노트는 notes 커밋의 트리에서 직접 읽는다(git을 띄우지 않는다). 트리 이름은 주석 단 커밋의 sha이고 노트가 많으면
//! `ab/cdef…`처럼 나뉜다(fanout). 해석한 노트는 blob id로 기억한다. git-ai는 커밋마다 노트를 붙여 notes 참조가
//! 자주 움직이지만, 그대로인 노트는 blob도 그대로라 다시 읽지 않는다.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gix::ObjectId;
use serde::{Deserialize, Serialize};

use crate::repo::{Error, Repo, Result};
use crate::requests::lock_or_reset;

/// git-ai가 쓰는 노트 참조
pub const NOTES_REF: &str = "refs/notes/ai";

/// 노트 하나(커밋 하나의 authorship 로그)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuthorshipLog {
    pub files: Vec<FileAttestation>,
    pub metadata: Metadata,
}

/// 파일 하나의 항목들(적힌 순서)
#[derive(Debug, Clone, PartialEq)]
pub struct FileAttestation {
    pub path: String,
    pub entries: Vec<Entry>,
}

/// 키 하나와 그 키가 쓴 줄 범위(양쪽 포함, 1부터)
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    pub ranges: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Metadata {
    #[serde(default)]
    pub schema_version: String,
    #[serde(default)]
    pub git_ai_version: Option<String>,
    #[serde(default)]
    pub base_commit_sha: String,
    #[serde(default)]
    pub prompts: HashMap<String, PromptRecord>,
    #[serde(default)]
    pub sessions: HashMap<String, SessionRecord>,
    #[serde(default)]
    pub humans: HashMap<String, HumanRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Deserialize)]
pub struct AgentId {
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
}

/// 옛 형식 프롬프트(v1.4.0 전)
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PromptRecord {
    pub agent_id: AgentId,
    #[serde(default)]
    pub human_author: Option<String>,
    #[serde(default)]
    pub messages_url: Option<String>,
    #[serde(default)]
    pub total_additions: u32,
    #[serde(default)]
    pub total_deletions: u32,
    #[serde(default)]
    pub accepted_lines: u32,
    #[serde(default)]
    pub overriden_lines: u32,
    #[serde(default)]
    pub custom_attributes: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SessionRecord {
    pub agent_id: AgentId,
    #[serde(default)]
    pub human_author: Option<String>,
    #[serde(default)]
    pub custom_attributes: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HumanRecord {
    #[serde(default)]
    pub author: String,
}

/// 줄을 쓴 이
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Author {
    /// AI 에이전트(세션이나 옛 형식 프롬프트)
    #[serde(rename_all = "camelCase")]
    Agent {
        /// 예: `claude`, `cursor`, `codex`
        tool: String,
        model: String,
        /// 그 도구에서의 대화 id
        id: String,
        /// 에이전트에게 시킨 사람
        human: Option<String>,
        messages_url: Option<String>,
        /// 설정에서 온 사용자 속성(키 순)
        attributes: Vec<(String, String)>,
        /// 옛 형식 프롬프트에만 있다
        stats: Option<PromptStats>,
    },
    /// IDE 확장이 사람이 쳤다고 본 줄
    Human { author: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptStats {
    pub additions: u32,
    pub deletions: u32,
    pub accepted: u32,
    pub overridden: u32,
}

impl From<&PromptRecord> for Author {
    fn from(p: &PromptRecord) -> Author {
        Author::Agent {
            tool: p.agent_id.tool.clone(),
            model: p.agent_id.model.clone(),
            id: p.agent_id.id.clone(),
            human: p.human_author.clone(),
            messages_url: p.messages_url.clone(),
            attributes: attributes(&p.custom_attributes),
            stats: Some(PromptStats {
                additions: p.total_additions,
                deletions: p.total_deletions,
                accepted: p.accepted_lines,
                overridden: p.overriden_lines,
            }),
        }
    }
}

impl From<&SessionRecord> for Author {
    fn from(s: &SessionRecord) -> Author {
        Author::Agent {
            tool: s.agent_id.tool.clone(),
            model: s.agent_id.model.clone(),
            id: s.agent_id.id.clone(),
            human: s.human_author.clone(),
            messages_url: None,
            attributes: attributes(&s.custom_attributes),
            stats: None,
        }
    }
}

fn attributes(map: &Option<BTreeMap<String, String>>) -> Vec<(String, String)> {
    map.iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// 노트 글을 읽는다. git-ai의 파서와 같이 빈 줄은 건너뛰고, 항목이 없는 파일은 버리고, 경로가 `"`로 감싸였으면 벗긴다.
pub fn parse(text: &str) -> std::result::Result<AuthorshipLog, String> {
    let lines: Vec<&str> = text.lines().collect();
    let divider = lines
        .iter()
        .position(|&line| line == "---")
        .ok_or("구분선 '---'이 없다")?;
    let mut files: Vec<FileAttestation> = Vec::new();
    let mut current: Option<FileAttestation> = None;
    for line in &lines[..divider] {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(entry) = line.strip_prefix("  ") {
            let (key, ranges) = entry
                .split_once(' ')
                .ok_or_else(|| format!("항목 모양이 아니다: {entry}"))?;
            let ranges = parse_ranges(ranges)?;
            current
                .as_mut()
                .ok_or("파일 경로 없이 항목이 나왔다")?
                .entries
                .push(Entry {
                    key: key.to_owned(),
                    ranges,
                });
        } else {
            if let Some(file) = current.take().filter(|f| !f.entries.is_empty()) {
                files.push(file);
            }
            let path = if line.len() >= 2 && line.starts_with('"') && line.ends_with('"') {
                &line[1..line.len() - 1]
            } else {
                line
            };
            current = Some(FileAttestation {
                path: path.to_owned(),
                entries: Vec::new(),
            });
        }
    }
    if let Some(file) = current.filter(|f| !f.entries.is_empty()) {
        files.push(file);
    }
    let json = lines[divider + 1..].join("\n");
    let metadata: Metadata =
        serde_json::from_str(&json).map_err(|e| format!("메타데이터 JSON: {e}"))?;
    Ok(AuthorshipLog { files, metadata })
}

/// `1-4,9-10,12` → [(1,4), (9,10), (12,12)]
fn parse_ranges(text: &str) -> std::result::Result<Vec<(u32, u32)>, String> {
    let number = |s: &str| {
        s.trim()
            .parse::<u32>()
            .ok()
            .filter(|&n| n > 0)
            .ok_or_else(|| format!("줄 번호가 아니다: {s}"))
    };
    text.split(',')
        .filter(|item| !item.trim().is_empty())
        .map(|item| match item.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (number(a)?, number(b)?);
                Ok((a.min(b), a.max(b)))
            }
            None => number(item).map(|n| (n, n)),
        })
        .collect()
}

impl AuthorshipLog {
    /// 키가 가리키는 이. 이 노트의 메타데이터에 없으면 옛 형식 키만 `foreign`(다른 노트)에서 찾는다.
    pub fn resolve(
        &self,
        key: &str,
        foreign: &mut dyn FnMut(&str) -> Option<PromptRecord>,
    ) -> Option<Author> {
        let meta = &self.metadata;
        if key.starts_with("h_") {
            return meta.humans.get(key).map(|h| Author::Human {
                author: h.author.clone(),
            });
        }
        if key.starts_with("s_") {
            let session = key.split("::").next().unwrap_or(key);
            return meta.sessions.get(session).map(Author::from);
        }
        match meta.prompts.get(key) {
            Some(prompt) => Some(Author::from(prompt)),
            None => foreign(key).as_ref().map(Author::from),
        }
    }

    /// 파일의 줄 구간별 작성자: 겹치지 않고 시작 순. 뒤 항목이 앞 항목을 덮고, 찾을 수 없는 키는 건너뛴다
    /// (`git-ai blame`이 항목을 뒤에서부터 보며 찾을 수 있는 첫 항목을 쓰는 것과 같다).
    pub fn spans(
        &self,
        path: &str,
        foreign: &mut dyn FnMut(&str) -> Option<PromptRecord>,
    ) -> Vec<(u32, u32, Author)> {
        let Some(file) = self.files.iter().find(|f| f.path == path) else {
            return Vec::new();
        };
        let mut spans: Vec<(u32, u32, Author)> = Vec::new();
        for entry in &file.entries {
            let Some(author) = self.resolve(&entry.key, foreign) else {
                continue;
            };
            for &(start, end) in &entry.ranges {
                paint(&mut spans, start, end, &author);
            }
        }
        spans
    }
}

/// `[start, end]`를 `author`로 칠한다(겹치는 앞 구간은 잘라 낸다). `spans`는 겹치지 않고 시작 순이다.
fn paint(spans: &mut Vec<(u32, u32, Author)>, start: u32, end: u32, author: &Author) {
    let mut out = Vec::with_capacity(spans.len() + 2);
    for (s, e, a) in spans.drain(..) {
        if e < start || s > end {
            out.push((s, e, a));
            continue;
        }
        if s < start {
            out.push((s, start - 1, a.clone()));
        }
        if e > end {
            out.push((end + 1, e, a));
        }
    }
    out.push((start, end, author.clone()));
    out.sort_by_key(|&(s, _, _)| s);
    // 같은 작성자의 붙은 구간은 하나로
    let mut merged: Vec<(u32, u32, Author)> = Vec::with_capacity(out.len());
    for (s, e, a) in out {
        match merged.last_mut() {
            Some(last) if last.2 == a && last.1.checked_add(1) == Some(s) => last.1 = e,
            _ => merged.push((s, e, a)),
        }
    }
    *spans = merged;
}

/// notes 커밋 하나의 노트 목록(주석 단 커밋 → 노트 blob)
#[derive(Debug)]
pub struct NotesTree {
    pub tip: ObjectId,
    notes: HashMap<ObjectId, ObjectId>,
}

impl NotesTree {
    /// `refs/notes/ai`. 없으면 `None`.
    pub fn open(repo: &gix::Repository) -> Result<Option<NotesTree>> {
        let Some(mut reference) = repo
            .try_find_reference(NOTES_REF)
            .map_err(|e| Error(format!("{NOTES_REF}: {e}")))?
        else {
            return Ok(None);
        };
        let tip = reference
            .peel_to_id()
            .map_err(|e| Error(format!("{NOTES_REF}: {e}")))?
            .detach();
        let tree = repo
            .find_commit(tip)
            .and_then(|c| c.tree_id().map(|id| id.detach()))
            .map_err(|e| Error(format!("{NOTES_REF} 커밋: {e}")))?;
        let mut notes = HashMap::new();
        collect(repo, tree, String::new(), &mut notes)?;
        Ok(Some(NotesTree { tip, notes }))
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// 주석 단 커밋들
    pub fn commits(&self) -> impl Iterator<Item = &ObjectId> {
        self.notes.keys()
    }

    /// `commit`의 노트 blob. 없으면 `None`.
    pub fn blob(&self, commit: ObjectId) -> Option<ObjectId> {
        self.notes.get(&commit).copied()
    }

    /// `commit`의 노트 글. 없으면 `None`.
    pub fn read(&self, repo: &gix::Repository, commit: ObjectId) -> Result<Option<String>> {
        self.blob(commit)
            .map(|blob| read_blob(repo, blob))
            .transpose()
    }
}

fn read_blob(repo: &gix::Repository, blob: ObjectId) -> Result<String> {
    let object = repo
        .find_object(blob)
        .map_err(|e| Error(format!("노트 {blob}: {e}")))?;
    Ok(String::from_utf8_lossy(&object.data).into_owned())
}

/// 노트 blob을 읽어 해석한다. 못 읽거나 형식이 틀리면 `None`(그 노트는 없는 것으로 본다).
fn parse_blob(repo: &gix::Repository, blob: ObjectId) -> Option<AuthorshipLog> {
    read_blob(repo, blob)
        .ok()
        .and_then(|text| parse(&text).ok())
}

/// 트리를 내려가며 이름(16진수)을 이어 붙여 sha가 되는 blob을 모은다(fanout 깊이와 무관).
fn collect(
    repo: &gix::Repository,
    tree: ObjectId,
    prefix: String,
    out: &mut HashMap<ObjectId, ObjectId>,
) -> Result<()> {
    let object = repo
        .find_object(tree)
        .map_err(|e| Error(format!("노트 트리: {e}")))?;
    let tree = object
        .try_into_tree()
        .map_err(|e| Error(format!("노트 트리: {e}")))?;
    let decoded = tree
        .decode()
        .map_err(|e| Error(format!("노트 트리: {e}")))?;
    let hex_len = repo.object_hash().len_in_hex();
    for entry in &decoded.entries {
        let Ok(name) = std::str::from_utf8(entry.filename) else {
            continue;
        };
        if !name.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let path = format!("{prefix}{name}");
        if entry.mode.is_tree() {
            if path.len() < hex_len {
                collect(repo, entry.oid.to_owned(), path, out)?;
            }
        } else if path.len() == hex_len
            && let Ok(id) = ObjectId::from_hex(path.as_bytes())
        {
            out.insert(id, entry.oid.to_owned());
        }
    }
    Ok(())
}

/// 요청 하나: 커밋과 그 커밋에서의 경로(blame의 원래 경로)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct AiItem {
    pub sha: String,
    pub path: String,
}

/// 응답: 작성자 목록과 (커밋, 경로)마다 구간 `[시작, 끝, 작성자 번호]`(그 커밋 버전의 줄, 시작 순)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAttribution {
    /// 저장소에 `refs/notes/ai`가 있는가(없으면 더 물을 것이 없다)
    pub notes: bool,
    pub authors: Vec<Author>,
    /// 구간이 하나라도 있는 것만
    pub files: Vec<AiFile>,
    /// 이번에 읽어 해석한 노트 수(앞서 해석한 노트는 다시 읽지 않는다. 측정용)
    pub parsed: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFile {
    pub sha: String,
    pub path: String,
    pub spans: Vec<[u32; 3]>,
}

/// 저장소마다 읽은 노트를 기억한다(notes 참조가 바뀌면 트리를 다시 읽고 바뀐 노트만 다시 해석한다).
/// 앱 전체가 하나를 나눠 쓴다.
#[derive(Default)]
pub struct AiNotes {
    repos: Mutex<HashMap<PathBuf, Arc<Mutex<Option<Cached>>>>>,
}

struct Cached {
    tree: NotesTree,
    /// 해석한 노트(blob → 로그, 못 읽은 노트는 None). 지금 트리에 있는 blob만 남긴다.
    logs: HashMap<ObjectId, Option<Arc<AuthorshipLog>>>,
    /// 옛 형식 키 → 프롬프트(처음 필요할 때 모든 노트에서 모은다)
    foreign: Option<Foreign>,
    /// 이번 요청에서 해석한 노트 수
    parsed: usize,
}

/// 모든 노트의 옛 형식 프롬프트. 참조가 움직이면 새 노트만 더하고, 노트가 빠지거나 바뀌면(옛 blob이 트리에 없으면)
/// 처음부터 다시 모은다.
struct Foreign {
    /// 이 트리까지 모았다
    tip: ObjectId,
    /// 모은 노트 blob
    blobs: HashSet<ObjectId>,
    prompts: HashMap<String, PromptRecord>,
}

impl AiNotes {
    pub fn new() -> AiNotes {
        AiNotes::default()
    }

    /// (커밋, 경로)들의 AI 작성 구간. 저장소에 노트가 없으면 `notes: false`.
    pub fn attribution(&self, repo: &Repo, items: &[AiItem]) -> Result<AiAttribution> {
        let local = repo.local();
        let slot = self
            .repos
            .lock()
            .unwrap()
            .entry(repo.workdir().to_path_buf())
            .or_default()
            .clone();
        // 노트를 읽고 해석하는 동안 쥐는 잠금이라 패닉이 지나갔으면 처음부터 다시 읽는다.
        let mut guard = lock_or_reset(&slot);
        // 참조가 그대로면 앞서 읽은 트리를 쓴다.
        let tip = local
            .try_find_reference(NOTES_REF)
            .ok()
            .flatten()
            .and_then(|mut r| r.peel_to_id().ok().map(|id| id.detach()));
        let Some(tip) = tip else {
            *guard = None;
            return Ok(AiAttribution {
                notes: false,
                authors: Vec::new(),
                files: Vec::new(),
                parsed: 0,
            });
        };
        if guard.as_ref().is_none_or(|c| c.tree.tip != tip) {
            let tree =
                NotesTree::open(&local)?.ok_or_else(|| Error(format!("{NOTES_REF}가 없다")))?;
            *guard = Some(match guard.take() {
                Some(old) => old.moved(tree),
                None => Cached {
                    tree,
                    logs: HashMap::new(),
                    foreign: None,
                    parsed: 0,
                },
            });
        }
        let cached = guard.as_mut().expect("방금 채웠다");
        cached.parsed = 0;

        let mut authors: Vec<Author> = Vec::new();
        let mut index: HashMap<Author, u32> = HashMap::new();
        let mut files = Vec::new();
        for item in items {
            let Ok(commit) = ObjectId::from_hex(item.sha.as_bytes()) else {
                continue;
            };
            let Some(log) = cached.log(&local, commit) else {
                continue;
            };
            let Cached {
                tree,
                foreign,
                parsed,
                ..
            } = &mut *cached;
            let mut lookup = |key: &str| foreign_prompt(&local, tree, foreign, parsed, key);
            let spans: Vec<[u32; 3]> = log
                .spans(&item.path, &mut lookup)
                .into_iter()
                .map(|(start, end, author)| {
                    let next = authors.len() as u32;
                    let i = *index.entry(author.clone()).or_insert_with(|| {
                        authors.push(author);
                        next
                    });
                    [start, end, i]
                })
                .collect();
            if !spans.is_empty() {
                files.push(AiFile {
                    sha: item.sha.clone(),
                    path: item.path.clone(),
                    spans,
                });
            }
        }
        Ok(AiAttribution {
            notes: true,
            authors,
            files,
            parsed: cached.parsed,
        })
    }
}

impl Cached {
    /// notes 참조가 `tree`로 움직였다. 그대로인 노트(같은 blob)의 해석과 모은 프롬프트는 그대로 쓴다.
    fn moved(self, tree: NotesTree) -> Cached {
        let live: HashSet<ObjectId> = tree.notes.values().copied().collect();
        let mut logs = self.logs;
        logs.retain(|blob, _| live.contains(blob));
        // 빠진 노트가 있으면 그 노트의 프롬프트를 가려낼 수 없어 처음부터 다시 모은다(드물다).
        let foreign = self
            .foreign
            .filter(|foreign| foreign.blobs.iter().all(|blob| live.contains(blob)));
        Cached {
            tree,
            logs,
            foreign,
            parsed: 0,
        }
    }

    fn log(&mut self, repo: &gix::Repository, commit: ObjectId) -> Option<Arc<AuthorshipLog>> {
        let blob = self.tree.blob(commit)?;
        if let Some(log) = self.logs.get(&blob) {
            return log.clone();
        }
        self.parsed += 1;
        let log = parse_blob(repo, blob).map(Arc::new);
        self.logs.insert(blob, log.clone());
        log
    }
}

/// 다른 노트에 적힌 옛 형식 프롬프트(cherry-pick으로 옮겨진 줄). 처음 부를 때 모든 노트의 프롬프트를 모으고,
/// 그 뒤 트리가 바뀌었으면 아직 안 본 노트만 더한다.
fn foreign_prompt(
    repo: &gix::Repository,
    tree: &NotesTree,
    foreign: &mut Option<Foreign>,
    parsed: &mut usize,
    key: &str,
) -> Option<PromptRecord> {
    let all = foreign.get_or_insert_with(|| Foreign {
        tip: ObjectId::null(repo.object_hash()),
        blobs: HashSet::new(),
        prompts: HashMap::new(),
    });
    if all.tip != tree.tip {
        for &blob in tree.notes.values() {
            if !all.blobs.insert(blob) {
                continue;
            }
            *parsed += 1;
            let Some(log) = parse_blob(repo, blob) else {
                continue;
            };
            for (hash, prompt) in log.metadata.prompts {
                all.prompts.entry(hash).or_insert(prompt);
            }
        }
        all.tip = tree.tip;
    }
    all.prompts.get(key).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Git AI Standard v3.0.0의 1.2.7(섞인 형식) 예
    const MIXED: &str = r#"src/main.rs
  s_c9883b05a2487d::t_9f8e7d6c5b4a32 1-10
  h_31dce776f88375 15-20
src/lib.rs
  abcd1234abcd1234 1-50
---
{
  "schema_version": "authorship/3.0.0",
  "git_ai_version": "1.4.5",
  "base_commit_sha": "7734793b756b3921c88db5375a8c156e9532447b",
  "prompts": {
    "abcd1234abcd1234": {
      "agent_id": {
        "tool": "cursor",
        "id": "old_session_id",
        "model": "gpt-4"
      },
      "human_author": "Developer <dev@example.com>",
      "total_additions": 50,
      "total_deletions": 0,
      "accepted_lines": 50,
      "overriden_lines": 0
    }
  },
  "humans": {
    "h_31dce776f88375": {
      "author": "Developer <dev@example.com>"
    }
  },
  "sessions": {
    "s_c9883b05a2487d": {
      "agent_id": {
        "tool": "claude",
        "id": "conv_abc123",
        "model": "claude-sonnet-4-5-20250514"
      },
      "human_author": "dev@example.com",
      "custom_attributes": { "team": "backend" }
    }
  }
}"#;

    fn none(_: &str) -> Option<PromptRecord> {
        None
    }

    fn tool(author: &Author) -> &str {
        match author {
            Author::Agent { tool, .. } => tool,
            Author::Human { author } => author,
        }
    }

    #[test]
    fn parses_the_spec_example() {
        let log = parse(MIXED).unwrap();
        assert_eq!(log.files.len(), 2);
        assert_eq!(log.files[0].path, "src/main.rs");
        assert_eq!(
            log.files[0].entries[0],
            Entry {
                key: "s_c9883b05a2487d::t_9f8e7d6c5b4a32".into(),
                ranges: vec![(1, 10)]
            }
        );
        assert_eq!(log.metadata.schema_version, "authorship/3.0.0");
        assert_eq!(log.metadata.prompts["abcd1234abcd1234"].accepted_lines, 50);
        assert_eq!(
            log.metadata.sessions["s_c9883b05a2487d"].agent_id.model,
            "claude-sonnet-4-5-20250514"
        );
    }

    #[test]
    fn resolves_each_key_kind() {
        let log = parse(MIXED).unwrap();
        let main = log.spans("src/main.rs", &mut none);
        assert_eq!(main.len(), 2);
        assert_eq!((main[0].0, main[0].1, tool(&main[0].2)), (1, 10, "claude"));
        match &main[0].2 {
            Author::Agent {
                human,
                attributes,
                stats,
                ..
            } => {
                assert_eq!(human.as_deref(), Some("dev@example.com"));
                assert_eq!(attributes, &[("team".to_owned(), "backend".to_owned())]);
                assert_eq!(stats, &None, "세션에는 통계가 없다");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            main[1],
            (
                15,
                20,
                Author::Human {
                    author: "Developer <dev@example.com>".into()
                }
            )
        );
        let lib = log.spans("src/lib.rs", &mut none);
        assert_eq!((lib[0].0, lib[0].1, tool(&lib[0].2)), (1, 50, "cursor"));
        assert!(log.spans("없는.rs", &mut none).is_empty());
    }

    #[test]
    fn later_entries_win_and_unknown_keys_fall_through() {
        let text = "a.rs\n  aaaaaaaaaaaaaaaa 1-10\n  bbbbbbbbbbbbbbbb 4-6\n  s_00000000000000::t_00000000000000 5\n  h_ffffffffffffff 9\n---\n{\"prompts\":{\
            \"aaaaaaaaaaaaaaaa\":{\"agent_id\":{\"tool\":\"cursor\",\"id\":\"1\",\"model\":\"m1\"}},\
            \"bbbbbbbbbbbbbbbb\":{\"agent_id\":{\"tool\":\"claude\",\"id\":\"2\",\"model\":\"m2\"}}}}";
        let log = parse(text).unwrap();
        let spans: Vec<(u32, u32, String)> = log
            .spans("a.rs", &mut none)
            .iter()
            .map(|(s, e, a)| (*s, *e, tool(a).to_owned()))
            .collect();
        // 5(세션 없음)와 9(사람 없음)는 건너뛰어 앞 항목이 남는다.
        assert_eq!(
            spans,
            [
                (1, 3, "cursor".to_owned()),
                (4, 6, "claude".to_owned()),
                (7, 10, "cursor".to_owned())
            ]
        );
    }

    #[test]
    fn foreign_prompts_come_from_other_notes() {
        let text = "a.rs\n  cccccccccccccccc 2-3\n---\n{\"prompts\":{}}";
        let log = parse(text).unwrap();
        assert!(log.spans("a.rs", &mut none).is_empty());
        let mut foreign = |key: &str| {
            (key == "cccccccccccccccc").then(|| PromptRecord {
                agent_id: AgentId {
                    tool: "codex".into(),
                    id: "x".into(),
                    model: "gpt-5".into(),
                },
                human_author: None,
                messages_url: None,
                total_additions: 0,
                total_deletions: 0,
                accepted_lines: 0,
                overriden_lines: 0,
                custom_attributes: None,
            })
        };
        let spans = log.spans("a.rs", &mut foreign);
        assert_eq!((spans[0].0, spans[0].1, tool(&spans[0].2)), (2, 3, "codex"));
    }

    #[test]
    fn quoted_paths_blank_lines_and_crlf() {
        let text = "\"소스/파일 이름.rs\"\r\n  s_11111111111111::t_22222222222222 7,1-2\r\n\r\nempty.rs\r\n---\r\n{\"prompts\":{},\"sessions\":{\"s_11111111111111\":{\"agent_id\":{\"tool\":\"claude\",\"id\":\"c\",\"model\":\"opus\"}}}}";
        let log = parse(text).unwrap();
        assert_eq!(log.files.len(), 1, "항목이 없는 파일은 버린다");
        assert_eq!(log.files[0].path, "소스/파일 이름.rs");
        assert_eq!(log.files[0].entries[0].ranges, [(7, 7), (1, 2)]);
        let spans = log.spans("소스/파일 이름.rs", &mut none);
        assert_eq!(
            spans.iter().map(|s| (s.0, s.1)).collect::<Vec<_>>(),
            [(1, 2), (7, 7)]
        );
    }

    #[test]
    fn rejects_malformed_notes() {
        assert!(parse("a.rs\n  x 1\n").is_err(), "구분선 없음");
        assert!(parse("  x 1\n---\n{}").is_err(), "경로 없는 항목");
        assert!(parse("a.rs\n  x\n---\n{}").is_err(), "범위 없는 항목");
        assert!(parse("a.rs\n  x 0\n---\n{}").is_err(), "0번 줄");
        assert!(parse("a.rs\n  x 1\n---\nnot json").is_err());
        // 메타데이터의 모르는 필드·빠진 필드는 괜찮다
        assert!(parse("a.rs\n  x 1\n---\n{\"future\":1}").is_ok());
    }

    #[test]
    fn painting_merges_and_splits() {
        let a = Author::Human { author: "a".into() };
        let b = Author::Human { author: "b".into() };
        let mut spans = Vec::new();
        paint(&mut spans, 1, 10, &a);
        paint(&mut spans, 3, 4, &b);
        paint(&mut spans, 5, 5, &a);
        assert_eq!(
            spans,
            [(1, 2, a.clone()), (3, 4, b.clone()), (5, 10, a.clone())]
        );
        paint(&mut spans, 1, 20, &b);
        assert_eq!(spans, [(1, 20, b)]);
    }
}
