//! AI 출처: `refs/notes/ai`의 노트를 git과 같이 읽고(fanout 포함), (커밋, 경로)의 AI 작성 구간을 돌려준다.

mod common;

use std::path::Path;
use std::process::Command;

use common::{NULL_DEVICE, git, git_text};
use plx_core::ai_notes::{AiItem, AiNotes, Author, NotesTree};
use plx_core::repo::Repo;

fn git_as_user(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "user.name=plx", "-c", "user.email=plx@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn note(path: &str, entries: &[(&str, &str)], sessions: &str) -> String {
    let mut text = format!("{path}\n");
    for (key, ranges) in entries {
        text.push_str(&format!("  {key} {ranges}\n"));
    }
    text.push_str("---\n");
    text.push_str(&format!(
        "{{\"schema_version\":\"authorship/3.0.0\",\"base_commit_sha\":\"\",\"prompts\":{{}},\"sessions\":{{{sessions}}}}}"
    ));
    text
}

const CLAUDE: &str = r#""s_aaaaaaaaaaaaaa":{"agent_id":{"tool":"claude","id":"conv-1","model":"claude-opus"},"human_author":"dev@example.com"}"#;

fn setup(root: &Path) -> (String, String) {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    std::fs::create_dir(root.join("소스")).unwrap();
    std::fs::write(root.join("소스/파일.rs"), "a\nb\nc\nd\n").unwrap();
    git(root, &["add", "-A"]);
    git_as_user(root, &["commit", "--quiet", "-m", "처음"]);
    let first = git_text(root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("소스/파일.rs"), "a\nb\nc\nd\ne\n").unwrap();
    git_as_user(root, &["commit", "--quiet", "-am", "둘째"]);
    let second = git_text(root, &["rev-parse", "HEAD"]);
    (first, second)
}

#[test]
fn reads_notes_like_git() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("저장소");
    let (first, second) = setup(&root);
    let repo = Repo::discover(&root).unwrap();
    assert!(
        NotesTree::open(&repo.local()).unwrap().is_none(),
        "노트 없음"
    );

    let text = note(
        "소스/파일.rs",
        &[("s_aaaaaaaaaaaaaa::t_bbbbbbbbbbbbbb", "2-3")],
        CLAUDE,
    );
    let file = dir.path().join("note.txt");
    std::fs::write(&file, &text).unwrap();
    git_as_user(
        &root,
        &[
            "notes",
            "--ref=ai",
            "add",
            "-F",
            file.to_str().unwrap(),
            &first,
        ],
    );
    let tree = NotesTree::open(&repo.local()).unwrap().unwrap();
    assert_eq!(tree.len(), 1);
    let read = tree
        .read(&repo.local(), first.parse().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        read.trim_end(),
        git_text(&root, &["notes", "--ref=ai", "show", &first])
    );
    assert!(
        tree.read(&repo.local(), second.parse().unwrap())
            .unwrap()
            .is_none()
    );
}

/// 노트가 많으면 git은 `ab/cdef…`로 나눠 둔다. 나눈 트리를 손으로 만들어 git과 같이 읽는지 본다.
#[test]
fn reads_fanout_trees() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let (first, second) = setup(&root);
    let blob_for = |text: &str| {
        let file = dir.path().join("blob.txt");
        std::fs::write(&file, text).unwrap();
        git_text(&root, &["hash-object", "-w", file.to_str().unwrap()])
    };
    let a = blob_for(&note(
        "소스/파일.rs",
        &[("s_aaaaaaaaaaaaaa::t_cccccccccccccc", "1")],
        CLAUDE,
    ));
    let b = blob_for(&note(
        "소스/파일.rs",
        &[("s_aaaaaaaaaaaaaa::t_dddddddddddddd", "5")],
        CLAUDE,
    ));
    let mktree = |entries: &str| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("mktree")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(entries.as_bytes())?;
                child.wait_with_output()
            })
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    // first는 한 단계(2/38), second는 두 단계(2/2/36)로 나눈다.
    let one = mktree(&format!("100644 blob {a}\t{}\n", &first[2..]));
    let two_inner = mktree(&format!("100644 blob {b}\t{}\n", &second[4..]));
    let two = mktree(&format!("040000 tree {two_inner}\t{}\n", &second[2..4]));
    let mut root_entries = vec![
        format!("040000 tree {one}\t{}", &first[..2]),
        format!("040000 tree {two}\t{}", &second[..2]),
    ];
    if first[..2] == second[..2] {
        // 같은 접두어면 한 디렉터리에 둘 다
        let merged = mktree(&format!(
            "100644 blob {a}\t{}\n040000 tree {two_inner}\t{}\n",
            &first[2..],
            &second[2..4]
        ));
        root_entries = vec![format!("040000 tree {merged}\t{}", &first[..2])];
    }
    let tree = mktree(&(root_entries.join("\n") + "\n"));
    let commit = git_as_user(&root, &["commit-tree", &tree, "-m", "notes"]);
    git(&root, &["update-ref", "refs/notes/ai", &commit]);

    let repo = Repo::discover(&root).unwrap();
    let notes = NotesTree::open(&repo.local()).unwrap().unwrap();
    assert_eq!(notes.len(), 2);
    for sha in [&first, &second] {
        let ours = notes
            .read(&repo.local(), sha.parse().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            ours.trim_end(),
            git_text(&root, &["notes", "--ref=ai", "show", sha]),
            "{sha}"
        );
    }
}

