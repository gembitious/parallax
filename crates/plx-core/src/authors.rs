//! 작성자 보기(M18). 저장소(또는 한 경로)의 커밋을 작성자마다 세고 처음·마지막 커밋 시각을 붙인다. 많이 쓴 사람부터.
//!
//! `git log --no-merges --format=%aN%x00%aE%x00%at -z`를 한 번 띄워 센다(작성자는 `.mailmap`을 따른다: `%aN`·`%aE`).
//! 사람을 나누는 기준은 `git shortlog -sne --no-merges`와 같은 (이름, 메일)이다(골든 테스트). gix로 커밋을 직접 걸으면
//! 프로세스가 없지만 mailmap(`mailmap.file`·`mailmap.blob`)과 경로의 이력 단순화(Bloom 필터)를 git과 같게 다시 짜야 한다.
//! merge는 세지 않는다: 그래프 검색의 `author:`(`--no-merges`)가 찾는 커밋 수와 같게.
//!
//! 결과는 시작점(HEAD 또는 모든 참조의 끝)이 같으면 바뀌지 않으므로 디스크 캐시에 넣는다(얕은 클론은 넣지 않는다).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::cache::{CacheKey, DiskCache};
use crate::git::{self, Streaming};
use crate::repo::{Error, Repo, Result};

/// 어느 커밋을 세는가
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// HEAD에서 닿는 커밋
    Head,
    /// 모든 브랜치·원격 브랜치·태그에서 닿는 커밋(`--branches --remotes --tags`)
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    /// mailmap을 따른 이름(`%aN`)
    pub name: String,
    /// mailmap을 따른 메일(`%aE`)
    pub email: String,
    /// merge가 아닌 커밋 수
    pub commits: u32,
    /// 가장 이른 작성 시각(초)
    pub first: i64,
    /// 가장 늦은 작성 시각(초)
    pub last: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Authors {
    /// 작업트리 경로
    pub repo: String,
    /// 이 경로를 바꾼 커밋만 셌다(저장소 기준 경로). 없으면 저장소 전체
    pub path: Option<String>,
    pub scope: Scope,
    /// 센 커밋 수(모든 작성자의 합)
    pub commits: u32,
    /// 커밋 수가 많은 사람부터, 같으면 최근에 쓴 사람부터
    pub authors: Vec<Author>,
    /// 디스크 캐시에서 읽었다
    #[serde(default)]
    pub cached: bool,
}

/// `git log -z --format=%aN%x00%aE%x00%at` 출력을 받는 대로 센다. 커밋 하나는 NUL로 끝나는 필드 셋이다.
/// 시각이 숫자가 아니면 0으로 센다(필드 수만 맞으면 git 출력으로 받아들인다).
#[derive(Debug, Default)]
pub struct Tally {
    pending: Vec<u8>,
    fields: Vec<Vec<u8>>,
    by: HashMap<(String, String), usize>,
    authors: Vec<Author>,
    commits: u32,
}

impl Tally {
    pub fn new() -> Tally {
        Tally::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        while let Some(i) = rest.iter().position(|&b| b == 0) {
            let mut field = std::mem::take(&mut self.pending);
            field.extend_from_slice(&rest[..i]);
            rest = &rest[i + 1..];
            self.fields.push(field);
            if self.fields.len() == 3 {
                let time = std::mem::take(&mut self.fields[2]);
                let email = std::mem::take(&mut self.fields[1]);
                let name = std::mem::take(&mut self.fields[0]);
                self.fields.clear();
                self.add(name, email, &time);
            }
        }
        self.pending.extend_from_slice(rest);
    }

    fn add(&mut self, name: Vec<u8>, email: Vec<u8>, time: &[u8]) {
        // `-z`라 다음 커밋의 첫 필드 앞에는 아무것도 붙지 않지만, 옛 git이 줄바꿈을 끼워도 같은 사람으로 센다.
        let name = String::from_utf8_lossy(name.trim_ascii_start()).into_owned();
        let email = String::from_utf8_lossy(&email).into_owned();
        let time = std::str::from_utf8(time)
            .ok()
            .and_then(|t| t.trim().parse::<i64>().ok())
            .unwrap_or(0);
        self.commits = self.commits.saturating_add(1);
        let key = (name, email);
        match self.by.get(&key) {
            Some(&i) => {
                let author = &mut self.authors[i];
                author.commits = author.commits.saturating_add(1);
                author.first = author.first.min(time);
                author.last = author.last.max(time);
            }
            None => {
                self.by.insert(key.clone(), self.authors.len());
                self.authors.push(Author {
                    name: key.0,
                    email: key.1,
                    commits: 1,
                    first: time,
                    last: time,
                });
            }
        }
    }

    /// 지금까지 센 커밋 수
    pub fn commits(&self) -> u32 {
        self.commits
    }

    /// 다 받았다. 커밋 하나가 덜 왔으면(출력이 잘렸다) 오류.
    pub fn finish(self) -> std::result::Result<(u32, Vec<Author>), String> {
        if !self.pending.is_empty() || !self.fields.is_empty() {
            return Err("git log 출력이 커밋 가운데에서 끝났다".to_owned());
        }
        let mut authors = self.authors;
        sort(&mut authors);
        Ok((self.commits, authors))
    }
}

/// 커밋 수가 많은 사람부터, 같으면 최근에 쓴 사람, 이름, 메일 순
pub fn sort(authors: &mut [Author]) {
    authors.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then(b.last.cmp(&a.last))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.email.cmp(&b.email))
    });
}

