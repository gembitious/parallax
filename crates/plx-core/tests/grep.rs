//! 저장소 검색(`plx_core::grep`)을 색 없는 `git grep` 결과와 대조한다: 맞은 줄(경로·줄 번호·내용)은
//! `git grep --null -n`과, 줄 안의 맞은 자리는 `git grep -o --column`(맞은 것마다 한 줄)과 같아야 한다.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{commit_all, git, scratch_repo};
use plx_core::grep::{GrepEvent, GrepFile, GrepQuery, GrepSessions};
use plx_core::repo::RepoCache;

/// (경로, 줄) → (내용, 맞은 글자들)
type Found = BTreeMap<(String, u32), (String, Vec<String>)>;

fn run(dir: &Path, query: &GrepQuery) -> (Vec<GrepFile>, GrepEvent) {
    let sessions = GrepSessions::new();
    let id = sessions.register();
    let mut files: Vec<GrepFile> = Vec::new();
    let mut end = None;
    sessions.run(id, &RepoCache::new(), dir, query, |event| {
        match event {
            GrepEvent::Files { files: more } => {
                for file in more {
                    match files.last_mut() {
                        Some(last) if last.path == file.path => last.lines.extend(file.lines),
                        _ => files.push(file),
                    }
                }
            }
            GrepEvent::Meta { .. } => {}
            other => end = Some(other),
        }
        true
    });
    (files, end.expect("끝 이벤트"))
}

fn ours(files: &[GrepFile]) -> Found {
    let mut found = Found::new();
    for file in files {
        for line in &file.lines {
            let utf16: Vec<u16> = line.text.encode_utf16().collect();
            let hits = line
                .ranges
                .iter()
                .map(|[s, e]| String::from_utf16(&utf16[*s as usize..*e as usize]).unwrap())
                .collect();
            let old = found.insert((file.path.clone(), line.line), (line.text.clone(), hits));
            assert!(
                old.is_none(),
                "같은 줄이 두 번: {}:{}",
                file.path,
                line.line
            );
        }
    }
    found
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// 색 없는 git grep. `flags`는 -F/-E/-i/-w 등, `tail`은 리비전과 `--`·경로
fn theirs(dir: &Path, flags: &[&str], pattern: &str, tail: &[&str]) -> Found {
    let mut base = vec!["grep", "--null", "-n", "-I"];
    base.extend_from_slice(flags);
    base.extend_from_slice(&["-e", pattern]);
    let strip = |path: &[u8]| -> String {
        let path = text(path);
        match tail.first().filter(|t| **t != "--") {
            Some(rev) => path
                .strip_prefix(&format!("{rev}:"))
                .unwrap_or(&path)
                .to_owned(),
            None => path,
        }
    };
    let mut found = Found::new();
    let mut args = base.clone();
    args.extend_from_slice(tail);
    for line in git(dir, &args)
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
    {
        let mut parts = line.splitn(3, |&b| b == 0);
        let path = strip(parts.next().unwrap());
        let number: u32 = text(parts.next().unwrap()).parse().unwrap();
        let content = parts.next().unwrap();
        let content = text(content.strip_suffix(b"\r").unwrap_or(content));
        found.insert((path, number), (content, Vec::new()));
    }
    let mut args = base;
    args.extend_from_slice(&["-o"]);
    args.extend_from_slice(tail);
    for line in git(dir, &args)
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
    {
        let mut parts = line.splitn(3, |&b| b == 0);
        let path = strip(parts.next().unwrap());
        let number: u32 = text(parts.next().unwrap()).parse().unwrap();
        let hit = text(parts.next().unwrap());
        found.get_mut(&(path, number)).expect("-o의 줄").1.push(hit);
    }
    found
}

/// (git grep 플래그, 패턴, 질의 바꾸기)
type Case = (&'static [&'static str], &'static str, fn(&mut GrepQuery));

fn query(pattern: &str) -> GrepQuery {
    GrepQuery {
        pattern: pattern.to_owned(),
        regex: false,
        case_sensitive: false,
        word: false,
        rev: None,
        paths: Vec::new(),
    }
}

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn sample(name: &str) -> std::path::PathBuf {
    let dir = scratch_repo(name);
    write(
        &dir,
        "a.rs",
        b"fn main() {\n    let value = value + Value;\n}\n",
    );
    write(
        &dir,
        "한글 경로/파일.txt",
        "값 = value\n한글도 찾는다: 값값\n".as_bytes(),
    );
    write(&dir, "crlf.txt", b"value one\r\nnone\r\n");
    write(&dir, "bin.dat", b"value\0binary\n");
    write(&dir, "deep/x/y.md", b"# Value\nvalues and valued\n");
    commit_all(&dir, "base");
    dir
}

#[test]
fn worktree_search_matches_git() {
    let dir = sample("grep-worktree");
    // 커밋하지 않은 고침도 찾고(작업트리), 추적하지 않는 파일은 찾지 않는다.
    write(
        &dir,
        "a.rs",
        b"fn main() {\n    let value = value + Value;\n    // new value\n}\n",
    );
    write(&dir, "untracked.txt", b"value\n");
    let cases: [Case; 5] = [
        (&["-F", "-i"], "value", |_| {}),
        (&["-F"], "value", |q| q.case_sensitive = true),
        (&["-F", "-i", "-w"], "value", |q| q.word = true),
        (&["-E", "-i"], "val(ue)?s?", |q| q.regex = true),
        (&["-F", "-i"], "값", |_| {}),
    ];
    for (flags, pattern, set) in cases {
        let mut q = query(pattern);
        set(&mut q);
        let (files, end) = run(&dir, &q);
        assert!(
            matches!(
                end,
                GrepEvent::Done {
                    truncated: false,
                    ..
                }
            ),
            "{end:?}"
        );
        let expected = theirs(&dir, flags, pattern, &["--"]);
        assert_eq!(ours(&files), expected, "{flags:?} {pattern}");
        assert!(
            files
                .iter()
                .all(|f| f.path != "untracked.txt" && f.path != "bin.dat")
        );
    }
}

#[test]
fn revision_and_paths_match_git() {
    let dir = sample("grep-rev");
    let base = common::git_text(&dir, &["rev-parse", "HEAD"]);
    write(&dir, "a.rs", b"nothing here\n");
    commit_all(&dir, "change");
    let mut q = query("value");
    q.rev = Some(base.clone());
    let (files, _) = run(&dir, &q);
    assert_eq!(
        ours(&files),
        theirs(&dir, &["-F", "-i"], "value", &[&base, "--"])
    );
    assert!(
        files.iter().any(|f| f.path == "a.rs"),
        "그 리비전의 내용을 찾는다"
    );

    q.paths = vec![
        "deep/".to_owned(),
        "*.txt".to_owned(),
        "!crlf.txt".to_owned(),
    ];
    let (files, _) = run(&dir, &q);
    assert_eq!(
        ours(&files),
        theirs(
            &dir,
            &["-F", "-i"],
            "value",
            &[&base, "--", "deep/", "*.txt", ":(exclude)crlf.txt"]
        )
    );
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["deep/x/y.md", "한글 경로/파일.txt"]);
}

