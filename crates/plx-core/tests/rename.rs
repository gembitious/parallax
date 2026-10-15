//! 파일 하나의 rename 원본 찾기(`Repo::rename_source`)와 유사도를 git(`git diff-tree -M`)과 대조한다.

mod common;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use common::{NULL_DEVICE, encoding, git, git_text, history};
use plx_core::repo::Repo;
use plx_core::similarity::{self, MAX_SCORE, Spans};

/// 결정적인 의사 난수 줄
fn words(seed: u64, n: usize, width: usize) -> Vec<String> {
    let mut state = seed;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as usize
    };
    const WORDS: [&str; 12] = [
        "alpha", "beta", "gamma", "delta", "fox", "dog", "quick", "lazy", "jumps", "over", "한글",
        "줄",
    ];
    (0..n)
        .map(|_| {
            let mut line = String::new();
            while line.len() < width {
                line.push_str(WORDS[next() % WORDS.len()]);
                line.push(' ');
            }
            line.push_str(&next().to_string());
            line
        })
        .collect()
}

fn join(lines: &[String], eol: &str) -> String {
    lines.iter().map(|l| format!("{l}{eol}")).collect()
}

/// `count`줄을 다른 줄로 바꾼다.
fn edit(lines: &[String], count: usize, seed: u64) -> Vec<String> {
    let replacement = words(seed, count, 30);
    let mut out = lines.to_vec();
    for (i, line) in replacement.into_iter().enumerate() {
        let at = i * lines.len() / count.max(1);
        out[at] = line;
    }
    out
}