/// 작성자를 센다. `path`는 저장소 기준 경로(파일·디렉터리, 비거나 `.`이면 저장소 전체).
pub fn list(
    repo: &Repo,
    scope: Scope,
    path: Option<&str>,
    cache: Option<&DiskCache>,
) -> Result<Authors> {
    let path = path
        .map(|p| p.trim().trim_start_matches("./").trim_end_matches('/'))
        .filter(|p| !p.is_empty() && *p != ".")
        .map(str::to_owned);
    if path.as_deref().is_some_and(|p| p.contains('\0')) {
        return Err(Error("경로에 NUL이 있다".to_owned()));
    }
    let workdir = repo.workdir().to_path_buf();
    let empty = |repo: &Repo| Authors {
        repo: repo.workdir().display().to_string(),
        path: path.clone(),
        scope,
        commits: 0,
        authors: Vec::new(),
        cached: false,
    };
    let Some(tips) = tips(repo, scope) else {
        // 빈 저장소(HEAD가 커밋을 가리키지 않는다)
        return Ok(empty(repo));
    };
    let key = (cache.is_some() && repo.cacheable()).then(|| {
        CacheKey::authors(
            &workdir,
            &tips,
            path.as_deref().unwrap_or(""),
            &repo.history_settings(),
        )
    });
    if let (Some(cache), Some(key)) = (cache, &key)
        && let Some(mut found) = cache.get::<Authors>(key)
    {
        found.cached = true;
        found.repo = workdir.display().to_string();
        return Ok(found);
    }

    let mut command = git::command(&workdir);
    command.args([
        "--literal-pathspecs",
        "log",
        "--no-merges",
        "--no-color",
        "-z",
        "--format=%aN%x00%aE%x00%at",
    ]);
    match scope {
        Scope::Head => command.arg("HEAD"),
        Scope::All => command.args(["--branches", "--remotes", "--tags"]),
    };
    command.arg("--");
    if let Some(path) = &path {
        command.arg(path);
    }
    let mut git = Streaming::spawn(command, "git log").map_err(|e| Error(e.to_string()))?;
    let mut tally = Tally::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = git.read(&mut buf).map_err(|e| Error(e.to_string()))?;
        if n == 0 {
            break;
        }
        tally.feed(&buf[..n]);
    }
    git.finish().map_err(|e| Error(e.to_string()))?;
    let (commits, authors) = tally.finish().map_err(Error)?;
    let result = Authors {
        commits,
        authors,
        ..empty(repo)
    };
    // 세는 동안 참조가 움직였으면(그 사이의 커밋, fetch) 이 결과가 어느 시작점의 것인지 모른다: 넣지 않는다.
    if let (Some(cache), Some(key)) = (cache, &key)
        && self::tips(repo, scope).as_deref() == Some(tips.as_str())
    {
        let _ = cache.put(key, &result);
    }
    Ok(result)
}

/// 시작점의 지문: HEAD 커밋, 또는 브랜치·원격 브랜치·태그마다 이름과 가리키는 객체. 시작점이 없으면 `None`.
fn tips(repo: &Repo, scope: Scope) -> Option<String> {
    let local = repo.local();
    match scope {
        Scope::Head => local.head_id().ok().map(|id| id.detach().to_string()),
        Scope::All => {
            let platform = local.references().ok()?;
            let mut out = Vec::new();
            for reference in platform.all().ok()?.flatten() {
                let name = reference.name().as_bstr();
                if !(name.starts_with(b"refs/heads/")
                    || name.starts_with(b"refs/remotes/")
                    || name.starts_with(b"refs/tags/"))
                {
                    continue;
                }
                if let gix::refs::TargetRef::Object(id) = reference.target() {
                    out.push(format!("{name} {id}"));
                }
            }
            if out.is_empty() {
                return None;
            }
            out.sort();
            // 태그가 수만 개인 저장소도 열쇠가 길어지지 않게 해시로 줄인다(참조 수를 함께 둬 겹칠 일을 더 줄인다).
            let joined = out.join("\n");
            Some(format!(
                "all {} {:016x}",
                out.len(),
                crate::cache::fnv1a64(joined.as_bytes())
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, email: &str, t: i64) -> Vec<u8> {
        format!("{name}\0{email}\0{t}\0").into_bytes()
    }

    #[test]
    fn counts_by_name_and_mail() {
        let mut out = Vec::new();
        out.extend(record("가나", "a@x", 30));
        out.extend(record("B", "b@x", 20));
        out.extend(record("가나", "a@x", 10));
        out.extend(record("가나", "other@x", 40));
        // 한 바이트씩 나눠 받아도 같다(필드·글자 가운데에서 끊긴다)
        let mut tally = Tally::new();
        for b in &out {
            tally.feed(std::slice::from_ref(b));
        }
        assert_eq!(tally.commits(), 4);
        let (commits, authors) = tally.finish().unwrap();
        assert_eq!(commits, 4);
        assert_eq!(
            authors
                .iter()
                .map(|a| (
                    a.name.as_str(),
                    a.email.as_str(),
                    a.commits,
                    a.first,
                    a.last
                ))
                .collect::<Vec<_>>(),
            vec![
                ("가나", "a@x", 2, 10, 30),
                ("가나", "other@x", 1, 40, 40),
                ("B", "b@x", 1, 20, 20),
            ]
        );
    }

    #[test]
    fn cut_output_is_an_error() {
        let mut tally = Tally::new();
        tally.feed(b"A\0a@x\0");
        assert!(tally.finish().is_err());
        let mut tally = Tally::new();
        tally.feed(b"A\0a@x\0not a time\0");
        let (_, authors) = tally.finish().unwrap();
        assert_eq!(authors[0].last, 0);
        assert_eq!(Tally::new().finish().unwrap(), (0, Vec::new()));
    }
}
