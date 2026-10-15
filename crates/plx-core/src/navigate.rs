//! blame 화면 사이의 이동: `,`(부모 리비전 blame), `<`/`>`(이전·다음 리비전).
//!
//! 리비전은 파일을 바꾼 커밋 단위로 움직인다. 커서 줄은 두 버전의 줄 diff([`LineMap`])로 옮기고,
//! 옮길 곳에서 생기거나 바뀐 줄이면 그 부분(hunk)이 시작하는 줄로 간다.
//!
//! 이력은 첫 부모만 따라간다(`--first-parent`). `git log --follow`만으로는 merge가 바꾼 버전(evil merge,
//! 충돌 해결)이 빠지고, 다른 브랜치의 커밋이 날짜 순으로 끼어들어 "이 버전의 바로 앞 버전"이 아니게 된다.

use serde::{Deserialize, Serialize};

use crate::git;
use crate::linemap::LineMap;
use crate::repo::{Error, Repo, Result};

/// 파일의 한 버전. `rev`가 없으면 작업트리 파일이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// 커밋 sha
    pub rev: Option<String>,
    /// 저장소 기준 경로(`/` 구분)
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    /// 더 오래된 리비전(`<`)
    Older,
    /// 더 최근 리비전(`>`). 가장 최근 다음은 작업트리다.
    Newer,
}

/// 버전의 내용. 그 버전에 파일이 없으면 `None`.
pub fn content(repo: &Repo, version: &Version) -> Result<Option<Vec<u8>>> {
    match &version.rev {
        Some(rev) => {
            let commit = repo.resolve_commit(rev)?;
            repo.blob_at(commit, &version.path)
        }
        None => match std::fs::read(repo.workdir().join(&version.path)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error(format!("파일을 읽지 못했다 ({}): {e}", version.path))),
        },
    }
}

fn required(repo: &Repo, version: &Version) -> Result<Vec<u8>> {
    content(repo, version)?.ok_or_else(|| {
        Error(match &version.rev {
            Some(rev) => format!("{}에 {} 파일이 없다", short(rev), version.path),
            None => format!("작업트리에 {} 파일이 없다", version.path),
        })
    })
}

/// `from`의 `line`번째 줄(1부터)이 `to`에서 몇 번째 줄인가.
pub fn map_line(repo: &Repo, from: &Version, to: &Version, line: u32) -> Result<u32> {
    if from == to {
        return Ok(line);
    }
    let from_bytes = required(repo, from)?;
    let to_bytes = required(repo, to)?;
    Ok(LineMap::new(&to_bytes, &from_bytes).to_before(line))
}

/// `current`의 이전·다음 리비전. 없으면 `None`.
///
/// - 이전: `current`와 내용이 다른, 파일을 바꾼 바로 앞 커밋의 버전. 작업트리가 HEAD와 같으면 HEAD 버전을 건너뛴다.
/// - 다음: `current` 이후 파일을 바꾼 첫 커밋의 버전. 그런 커밋이 없으면 작업트리(파일이 있을 때).
///   그 커밋에서 파일 이름이 바뀌었으면 새 이름을 따라간다.
pub fn neighbor(repo: &Repo, current: &Version, direction: Direction) -> Result<Option<Version>> {
    match direction {
        Direction::Older => older(repo, current),
        Direction::Newer => newer(repo, current),
    }
}

fn older(repo: &Repo, current: &Version) -> Result<Option<Version>> {
    let start = current.rev.as_deref().unwrap_or("HEAD");
    if current.rev.is_none() && repo.head()?.is_none() {
        return Ok(None); // 커밋이 없는 저장소
    }
    // 처음 것은 `start` 이하에서 파일을 마지막으로 바꾼 커밋이다. 고정 리비전의 내용은 그 커밋의 내용과 같다.
    let entries = log(
        repo,
        &[
            "-2",
            "--first-parent",
            "--follow",
            start,
            "--",
            &current.path,
        ],
        &current.path,
    )?;
    let mut entries = entries.into_iter();
    let Some(first) = entries.next() else {
        return Ok(None);
    };
    if current.rev.is_none() {
        // 작업트리가 마지막 커밋과 다르면 그 커밋이 바로 앞 버전이다.
        let working = content(repo, current)?;
        if working != content(repo, &first)? {
            return Ok(Some(first));
        }
    }
    Ok(entries.next())
}

fn newer(repo: &Repo, current: &Version) -> Result<Option<Version>> {
    let Some(rev) = &current.rev else {
        return Ok(None); // 작업트리보다 새 버전은 없다
    };
    let range = format!("{rev}..HEAD");
    let after = |first_parent: bool| {
        let mut args = vec!["--reverse", "--ancestry-path", "-M"];
        if first_parent {
            args.push("--first-parent");
        }
        args.extend([range.as_str(), "--", current.path.as_str()]);
        log(repo, &args, &current.path)
    };
    // 첫 부모 줄기에 없는 리비전(merge된 브랜치의 커밋)이면 첫 부모로는 이어지지 않으므로 전체 이력에서 찾는다.
    let mut entries = after(true)?;
    if entries.is_empty() {
        entries = after(false)?;
    }
    let Some(next) = entries.into_iter().next() else {
        // 이후로 파일을 바꾼 커밋이 없다: 다음은 작업트리
        let working = Version {
            rev: None,
            path: current.path.clone(),
        };
        return Ok(content(repo, &working)?.map(|_| working));
    };
    if content(repo, &next)?.is_some() {
        return Ok(Some(next));
    }
    // 그 커밋에서 파일이 없어졌다. 경로를 한정한 log는 rename을 지움으로 보이므로 커밋 전체에서 rename을 찾는다.
    let commit = next.rev.as_deref().unwrap_or_default();
    Ok(
        renamed_to(repo, commit, &current.path)?.map(|path| Version {
            rev: next.rev.clone(),
            path,
        }),
    )
}