fn run_git(repo: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "user.name=t", "-c", "user.email=t@e"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", "1700000000 +0000")
        .env("GIT_COMMITTER_DATE", "1700000000 +0000")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// 여러 종류의 rename을 담은 저장소: 그대로, 조금·절반·절반 넘게 고침, CRLF로 바꿈, 긴 줄, 바이너리,
/// 크기가 두 배 넘게 바뀜, 디렉터리 이동, 내용이 같은 원본 둘. 테스트끼리 한 번만 만든다.
fn generated() -> PathBuf {
    static REPO: OnceLock<PathBuf> = OnceLock::new();
    REPO.get_or_init(make_generated).clone()
}

fn make_generated() -> PathBuf {
    let repo = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rename-sources");
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-q", "-b", "main"]);
    run_git(&repo, &["config", "core.autocrlf", "false"]);
    let write = |path: &str, bytes: &[u8]| {
        let file = repo.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    };
    let base = |seed| words(seed, 40, 30);
    let binary = |seed: u64| -> Vec<u8> {
        let mut bytes = join(&words(seed, 60, 20), "\n").into_bytes();
        for i in (0..bytes.len()).step_by(97) {
            bytes[i] = 0;
        }
        bytes
    };
    write("exact.txt", join(&base(1), "\n").as_bytes());
    write("small-edit.txt", join(&base(2), "\n").as_bytes());
    write("half.txt", join(&base(3), "\n").as_bytes());
    write("below.txt", join(&base(4), "\n").as_bytes());
    write("crlf.txt", join(&base(5), "\n").as_bytes());
    write("long-lines.txt", join(&words(6, 30, 200), "\n").as_bytes());
    write("binary.bin", &binary(7));
    write("grow.txt", join(&words(8, 15, 30), "\n").as_bytes());
    write("dir/moved.txt", join(&base(9), "\n").as_bytes());
    write("x/dup.txt", join(&base(10), "\n").as_bytes());
    write("y/copy.txt", join(&base(10), "\n").as_bytes());
    write("한글/문서.md", join(&base(11), "\n").as_bytes());
    write("keep.txt", join(&base(12), "\n").as_bytes());
    run_git(&repo, &["add", "-A"]);
    run_git(&repo, &["commit", "-q", "-m", "base"]);

    for path in [
        "exact.txt",
        "small-edit.txt",
        "half.txt",
        "below.txt",
        "crlf.txt",
        "long-lines.txt",
        "binary.bin",
        "grow.txt",
        "dir/moved.txt",
        "x/dup.txt",
        "y/copy.txt",
        "한글/문서.md",
    ] {
        std::fs::remove_file(repo.join(path)).unwrap();
    }
    write("renamed/exact.txt", join(&base(1), "\n").as_bytes());
    write(
        "renamed/small.txt",
        join(&edit(&base(2), 2, 100), "\n").as_bytes(),
    );
    write(
        "renamed/half.txt",
        join(&edit(&base(3), 17, 101), "\n").as_bytes(),
    );
    write(
        "renamed/below.txt",
        join(&edit(&base(4), 23, 102), "\n").as_bytes(),
    );
    write("renamed/crlf.txt", join(&base(5), "\r\n").as_bytes());
    write(
        "renamed/long-lines.txt",
        join(&edit(&words(6, 30, 200), 1, 103), "\n").as_bytes(),
    );
    let mut changed = binary(7);
    for b in changed.iter_mut().skip(5).step_by(300) {
        *b = b'#';
    }
    write("renamed/binary.bin", &changed);
    let mut grown = words(8, 15, 30);
    grown.extend(words(104, 20, 30));
    write("renamed/grow.txt", join(&grown, "\n").as_bytes());
    write("other/moved.txt", join(&base(9), "\n").as_bytes());
    write("z/dup.txt", join(&base(10), "\n").as_bytes());
    write(
        "문서/옮긴 문서.md",
        join(&edit(&base(11), 3, 105), "\n").as_bytes(),
    );
    write("new.txt", join(&base(13), "\n").as_bytes());
    // 지우지 않은 파일과 비슷한 새 파일: 복사(copy)는 따라가지 않는다.
    write("copied.txt", join(&base(12), "\n").as_bytes());
    run_git(&repo, &["add", "-A"]);
    run_git(&repo, &["commit", "-q", "-m", "renames"]);
    repo
}

/// git이 본 첫 부모와의 변경 중 새로 생긴 파일과 그 rename 원본(+ 점수 %)
fn git_additions(repo: &Path, commit: &str) -> Vec<(String, Option<(String, u64)>)> {
    let out = git(
        repo,
        &[
            "diff-tree",
            "-r",
            "-M",
            "-z",
            "--name-status",
            "--root",
            "--no-commit-id",
            &format!("{commit}^{{commit}}"),
        ],
    );
    let mut tokens = out.split(|&b| b == 0).filter(|t| !t.is_empty());
    let text = |t: &[u8]| String::from_utf8(t.to_vec()).unwrap();
    let mut additions = Vec::new();
    while let Some(status) = tokens.next() {
        let status = text(status);
        let first = text(tokens.next().unwrap());
        match status.as_bytes()[0] {
            b'R' | b'C' => {
                let second = text(tokens.next().unwrap());
                if let Some(score) = status.strip_prefix('R') {
                    additions.push((second, Some((first, score.parse().unwrap()))));
                }
            }
            b'A' => additions.push((first, None)),
            _ => {}
        }
    }
    additions
}

fn blob(repo: &Path, spec: &str) -> Vec<u8> {
    git(repo, &["cat-file", "blob", spec])
}

#[test]
fn similarity_scores_match_git() {
    let repo = generated();
    let additions = git_additions(&repo, "HEAD");
    let renames: Vec<_> = additions
        .iter()
        .filter_map(|(dest, source)| source.as_ref().map(|(src, score)| (dest, src, *score)))
        .collect();
    assert!(renames.len() >= 8, "{additions:?}");
    for (dest, src, percent) in renames {
        let src_data = blob(&repo, &format!("HEAD~1:{src}"));
        let dest_data = blob(&repo, &format!("HEAD:{dest}"));
        let spans = |data: &[u8]| Spans::new(data, !plx_core::text::is_binary(data));
        let score = similarity::score(&spans(&src_data), &spans(&dest_data));
        assert_eq!(score * 100 / MAX_SCORE, percent, "{src} → {dest}");
    }
    // 크기가 두 배 넘게 바뀌었거나 절반 넘게 고친 파일은 git도 rename으로 보지 않는다.
    for dest in [
        "renamed/below.txt",
        "renamed/grow.txt",
        "copied.txt",
        "new.txt",
    ] {
        assert!(
            additions.iter().any(|(d, s)| d == dest && s.is_none()),
            "{dest}: {additions:?}"
        );
    }
}

/// 커밋마다, 새로 생긴 파일마다 git이 찾은 rename 원본과 같다.
#[test]
fn rename_sources_match_git() {
    for repo in [generated(), history(), encoding()] {
        let opened = Repo::discover(&repo).unwrap();
        let commits = git_text(&repo, &["rev-list", "--all"]);
        let mut checked = 0;
        for commit in commits.lines() {
            let id = opened.resolve_commit(commit).unwrap();
            for (dest, source) in git_additions(&repo, commit) {
                let expected = source.map(|(src, _)| src);
                assert_eq!(
                    opened.rename_source(id, &dest).unwrap(),
                    expected,
                    "{} {commit} {dest}",
                    repo.display()
                );
                checked += 1;
            }
        }
        assert!(checked > 5, "{}: {checked}", repo.display());
    }
}

#[test]
fn not_an_addition_or_root_commit() {
    let repo = history();
    let opened = Repo::discover(&repo).unwrap();
    let head = opened.resolve_commit("HEAD").unwrap();
    assert_eq!(opened.rename_source(head, "src/lib.rs").unwrap(), None);
    assert_eq!(opened.rename_source(head, "no/such/file").unwrap(), None);
    let root = opened
        .resolve_commit(&git_text(&repo, &["rev-list", "--max-parents=0", "HEAD"]))
        .unwrap();
    assert_eq!(opened.rename_source(root, "src/lib.rs").unwrap(), None);
}
