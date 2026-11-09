//! 멈춰 있는 merge·rebase·cherry-pick·revert·am·bisect와 충돌 파일을 `git status`·`git rev-parse`·
//! `git diff --diff-filter=U`와 견준다(M15).

mod common;

use std::path::Path;
use std::process::Command;

use common::{NULL_DEVICE, commit_all, git, git_text, scratch_repo};
use plx_core::changes::{self, StatusKind};
use plx_core::progress::{self, Operation, Progress};
use plx_core::repo::Repo;

/// 실패해도 되는 git(충돌로 멈추는 merge 등). 사용자 설정과 언어의 영향을 받지 않는다.
fn git_any(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.quotepath=false", "-c", "core.editor=true"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_EDITOR", "true")
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .output()
        .expect("git 실행");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn status(dir: &Path) -> String {
    git_any(dir, &["status"])
}

fn rev(dir: &Path, name: &str) -> String {
    git_text(dir, &["rev-parse", name])
}

fn read(dir: &Path) -> Option<Progress> {
    progress::read(&Repo::discover(dir).unwrap()).unwrap()
}

/// git이 본 충돌 경로(경로 순)
fn git_conflicts(dir: &Path) -> Vec<String> {
    let mut paths: Vec<String> = git_text(dir, &["diff", "--name-only", "--diff-filter=U", "-z"])
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn assert_conflicts(dir: &Path) -> Vec<String> {
    let repo = Repo::discover(dir).unwrap();
    let ours = progress::conflicts(&repo).unwrap();
    assert_eq!(ours, git_conflicts(dir), "충돌 경로가 git과 같다");
    let status = changes::worktree_status(&repo).unwrap();
    for path in &ours {
        let entry = status
            .files
            .iter()
            .find(|e| &e.path == path)
            .expect("트리 상태에 충돌 파일");
        assert_eq!(entry.kind, StatusKind::Conflicted, "{path}");
    }
    ours
}

/// main과 곁가지(feat)가 같은 파일을 서로 다르게 고친 저장소. 곁가지에는 커밋이 셋이고 둘째가 main과 부딪친다.
fn diverged(name: &str) -> std::path::PathBuf {
    let dir = scratch_repo(name);
    for (file, text) in [
        ("a.txt", "a\n"),
        ("한글 파일.txt", "처음\n"),
        ("c.txt", "c\n"),
    ] {
        std::fs::write(dir.join(file), text).unwrap();
    }
    commit_all(&dir, "밑");
    git(&dir, &["checkout", "-q", "-b", "feat"]);
    std::fs::write(dir.join("a.txt"), "feat a\n").unwrap();
    commit_all(&dir, "곁 하나");
    std::fs::write(dir.join("한글 파일.txt"), "곁가지\n").unwrap();
    commit_all(&dir, "곁 둘");
    std::fs::write(dir.join("c.txt"), "feat c\n").unwrap();
    commit_all(&dir, "곁 셋");
    git(&dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("한글 파일.txt"), "본줄기\n").unwrap();
    commit_all(&dir, "본 하나");
    dir
}

#[test]
fn nothing_in_progress() {
    let dir = diverged("progress-none");
    assert_eq!(read(&dir), None);
    assert!(assert_conflicts(&dir).is_empty());
}

#[test]
fn merge_conflict() {
    let dir = diverged("progress-merge");
    git_any(&dir, &["merge", "feat"]);
    assert!(status(&dir).contains("You have unmerged paths."));
    let found = read(&dir).expect("merge 중");
    assert_eq!(found.operation, Operation::Merge);
    assert_eq!(found.applying.len(), 1);
    assert_eq!(found.applying[0].sha, rev(&dir, "MERGE_HEAD"));
    assert_eq!(found.applying[0].summary.as_deref(), Some("곁 셋"));
    assert_eq!(assert_conflicts(&dir), vec!["한글 파일.txt".to_owned()]);

    // 우리 쪽으로 되돌려 내용이 HEAD와 같아도 add하기 전까지는 충돌이다(git status의 both modified).
    git_any(&dir, &["checkout", "--ours", "한글 파일.txt"]);
    assert_eq!(assert_conflicts(&dir), vec!["한글 파일.txt".to_owned()]);

    git(&dir, &["add", "한글 파일.txt"]);
    assert!(assert_conflicts(&dir).is_empty());
    assert!(status(&dir).contains("All conflicts fixed but you are still merging."));
    assert_eq!(read(&dir).map(|p| p.operation), Some(Operation::Merge));

    git_any(&dir, &["merge", "--abort"]);
    assert_eq!(read(&dir), None);
}

/// `git status`의 "Last command(s) done (N command(s) done)"과 "(M remaining command(s))"
fn status_steps(text: &str) -> (u32, u32) {
    let count = |marker: &str| -> u32 {
        text.lines()
            .find_map(|line| {
                let at = line.find(marker)?;
                let before = &line[..at];
                let digits: String = before
                    .chars()
                    .rev()
                    .skip_while(|c| *c == ' ')
                    .take_while(char::is_ascii_digit)
                    .collect();
                digits.chars().rev().collect::<String>().parse().ok()
            })
            .unwrap_or(0)
    };
    let done = count("command done)").max(count("commands done)"));
    let remaining = count("remaining command)").max(count("remaining commands)"));
    (done, done + remaining)
}

/// "You are currently rebasing branch 'feat' on 'abc1234'."의 (브랜치, onto 앞자리)
fn status_rebasing(text: &str) -> (String, String) {
    let line = text
        .lines()
        .find(|line| line.starts_with("You are currently rebasing branch '"))
        .expect("rebase 중이라는 줄");
    let parts: Vec<&str> = line.split('\'').collect();
    (parts[1].to_owned(), parts[3].to_owned())
}

#[test]
fn rebase_conflict() {
    let dir = diverged("progress-rebase");
    git(&dir, &["checkout", "-q", "feat"]);
    git_any(&dir, &["rebase", "main"]);
    let text = status(&dir);
    let found = read(&dir).expect("rebase 중");
    assert_eq!(found.operation, Operation::Rebase);
    let (step, total) = status_steps(&text);
    assert_eq!(
        (found.step, found.total),
        (Some(step), Some(total)),
        "{text}"
    );
    assert_eq!((step, total), (2, 3));
    let (branch, onto) = status_rebasing(&text);
    assert_eq!(found.branch.as_deref(), Some(branch.as_str()));
    assert!(found.onto.as_deref().unwrap().starts_with(&onto));
    assert_eq!(found.onto.as_deref(), Some(rev(&dir, "main").as_str()));
    assert_eq!(found.applying.len(), 1);
    assert_eq!(found.applying[0].sha, rev(&dir, "REBASE_HEAD"));
    assert_eq!(found.applying[0].summary.as_deref(), Some("곁 둘"));
    assert_eq!(assert_conflicts(&dir), vec!["한글 파일.txt".to_owned()]);

    // 해결하고 이어 가면 끝난다.
    std::fs::write(dir.join("한글 파일.txt"), "합침\n").unwrap();
    git(&dir, &["add", "-A"]);
    git_any(&dir, &["rebase", "--continue"]);
    assert_eq!(read(&dir), None, "{}", status(&dir));
}

#[test]
fn rebase_apply_backend_and_detached() {
    let dir = diverged("progress-rebase-apply");
    git(&dir, &["checkout", "-q", "feat"]);
    git_any(&dir, &["rebase", "--apply", "main"]);
    let text = status(&dir);
    let found = read(&dir).expect("rebase 중");
    assert_eq!(found.operation, Operation::Rebase);
    let (branch, onto) = status_rebasing(&text);
    assert_eq!(found.branch.as_deref(), Some(branch.as_str()));
    assert!(found.onto.as_deref().unwrap().starts_with(&onto));
    assert_eq!(found.step, Some(2));
    assert_eq!(found.total, Some(3));
    assert_eq!(assert_conflicts(&dir), vec!["한글 파일.txt".to_owned()]);
    git_any(&dir, &["rebase", "--abort"]);

    // detached HEAD를 rebase하면 브랜치가 없다(git status: "You are currently rebasing.")
    git(&dir, &["checkout", "-q", "--detach", "feat"]);
    git_any(&dir, &["rebase", "main"]);
    assert!(status(&dir).contains("You are currently rebasing."));
    let found = read(&dir).expect("rebase 중");
    assert_eq!(found.branch, None);
}

#[test]
fn cherry_pick_and_revert() {
    let dir = diverged("progress-pick");
    let picked = rev(&dir, "feat~1");
    git_any(&dir, &["cherry-pick", &picked]);
    let text = status(&dir);
    assert!(text.contains(&format!(
        "You are currently cherry-picking commit {}",
        &picked[..7]
    )));
    let found = read(&dir).expect("cherry-pick 중");
    assert_eq!(found.operation, Operation::CherryPick);
    assert!(!found.sequence);
    assert_eq!(found.applying[0].sha, rev(&dir, "CHERRY_PICK_HEAD"));
    assert_eq!(found.applying[0].sha, picked);
    assert_eq!(assert_conflicts(&dir), vec!["한글 파일.txt".to_owned()]);
    git_any(&dir, &["cherry-pick", "--abort"]);
    assert_eq!(read(&dir), None);

    // 여러 커밋: 첫 커밋에서 멈춘다(sequencer/todo)
    git_any(&dir, &["cherry-pick", &picked, "feat"]);
    let found = read(&dir).expect("cherry-pick 중");
    assert!(found.sequence);
    assert!(status(&dir).contains("cherry-picking"));
    git_any(&dir, &["cherry-pick", "--abort"]);

    // revert: main의 커밋을 되돌리다 부딪친다(그 뒤에 같은 줄을 또 고쳤다)
    std::fs::write(dir.join("한글 파일.txt"), "또 고침\n").unwrap();
    commit_all(&dir, "본 둘");
    let reverted = rev(&dir, "HEAD~1");
    git_any(&dir, &["revert", "--no-edit", &reverted]);
    assert!(status(&dir).contains(&format!(
        "You are currently reverting commit {}",
        &reverted[..7]
    )));
    let found = read(&dir).expect("revert 중");
    assert_eq!(found.operation, Operation::Revert);
    assert_eq!(found.applying[0].sha, rev(&dir, "REVERT_HEAD"));
    assert_eq!(found.applying[0].sha, reverted);
    assert_eq!(assert_conflicts(&dir), vec!["한글 파일.txt".to_owned()]);
}

#[test]
fn bisect_and_am() {
    let dir = diverged("progress-bisect");
    git_any(&dir, &["bisect", "start", "feat", "main~1"]);
    let text = status(&dir);
    assert!(
        text.contains("You are currently bisecting, started from branch 'main'."),
        "{text}"
    );
    let found = read(&dir).expect("bisect 중");
    assert_eq!(found.operation, Operation::Bisect);
    assert_eq!(found.branch.as_deref(), Some("main"));
    git_any(&dir, &["bisect", "reset"]);
    assert_eq!(read(&dir), None);

    // am: 곁가지의 패치를 main에 붙이다 부딪친다
    let patches = Path::new(env!("CARGO_TARGET_TMPDIR")).join("progress-am-patches");
    let _ = std::fs::remove_dir_all(&patches);
    git(
        &dir,
        &[
            "format-patch",
            "-q",
            "-o",
            patches.to_str().unwrap(),
            "main~1..feat",
        ],
    );
    let mut files: Vec<_> = std::fs::read_dir(&patches)
        .unwrap()
        .map(|e| e.unwrap().path().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let mut args = vec!["am", "-3"];
    args.extend(files.iter().map(String::as_str));
    git_any(&dir, &args);
    assert!(status(&dir).contains("You are in the middle of an am session."));
    let found = read(&dir).expect("am 중");
    assert_eq!(found.operation, Operation::Am);
    assert_eq!((found.step, found.total), (Some(2), Some(3)));
}