#[test]
fn fixtures_search_matches_git() {
    for dir in [common::history(), common::encoding()] {
        let (files, _) = run(&dir, &query("a"));
        assert_eq!(
            ours(&files),
            theirs(&dir, &["-F", "-i"], "a", &["--"]),
            "{}",
            dir.display()
        );
    }
}

#[test]
fn no_match_and_bad_input_end_cleanly() {
    let dir = sample("grep-empty");
    let (files, end) = run(&dir, &query("zzz-nothing"));
    assert!(files.is_empty());
    assert!(matches!(end, GrepEvent::Done { matches: 0, .. }), "{end:?}");

    let mut q = query("(");
    q.regex = true;
    let (_, end) = run(&dir, &q);
    assert!(
        matches!(&end, GrepEvent::Error { message } if message.starts_with("git grep 실패")),
        "{end:?}"
    );

    let mut q = query("value");
    q.rev = Some("no-such-rev".to_owned());
    assert!(matches!(run(&dir, &q).1, GrepEvent::Error { .. }));
    assert!(matches!(run(&dir, &query("")).1, GrepEvent::Error { .. }));
}

#[test]
fn many_matches_stop_at_the_limit() {
    let dir = scratch_repo("grep-limit");
    let body = "hit\n".repeat(plx_core::grep::MAX_MATCHES / 4 + 10);
    for i in 0..5 {
        write(&dir, &format!("f{i}.txt"), body.as_bytes());
    }
    commit_all(&dir, "many");
    let (files, end) = run(&dir, &query("hit"));
    let count: usize = files.iter().map(|f| f.lines.len()).sum();
    assert_eq!(count, plx_core::grep::MAX_MATCHES);
    assert!(
        matches!(
            end,
            GrepEvent::Done {
                truncated: true,
                ..
            }
        ),
        "{end:?}"
    );
}
