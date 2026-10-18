// parallax(plx) AI 출처 오버레이의 골든 데이터를 만든다. git-ai에 올리는 테스트가 아니다.
// PLX_GOLDEN_OUT=<디렉터리>를 주고 `cargo test --test integration plx_golden`으로 돌리면 시나리오마다
// `<이름>.bundle`(refs/notes/ai를 포함한 모든 참조)과 `<이름>.json`(HEAD와 파일마다 `git-ai blame --json`)을 쓴다.
use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;
use std::fs;
use std::path::PathBuf;

fn out_dir() -> Option<PathBuf> {
    std::env::var_os("PLX_GOLDEN_OUT").map(PathBuf::from)
}

/// `git-ai blame --json`에서 대조에 쓰는 부분만 남긴다(줄 범위 → 키, 키 → 에이전트·시킨 사람).
fn blame_json(repo: &TestRepo, path: &str) -> serde_json::Value {
    let out = repo
        .git_ai(&["blame", "--json", path])
        .unwrap_or_else(|e| panic!("git-ai blame --json {path}: {e}"));
    let start = out.find('{').expect("JSON 출력");
    let mut stream = serde_json::Deserializer::from_str(&out[start..]).into_iter::<serde_json::Value>();
    let value = stream.next().expect("JSON 값").expect("JSON 해석");
    let mut prompts = serde_json::Map::new();
    if let Some(map) = value["prompts"].as_object() {
        for (key, record) in map {
            prompts.insert(
                key.clone(),
                serde_json::json!({
                    "agent_id": record["agent_id"],
                    "human_author": record["human_author"],
                }),
            );
        }
    }
    serde_json::json!({ "lines": value["lines"], "prompts": prompts })
}

fn save(repo: &TestRepo, name: &str, paths: &[&str]) {
    let Some(dir) = out_dir() else { return };
    fs::create_dir_all(&dir).unwrap();
    let head = repo.git(&["rev-parse", "HEAD"]).unwrap().trim().to_string();
    let mut files = serde_json::Map::new();
    for path in paths {
        files.insert(path.to_string(), blame_json(repo, path));
    }
    let expected = serde_json::json!({
        "git_ai_version": env!("CARGO_PKG_VERSION"),
        "head": head,
        "files": files,
    });
    fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_string_pretty(&expected).unwrap() + "\n",
    )
    .unwrap();
    let bundle = dir.join(format!("{name}.bundle"));
    let _ = fs::remove_file(&bundle);
    repo.git_og(&["bundle", "create", bundle.to_str().unwrap(), "--all"])
        .unwrap();
}

/// 사람이 쓴 파일에 AI가 줄을 더하고, 사람이 그중 한 줄을 고친다(커밋 셋).
#[test]
fn plx_golden_basic() {
    let repo = TestRepo::new();
    let mut file = repo.filename("src/lib.rs");
    file.set_contents(crate::lines![
        "fn a() {}".human(),
        "fn b() {}".human(),
        "fn c() {}".human(),
    ]);
    repo.stage_all_and_commit("human start").unwrap();
    file.insert_at(
        1,
        crate::lines![
            "// written by the agent".ai(),
            "fn ai_one() {}".ai(),
            "fn ai_two() {}".ai(),
        ],
    );
    repo.stage_all_and_commit("agent adds functions").unwrap();
    file.replace_at(2, "fn ai_one() { /* human fix */ }".human());
    repo.stage_all_and_commit("human edits one agent line").unwrap();
    save(&repo, "basic", &["src/lib.rs"]);
}

/// 한 커밋 안에서 AI와 사람이 번갈아 고친다(AI 줄을 사람이 고치고 지운다).
#[test]
fn plx_golden_mixed_one_commit() {
    let repo = TestRepo::new();
    let mut file = repo.filename("notes.txt");
    file.set_contents(crate::lines!["one".human(), "two".human(), "three".human()]);
    repo.stage_all_and_commit("start").unwrap();
    file.insert_at(1, crate::lines!["ai a".ai(), "ai b".ai(), "ai c".ai(), "ai d".ai()]);
    file.replace_at(2, "ai b, fixed by a human".human());
    file.insert_at(7, crate::lines!["ai tail 1".ai(), "ai tail 2".ai()]);
    file.delete_at(4);
    repo.stage_all_and_commit("mixed session").unwrap();
    save(&repo, "mixed_one_commit", &["notes.txt"]);
}

/// AI가 쓴 파일의 이름을 바꾼 뒤 사람이 줄을 더한다(blame의 원래 경로가 옛 이름이다).
#[test]
fn plx_golden_rename() {
    let repo = TestRepo::new();
    let mut file = repo.filename("doc/old.md");
    file.set_contents(crate::lines![
        "# Title".human(),
        "agent paragraph 1".ai(),
        "agent paragraph 2".ai(),
        "human note".human(),
        "agent paragraph 3".ai(),
    ]);
    repo.stage_all_and_commit("agent writes doc").unwrap();
    repo.git(&["mv", "doc/old.md", "doc/new.md"]).unwrap();
    repo.stage_all_and_commit("rename doc").unwrap();
    let mut renamed = repo.filename("doc/new.md");
    renamed.insert_at(5, crate::lines!["human appendix".human()]);
    repo.stage_all_and_commit("human appends").unwrap();
    save(&repo, "rename", &["doc/new.md"]);
}