/// `rev`의 `path` 파일이 작업트리에서는 어느 경로인가(그 뒤의 rename을 따라간다). 작업트리에 그 경로가 그대로
/// 있으면 그것이고, 지워졌으면 `None`. rename마다 git을 두 번 띄운다(경로가 없어진 커밋 찾기, 그 커밋의 rename).
pub fn worktree_path(repo: &Repo, rev: &str, path: &str) -> Result<Option<String>> {
    let exists = |path: &str| repo.workdir().join(path).is_file();
    let mut rev = rev.to_owned();
    let mut path = path.to_owned();
    // rename 고리가 끝없이 돌지 않게(A → B → A …) 한도를 둔다.
    for _ in 0..64 {
        if exists(&path) {
            return Ok(Some(path));
        }
        let range = format!("{rev}..HEAD");
        let gone = log(
            repo,
            &[
                "--reverse",
                "--ancestry-path",
                "--diff-filter=D",
                &range,
                "--",
                &path,
            ],
            &path,
        )?;
        let Some(commit) = gone.into_iter().next().and_then(|v| v.rev) else {
            return Ok(None); // HEAD까지 지운 적이 없는데 작업트리에 없다(커밋하지 않고 지웠다)
        };
        match renamed_to(repo, &commit, &path)? {
            Some(to) => {
                rev = commit;
                path = to;
            }
            None => return Ok(None), // 지웠다
        }
    }
    Ok(None)
}

/// `commit`이 `path`를 다른 이름으로 옮겼다면 새 이름.
fn renamed_to(repo: &Repo, commit: &str, path: &str) -> Result<Option<String>> {
    let parent = format!("{commit}^");
    let out = git::output(
        repo.workdir(),
        [
            "diff-tree",
            "-r",
            "-M",
            "-z",
            "--name-status",
            &parent,
            commit,
        ],
    )
    .map_err(|e| Error(e.to_string()))?;
    let mut tokens = out.split(|&b| b == 0).filter(|t| !t.is_empty());
    while let Some(status) = tokens.next() {
        let two_paths = matches!(status.first(), Some(b'R' | b'C'));
        let first = tokens.next().unwrap_or_default();
        let second = if two_paths { tokens.next() } else { None };
        if status.first() == Some(&b'R')
            && first == path.as_bytes()
            && let Some(to) = second
        {
            return Ok(Some(String::from_utf8_lossy(to).into_owned()));
        }
    }
    Ok(None)
}

/// `git log --name-status -z`로 파일을 바꾼 커밋과 그때의 경로. 경로를 알 수 없는 항목(merge 등)은 앞 경로를 쓴다.
fn log(repo: &Repo, args: &[&str], path: &str) -> Result<Vec<Version>> {
    let mut command = vec!["log", "-z", "--format=%x01%H", "--name-status"];
    command.extend_from_slice(args);
    let out = git::output(repo.workdir(), &command).map_err(|e| Error(e.to_string()))?;
    Ok(parse_log(&out, path))
}

fn parse_log(out: &[u8], path: &str) -> Vec<Version> {
    let mut versions: Vec<Version> = Vec::new();
    let mut named = false;
    let mut tokens = out.split(|&b| b == 0);
    while let Some(token) = tokens.next() {
        // 커밋 머리 다음 항목은 줄바꿈으로 시작한다.
        let token = token.strip_prefix(b"\n").unwrap_or(token);
        if token.is_empty() {
            continue;
        }
        if let Some(sha) = token.strip_prefix(b"\x01") {
            let path = versions.last().map_or(path, |v| v.path.as_str()).to_owned();
            versions.push(Version {
                rev: Some(String::from_utf8_lossy(sha).trim().to_owned()),
                path,
            });
            named = false;
            continue;
        }
        // 상태(M, A, D, T, R067, C100 …)와 경로 하나, rename·copy면 둘
        let two_paths = matches!(token.first(), Some(b'R' | b'C'));
        let first = tokens.next().unwrap_or_default();
        let second = if two_paths { tokens.next() } else { None };
        if let Some(version) = versions.last_mut()
            && !named
        {
            version.path = String::from_utf8_lossy(second.unwrap_or(first)).into_owned();
            named = true;
        }
    }
    versions
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(rev: &str, path: &str) -> Version {
        Version {
            rev: Some(rev.to_owned()),
            path: path.to_owned(),
        }
    }

    #[test]
    fn parses_follow_log_with_rename() {
        let out = b"\x01aaa\0\nR067\0src/old.rs\0src/new.rs\0\x01bbb\0\nM\0src/old.rs\0\x01ccc\0\nA\0src/old.rs\0";
        assert_eq!(
            parse_log(out, "src/new.rs"),
            [
                v("aaa", "src/new.rs"),
                v("bbb", "src/old.rs"),
                v("ccc", "src/old.rs")
            ]
        );
    }

    #[test]
    fn entries_without_status_keep_the_previous_path() {
        let out = "\x01aaa\0\nM\0한글/파일 이름.txt\0\x01mmm\0\x01bbb\0\nA\0한글/파일 이름.txt\0"
            .as_bytes();
        assert_eq!(
            parse_log(out, "x"),
            [
                v("aaa", "한글/파일 이름.txt"),
                v("mmm", "한글/파일 이름.txt"),
                v("bbb", "한글/파일 이름.txt")
            ]
        );
        assert_eq!(parse_log(b"", "x"), []);
    }
}
