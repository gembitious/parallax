//! AI 출처(M5)의 골든 테스트: git-ai가 실제로 쓴 노트가 든 저장소(`fixtures/git-ai/*.bundle`)에서, 줄마다 우리가 찾은
//! 에이전트가 `git-ai blame --json`과 같은지 본다. 우리 쪽은 앱과 같은 길이다: git blame의 원래 커밋·원래 경로·원래 줄로
//! (커밋, 경로)의 노트 구간을 찾는다(화면의 `AiOverlay.indexAt`). 번들과 기대값을 만드는 법은 `fixtures/git-ai/README.md`.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use common::{git, scratch_repo};
use plx_core::ai_notes::{AiItem, AiNotes, Author};
use plx_core::blame::Blame;
use plx_core::repo::Repo;
use serde::Deserialize;

#[derive(Deserialize)]
struct Expected {
    git_ai_version: String,
    head: String,
    files: BTreeMap<String, ExpectedFile>,
}

/// `git-ai blame --json`에서 대조에 쓰는 부분
#[derive(Deserialize)]
struct ExpectedFile {
    /// `"3"`·`"3-5"`(지금 파일의 줄) → 키
    lines: BTreeMap<String, String>,
    prompts: BTreeMap<String, Prompt>,
}

#[derive(Deserialize)]
struct Prompt {
    agent_id: AgentId,
    human_author: Option<String>,
}

#[derive(Deserialize)]
struct AgentId {
    tool: String,
    id: String,
    model: String,
}

/// 줄 하나를 쓴 에이전트(도구, 대화 id, 모델, 시킨 사람). 에이전트가 쓰지 않았으면 None.
type Agent = Option<(String, String, String, Option<String>)>;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/git-ai")
}

/// 번들의 모든 참조(refs/notes/ai 포함)를 새 저장소로 가져와 `head`를 꺼낸다.
fn unbundle(name: &str, head: &str) -> PathBuf {
    let repo = scratch_repo(&format!("git-ai-{name}"));
    let bundle = dir().join(format!("{name}.bundle"));
    let bundle = bundle.to_str().expect("UTF-8 경로");
    git(
        &repo,
        &["fetch", "-q", "--update-head-ok", bundle, "+refs/*:refs/*"],
    );
    git(&repo, &["checkout", "-q", "--detach", head]);
    repo
}

fn theirs(file: &ExpectedFile, count: usize) -> Vec<Agent> {
    let mut lines = vec![None; count];
    for (range, key) in &file.lines {
        let (start, end): (usize, usize) = match range.split_once('-') {
            Some((a, b)) => (a.parse().unwrap(), b.parse().unwrap()),
            None => (range.parse().unwrap(), range.parse().unwrap()),
        };
        let prompt = &file.prompts[key];
        for line in start..=end {
            assert!(
                line <= count,
                "git-ai가 {line}번째 줄을 말하지만 파일은 {count}줄이다"
            );
            lines[line - 1] = Some((
                prompt.agent_id.tool.clone(),
                prompt.agent_id.id.clone(),
                prompt.agent_id.model.clone(),
                prompt.human_author.clone(),
            ));
        }
    }
    lines
}

/// `rev`가 None이면 작업트리 blame(앱의 `plx <파일>`), 아니면 그 리비전의 blame
fn ours(repo_dir: &Path, path: &str, rev: Option<&str>) -> Vec<Agent> {
    let mut blocks = Vec::new();
    Blame::spawn(repo_dir, path, rev)
        .expect("git blame 실행")
        .stream(|chunk| blocks.extend(chunk.blocks))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut items: Vec<AiItem> = Vec::new();
    for block in &blocks {
        let item = AiItem {
            sha: block.sha.clone(),
            path: block.path.clone(),
        };
        if !items.contains(&item) {
            items.push(item);
        }
    }
    let repo = Repo::discover(repo_dir).expect("저장소");
    let attribution = AiNotes::new().attribution(&repo, &items).expect("AI 출처");
    assert!(attribution.notes, "{path}: refs/notes/ai가 없다");
    let spans: HashMap<(&str, &str), &[[u32; 3]]> = attribution
        .files
        .iter()
        .map(|f| ((f.sha.as_str(), f.path.as_str()), f.spans.as_slice()))
        .collect();

    let total: u32 = blocks.iter().map(|b| b.num_lines).sum();
    let mut lines: Vec<Agent> = vec![None; total as usize];
    for block in &blocks {
        let Some(spans) = spans.get(&(block.sha.as_str(), block.path.as_str())) else {
            continue;
        };
        for i in 0..block.num_lines {
            // 구간은 그 커밋 버전의 줄 번호다(화면의 AiOverlay.indexAt과 같이 찾는다).
            let orig = block.orig_line + i;
            let Some([_, _, who]) = spans.iter().find(|[s, e, _]| *s <= orig && orig <= *e) else {
                continue;
            };
            if let Author::Agent {
                tool,
                id,
                model,
                human,
                ..
            } = &attribution.authors[*who as usize]
            {
                lines[(block.final_line - 1 + i) as usize] =
                    Some((tool.clone(), id.clone(), model.clone(), human.clone()));
            }
        }
    }
    lines
}

#[test]
fn ai_lines_match_git_ai_blame() {
    let mut names: Vec<String> = std::fs::read_dir(dir())
        .expect("fixtures/git-ai")
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            (path.extension()? == "json")
                .then(|| path.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    assert!(names.len() >= 10, "시나리오가 모자란다: {names:?}");
    let mut ai_lines = 0;
    for name in &names {
        let text = std::fs::read_to_string(dir().join(format!("{name}.json"))).unwrap();
        let expected: Expected = serde_json::from_str(&text).unwrap();
        let repo = unbundle(name, &expected.head);
        for (path, file) in &expected.files {
            for rev in [Some(expected.head.as_str()), None] {
                let ours = ours(&repo, path, rev);
                let theirs = theirs(file, ours.len());
                assert_eq!(
                    ours,
                    theirs,
                    "{name} {path} ({}): git-ai {} blame과 다르다",
                    rev.unwrap_or("작업트리"),
                    expected.git_ai_version
                );
                ai_lines += theirs.iter().flatten().count();
            }
        }
    }
    assert!(ai_lines > 0, "AI 줄이 하나도 없다");
}