#[test]
fn attribution_follows_the_notes_ref() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let (first, second) = setup(&root);
    let repo = Repo::discover(&root).unwrap();
    let ai = AiNotes::new();
    let items = [
        AiItem {
            sha: first.clone(),
            path: "소스/파일.rs".into(),
        },
        AiItem {
            sha: second.clone(),
            path: "소스/파일.rs".into(),
        },
    ];
    let none = ai.attribution(&repo, &items).unwrap();
    assert!(!none.notes);
    assert!(none.files.is_empty());

    let file = dir.path().join("note.txt");
    std::fs::write(
        &file,
        note(
            "소스/파일.rs",
            &[("s_aaaaaaaaaaaaaa::t_bbbbbbbbbbbbbb", "2-3")],
            CLAUDE,
        ),
    )
    .unwrap();
    git_as_user(
        &root,
        &[
            "notes",
            "--ref=ai",
            "add",
            "-F",
            file.to_str().unwrap(),
            &first,
        ],
    );
    let found = ai.attribution(&repo, &items).unwrap();
    assert!(found.notes);
    assert_eq!(found.parsed, 1);
    assert_eq!(found.files.len(), 1, "노트는 첫 커밋에만");
    assert_eq!(found.files[0].sha, first);
    assert_eq!(found.files[0].spans, [[2, 3, 0]]);
    match &found.authors[0] {
        Author::Agent {
            tool, model, human, ..
        } => {
            assert_eq!((tool.as_str(), model.as_str()), ("claude", "claude-opus"));
            assert_eq!(human.as_deref(), Some("dev@example.com"));
        }
        other => panic!("{other:?}"),
    }

    // 노트가 바뀌면(git-ai가 나중에 붙인다) 다시 읽는다.
    std::fs::write(
        &file,
        note(
            "소스/파일.rs",
            &[("s_aaaaaaaaaaaaaa::t_eeeeeeeeeeeeee", "5")],
            CLAUDE,
        ),
    )
    .unwrap();
    git_as_user(
        &root,
        &[
            "notes",
            "--ref=ai",
            "add",
            "-F",
            file.to_str().unwrap(),
            &second,
        ],
    );
    let both = ai.attribution(&repo, &items).unwrap();
    assert_eq!(both.files.len(), 2);
    assert_eq!(
        both.files[1].spans,
        [[5, 5, 0]],
        "같은 세션은 같은 작성자 번호"
    );
    assert_eq!(both.authors.len(), 1);
    assert_eq!(
        both.parsed, 1,
        "참조가 움직여도 그대로인 노트는 다시 읽지 않는다"
    );
    assert_eq!(ai.attribution(&repo, &items).unwrap().parsed, 0);

    // 이미 있는 노트를 고쳐 쓰면(blob이 바뀐다) 그 노트만 다시 읽는다.
    std::fs::write(
        &file,
        note(
            "소스/파일.rs",
            &[("s_aaaaaaaaaaaaaa::t_ffffffffffffff", "1")],
            CLAUDE,
        ),
    )
    .unwrap();
    git_as_user(
        &root,
        &[
            "notes",
            "--ref=ai",
            "add",
            "-f",
            "-F",
            file.to_str().unwrap(),
            &first,
        ],
    );
    let rewritten = ai.attribution(&repo, &items).unwrap();
    assert_eq!(rewritten.parsed, 1);
    assert_eq!(rewritten.files[0].spans, [[1, 1, 0]]);
    assert_eq!(rewritten.files[1].spans, [[5, 5, 0]]);
}

/// 옛 형식 프롬프트는 다른 노트에서도 찾는다(cherry-pick). 노트가 더해지면 새 노트만 더 모으고, 프롬프트를 적은
/// 노트가 고쳐지면 처음부터 다시 모은다.
#[test]
fn foreign_prompts_follow_note_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let (first, second) = setup(&root);
    let repo = Repo::discover(&root).unwrap();
    let ai = AiNotes::new();
    let file = dir.path().join("note.txt");
    let add = |text: &str, commit: &str| {
        std::fs::write(&file, text).unwrap();
        git_as_user(
            &root,
            &[
                "notes",
                "--ref=ai",
                "add",
                "-f",
                "-F",
                file.to_str().unwrap(),
                commit,
            ],
        );
    };
    const PROMPT: &str = r#""abcd1234abcd1234":{"agent_id":{"tool":"cursor","id":"old","model":"gpt-4"},"human_author":"dev@example.com","total_additions":2,"total_deletions":0,"accepted_lines":2,"overriden_lines":0}"#;
    let with_prompt = format!(
        "소스/파일.rs\n  abcd1234abcd1234 1-2\n---\n{{\"schema_version\":\"authorship/3.0.0\",\"base_commit_sha\":\"\",\"prompts\":{{{PROMPT}}}}}"
    );
    // 둘째 커밋의 노트는 프롬프트를 적지 않고 키만 쓴다(cherry-pick으로 옮겨진 줄).
    let key_only = note("소스/파일.rs", &[("abcd1234abcd1234", "4")], "");
    let items = [AiItem {
        sha: second.clone(),
        path: "소스/파일.rs".into(),
    }];

    add(&key_only, &second);
    let alone = ai.attribution(&repo, &items).unwrap();
    assert!(alone.files.is_empty(), "프롬프트를 아는 노트가 없다");

    add(&with_prompt, &first);
    let found = ai.attribution(&repo, &items).unwrap();
    assert_eq!(found.files.len(), 1);
    assert_eq!(found.files[0].spans, [[4, 4, 0]]);
    match &found.authors[0] {
        Author::Agent { tool, .. } => assert_eq!(tool, "cursor"),
        other => panic!("{other:?}"),
    }
    assert_eq!(found.parsed, 1, "새로 붙은 노트만 더 모은다");

    // 프롬프트를 적은 노트를 고치면 그 프롬프트는 사라진다.
    add(&note("소스/파일.rs", &[], ""), &first);
    let gone = ai.attribution(&repo, &items).unwrap();
    assert!(gone.files.is_empty(), "{gone:?}");
}