/// AI가 쓴 줄을 다음 커밋에서 AI가 다시 고친다.
#[test]
fn plx_golden_ai_over_ai() {
    let repo = TestRepo::new();
    let mut file = repo.filename("app.py");
    file.set_contents(crate::lines![
        "import os".ai(),
        "def run():".ai(),
        "    return 1".ai(),
        "".ai(),
        "run()".ai(),
    ]);
    repo.stage_all_and_commit("agent first pass").unwrap();
    file.replace_at(2, "    return os.getpid()".ai());
    file.insert_at(3, crate::lines!["    # second pass".ai()]);
    repo.stage_all_and_commit("agent second pass").unwrap();
    save(&repo, "ai_over_ai", &["app.py"]);
}

/// 사람이 위에 줄을 더하고 AI 줄 하나를 지워 AI 줄이 밀린다(원래 줄 번호가 지금과 다르다).
#[test]
fn plx_golden_shifted() {
    let repo = TestRepo::new();
    let mut file = repo.filename("list.txt");
    file.set_contents(crate::lines![
        "ai 1".ai(),
        "ai 2".ai(),
        "ai 3".ai(),
        "ai 4".ai(),
        "ai 5".ai(),
        "ai 6".ai(),
    ]);
    repo.stage_all_and_commit("agent list").unwrap();
    file.insert_at(0, crate::lines!["h top 1".human(), "h top 2".human(), "h top 3".human()]);
    file.delete_at(5);
    repo.stage_all_and_commit("human header, drop one").unwrap();
    save(&repo, "shifted", &["list.txt"]);
}

/// 곁가지의 AI 커밋을 본줄기로 cherry-pick한다(git-ai가 새 커밋에 노트를 옮긴다).
#[test]
fn plx_golden_cherry_pick() {
    let repo = TestRepo::new();
    let mut file = repo.filename("file.txt");
    file.set_contents(crate::lines!["base line".human()]);
    repo.stage_all_and_commit("base").unwrap();
    let main_branch = repo.current_branch();
    repo.git(&["checkout", "-b", "feature"]).unwrap();
    file.insert_at(1, crate::lines!["agent feature 1".ai(), "agent feature 2".ai()]);
    repo.stage_all_and_commit("agent feature").unwrap();
    let feature = repo.git(&["rev-parse", "HEAD"]).unwrap().trim().to_string();
    repo.git(&["checkout", &main_branch]).unwrap();
    repo.git(&["cherry-pick", &feature]).unwrap();
    save(&repo, "cherry_pick", &["file.txt"]);
}

/// AI 줄을 더한 뒤 커밋을 고친다(git commit --amend, git-ai가 고친 커밋의 노트를 새로 쓴다).
#[test]
fn plx_golden_amend() {
    let repo = TestRepo::new();
    let mut file = repo.filename("main.go");
    file.set_contents(crate::lines!["package main".human(), "func main() {}".human()]);
    repo.stage_all_and_commit("start").unwrap();
    file.insert_at(1, crate::lines!["// agent comment".ai(), "func helper() {}".ai()]);
    repo.stage_all_and_commit("agent helper").unwrap();
    file.insert_at(4, crate::lines!["func later() {}".ai()]);
    repo.git(&["add", "-A"]).unwrap();
    repo.git(&["commit", "--amend", "-m", "agent helper (amended)"])
        .unwrap();
    save(&repo, "amend", &["main.go"]);
}

/// 곁가지의 AI 줄을 merge 커밋으로 합친다(줄은 곁가지 커밋에서 왔다).
#[test]
fn plx_golden_merge() {
    let repo = TestRepo::new();
    let mut file = repo.filename("f.txt");
    file.set_contents(crate::lines!["first".human(), "second".human()]);
    let mut other = repo.filename("other.txt");
    other.set_contents(crate::lines!["other".human()]);
    repo.stage_all_and_commit("base").unwrap();
    let main_branch = repo.current_branch();
    repo.git(&["checkout", "-b", "feature"]).unwrap();
    file.insert_at(1, crate::lines!["agent between".ai()]);
    repo.stage_all_and_commit("agent on feature").unwrap();
    repo.git(&["checkout", &main_branch]).unwrap();
    let mut other_main = repo.filename("other.txt");
    other_main.insert_at(1, crate::lines!["main side".human()]);
    repo.stage_all_and_commit("human on main").unwrap();
    repo.git(&["merge", "--no-ff", "-m", "merge feature", "feature"])
        .unwrap();
    save(&repo, "merge", &["f.txt", "other.txt"]);
}

/// 한글·공백 경로와 한글 내용(노트의 경로는 따옴표로 쓰일 수 있다).
#[test]
fn plx_golden_korean_path() {
    let repo = TestRepo::new();
    let mut file = repo.filename("자료/설명 문서.md");
    file.set_contents(crate::lines![
        "# 설명".human(),
        "에이전트가 쓴 첫 문단".ai(),
        "사람이 쓴 줄".human(),
        "에이전트가 쓴 둘째 문단".ai(),
    ]);
    repo.stage_all_and_commit("한글 문서").unwrap();
    save(&repo, "korean_path", &["자료/설명 문서.md"]);
}

/// 한 커밋에서 두 파일에 AI 줄이 있다.
#[test]
fn plx_golden_two_files() {
    let repo = TestRepo::new();
    let mut a = repo.filename("a.txt");
    a.set_contents(crate::lines!["a human".human(), "a agent".ai()]);
    let mut b = repo.filename("sub/b.txt");
    b.set_contents(crate::lines!["b agent 1".ai(), "b human".human(), "b agent 2".ai()]);
    repo.stage_all_and_commit("two files").unwrap();
    save(&repo, "two_files", &["a.txt", "sub/b.txt"]);
}
