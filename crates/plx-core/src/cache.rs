//! 이력 질의 결과(blame, 파일 히스토리, 커밋 표) 디스크 캐시.
//!
//! 커밋 고정 결과는 바뀌지 않으므로 (저장소, 커밋, 경로, 결과를 바꾸는 설정)으로 영구히 쓴다. 파일 히스토리는
//! 시작 커밋(요청할 때의 HEAD)으로 고정한다. 작업트리 blame은 (저장소, HEAD, 경로, 설정, 파일 내용의 해시)로 쓴다:
//! git에 그 내용을 그대로 주므로(`git blame --contents -`) 내용이 같으면 결과도 같다. 얕은 클론은 이력을 더 받으면
//! 결과가 바뀌므로 넣지 않는다(부르는 쪽이 열쇠를 만들지 않는다).
//!
//! 결과 하나가 파일 하나다. 이름은 열쇠의 해시이고 파일 안에 열쇠 전체를 적어 해시 충돌을 걸러 낸다.
//! 임시 파일에 쓰고 rename하므로 도중에 끊겨도 깨진 파일을 읽지 않는다. 크기가 한도를 넘으면 오래 안 쓴 것부터 지운다.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// 형식이 바뀌면 올린다. 열쇠에 들어가므로 예전 파일은 읽지 않고 오래된 순으로 지워진다.
const FORMAT: &str = "plx-cache 3";
/// 이만큼 쓸 때마다 한도를 확인한다.
const PRUNE_EVERY: u32 = 64;
/// 임시 파일 이름의 일련번호. 한 프로세스에서 같은 열쇠를 동시에 써도 임시 파일이 겹치지 않는다.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    text: String,
}

impl CacheKey {
    /// 커밋 고정 blame. `repo`는 작업트리 경로, `commit`은 sha 전체,
    /// `settings`는 결과를 바꾸는 설정([`crate::repo::Repo::blame_settings`]).
    pub fn commit(repo: &Path, commit: &str, path: &str, settings: &str) -> CacheKey {
        CacheKey {
            text: format!(
                "{FORMAT}\ncommit\n{}\n{commit}\n{path}\n{settings}",
                repo.display()
            ),
        }
    }

    /// `start` 커밋부터의 파일 히스토리. `start`는 sha 전체.
    pub fn history(repo: &Path, start: &str, path: &str) -> CacheKey {
        CacheKey {
            text: format!("{FORMAT}\nhistory\n{}\n{start}\n{path}", repo.display()),
        }
    }

    /// `start` 커밋의 `path` 파일 `first`..=`last`줄의 라인 히스토리.
    pub fn line_history(repo: &Path, start: &str, path: &str, first: u32, last: u32) -> CacheKey {
        CacheKey {
            text: format!(
                "{FORMAT}\nlines\n{}\n{start}\n{first},{last}\n{path}",
                repo.display()
            ),
        }
    }

    /// 작업트리 blame. `contents`는 git에 준 작업트리 내용, `clean`은 그 내용을 git에 넣을 때의 모양
    /// ([`crate::repo::Repo::to_git`]): 줄 끝 설정·속성이 바뀌어 git이 보는 내용이 달라져도 다른 열쇠가 된다.
    pub fn worktree(
        repo: &Path,
        head: &str,
        path: &str,
        settings: &str,
        contents: &[u8],
        clean: &[u8],
    ) -> CacheKey {
        CacheKey {
            text: format!(
                "{FORMAT}\nworktree\n{}\n{head}\n{path}\n{settings}\n{} {:016x} {:016x}",
                repo.display(),
                contents.len(),
                fnv1a64(contents),
                fnv1a64(clean)
            ),
        }
    }

    /// commit-graph가 없는 저장소의 커밋 표([`crate::commit_table`]). `git_dir`는 git 공통 디렉터리.
    /// 커밋은 sha로 찾으므로 같은 경로에 다른 저장소가 생겨도 틀린 값을 주지 않는다(없는 커밋은 찾지 않는다).
    pub fn commit_table(git_dir: &Path) -> CacheKey {
        CacheKey {
            text: format!("{FORMAT}\ncommits\n{}", git_dir.display()),
        }
    }

    fn file_name(&self) -> String {
        format!("{:016x}.json", fnv1a64(self.text.as_bytes()))
    }

    fn bytes_file_name(&self) -> String {
        format!("{:016x}.bin", fnv1a64(self.text.as_bytes()))
    }
}

#[derive(Serialize)]
struct EntryRef<'a, T> {
    key: &'a str,
    value: &'a T,
}

#[derive(Deserialize)]
struct Entry<T> {
    key: String,
    value: T,
}

pub struct DiskCache {
    dir: PathBuf,
    max_bytes: u64,
    writes: AtomicU32,
}

impl DiskCache {
    pub fn new(dir: PathBuf, max_bytes: u64) -> DiskCache {
        DiskCache {
            dir,
            max_bytes,
            writes: AtomicU32::new(0),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 캐시된 결과. 없거나 읽지 못하면 `None`(캐시는 없어도 동작해야 한다).
    pub fn get<T: DeserializeOwned>(&self, key: &CacheKey) -> Option<T> {
        let path = self.dir.join(key.file_name());
        let bytes = fs::read(&path).ok()?;
        let entry: Entry<T> = serde_json::from_slice(&bytes).ok()?;
        if entry.key != key.text {
            return None;
        }
        // 오래 안 쓴 것부터 지우도록 쓴 시각을 갱신한다.
        if let Ok(file) = fs::File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(entry.value)
    }

    /// 결과를 넣는다. 실패해도 앱은 계속 동작한다.
    pub fn put<T: Serialize>(&self, key: &CacheKey, value: &T) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let entry = EntryRef {
            key: &key.text,
            value,
        };
        let path = self.dir.join(key.file_name());
        let tmp = self.dir.join(tmp_name(&key.file_name()));
        fs::write(&tmp, serde_json::to_vec(&entry).map_err(io::Error::other)?)?;
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        if self.writes.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY == PRUNE_EVERY - 1 {
            self.prune()?;
        }
        Ok(())
    }

    /// 바이너리 값(커서 JSON으로 두지 않는 것). 파일은 열쇠 길이(u32 LE), 열쇠, 값이다.
    pub fn get_bytes(&self, key: &CacheKey) -> Option<Vec<u8>> {
        let path = self.dir.join(key.bytes_file_name());
        let bytes = fs::read(&path).ok()?;
        let len = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
        let stored = bytes.get(4..4 + len)?;
        if stored != key.text.as_bytes() {
            return None;
        }
        if let Ok(file) = fs::File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        let mut bytes = bytes;
        bytes.drain(..4 + len);
        Some(bytes)
    }

    pub fn put_bytes(&self, key: &CacheKey, value: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let name = key.bytes_file_name();
        let path = self.dir.join(&name);
        let tmp = self.dir.join(tmp_name(&name));
        let mut bytes = Vec::with_capacity(4 + key.text.len() + value.len());
        bytes.extend_from_slice(&(key.text.len() as u32).to_le_bytes());
        bytes.extend_from_slice(key.text.as_bytes());
        bytes.extend_from_slice(value);
        fs::write(&tmp, bytes)?;
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        if self.writes.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY == PRUNE_EVERY - 1 {
            self.prune()?;
        }
        Ok(())
    }

    /// 크기가 한도를 넘으면 오래 안 쓴 파일부터 한도의 80%까지 지운다. 남은 임시 파일도 지운다.
    pub fn prune(&self) -> io::Result<()> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let now = SystemTime::now();
        let mut files = Vec::new();
        let mut total = 0u64;
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let path = entry.path();
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if path.extension().is_some_and(|e| e == "tmp") {
                // 쓰다 끊긴 임시 파일. 지금 쓰는 중일 수 있으니 오래된 것만 지운다.
                if now
                    .duration_since(modified)
                    .is_ok_and(|age| age > Duration::from_secs(60))
                {
                    let _ = fs::remove_file(&path);
                }
                continue;
            }
            total += meta.len();
            files.push((modified, meta.len(), path));
        }
        if total <= self.max_bytes {
            return Ok(());
        }
        files.sort_by_key(|(modified, ..)| *modified);
        let target = self.max_bytes / 10 * 8;
        for (_, len, path) in files {
            if total <= target {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
        Ok(())
    }
}

/// 임시 파일 이름: 프로세스와 쓰기마다 다르다(같은 열쇠를 두 스레드가 써도 서로의 파일을 덮지 않는다).
fn tmp_name(name: &str) -> String {
    format!(
        "{name}.{}.{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// 파일 이름용 해시. 충돌은 파일 안의 열쇠로 걸러 내므로 빠르고 안정적이면 된다(std 해시는 버전마다 다를 수 있다).
pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::blame::{BlameBlock, BlameCommit, Chunk};

    fn chunk(summary: &str) -> Chunk {
        Chunk {
            commits: vec![BlameCommit {
                sha: "a".repeat(40),
                summary: summary.to_owned(),
                ..BlameCommit::default()
            }],
            blocks: vec![BlameBlock {
                sha: "a".repeat(40),
                orig_line: 1,
                final_line: 1,
                num_lines: 3,
                path: "한글/파일.rs".to_owned(),
                previous: None,
            }],
        }
    }

    #[test]
    fn put_get_and_key_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(dir.path().join("blame"), 1 << 20);
        let key = CacheKey::commit(Path::new("/repo"), &"1".repeat(40), "a.rs", "");
        assert_eq!(cache.get::<Chunk>(&key), None);
        cache.put(&key, &chunk("첫 커밋")).unwrap();
        assert_eq!(cache.get(&key), Some(chunk("첫 커밋")));

        // 다른 열쇠, 그리고 같은 파일 이름에 다른 열쇠가 들어 있는 경우(해시 충돌 흉내)
        let other = CacheKey::commit(Path::new("/repo"), &"2".repeat(40), "a.rs", "");
        assert_eq!(cache.get::<Chunk>(&other), None);
        let collided = dir.path().join("blame").join(other.file_name());
        fs::copy(dir.path().join("blame").join(key.file_name()), &collided).unwrap();
        assert_eq!(
            cache.get::<Chunk>(&other),
            None,
            "열쇠가 다르면 읽지 않는다"
        );

        // 깨진 파일
        fs::write(dir.path().join("blame").join(key.file_name()), b"{").unwrap();
        assert_eq!(cache.get::<Chunk>(&key), None);
    }

    #[test]
    fn keys_cover_settings_head_and_contents() {
        let repo = Path::new("/repo");
        let sha = "1".repeat(40);
        assert_ne!(
            CacheKey::commit(repo, &sha, "a.rs", "").text,
            CacheKey::commit(repo, &sha, "a.rs", "blame.ignoreRevsFile=.revs\n").text,
            "blame 설정이 바뀌면 다른 열쇠"
        );

        let key = |head: &str, settings: &str, contents: &[u8], clean: &[u8]| {
            CacheKey::worktree(repo, head, "a.txt", settings, contents, clean).text
        };
        let base = key("h", "", b"a\r\n", b"a\n");
        assert_eq!(base, key("h", "", b"a\r\n", b"a\n"));
        assert_ne!(base, key("h2", "", b"a\r\n", b"a\n"), "HEAD");
        assert_ne!(base, key("h", "x=1\n", b"a\r\n", b"a\n"), "설정");
        assert_ne!(base, key("h", "", b"b\r\n", b"b\n"), "내용");
        assert_ne!(
            base,
            key("h", "", b"a\r\n", b"a\r\n"),
            "줄 끝 변환이 바뀌어 git이 보는 내용이 다르다(core.autocrlf, 속성)"
        );
    }

    #[test]
    fn concurrent_writes_of_one_key_do_not_share_a_temp_file() {
        assert_ne!(tmp_name("a.json"), tmp_name("a.json"));
        let dir = tempfile::tempdir().unwrap();
        let cache = Arc::new(DiskCache::new(dir.path().to_path_buf(), 1 << 30));
        let key = CacheKey::commit(Path::new("/r"), &"1".repeat(40), "a.rs", "");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let (cache, key) = (cache.clone(), key.clone());
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        // Windows는 같은 파일로 동시에 rename하면 하나가 실패할 수 있다(캐시 쓰기는 실패해도 된다).
                        let _ = cache.put(&key, &chunk(&i.to_string().repeat(500)));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let value: Chunk = cache.get(&key).expect("깨지지 않은 값 하나가 남는다");
        assert!(value.commits[0].summary.len() == 500);
    }

    #[test]
    fn prune_removes_least_recently_used_first() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(dir.path().to_path_buf(), 0);
        let keys: Vec<CacheKey> = (0..5)
            .map(|i| CacheKey::commit(Path::new("/r"), &i.to_string(), "a.rs", ""))
            .collect();
        let size = {
            cache.put(&keys[0], &chunk("x")).unwrap();
            fs::metadata(dir.path().join(keys[0].file_name()))
                .unwrap()
                .len()
        };
        let base = SystemTime::now() - Duration::from_secs(1000);
        for (i, key) in keys.iter().enumerate() {
            cache.put(key, &chunk("x")).unwrap();
            let file = fs::File::options()
                .write(true)
                .open(dir.path().join(key.file_name()))
                .unwrap();
            file.set_modified(base + Duration::from_secs(i as u64 * 10))
                .unwrap();
        }
        // 한도를 파일 세 개 크기로 두면 80%인 두 개 남짓까지 오래된 것부터 지운다.
        let cache = DiskCache::new(dir.path().to_path_buf(), size * 3);
        cache.prune().unwrap();
        let left: Vec<bool> = keys
            .iter()
            .map(|k| dir.path().join(k.file_name()).exists())
            .collect();
        assert_eq!(left, [false, false, false, true, true]);
    }

    #[test]
    fn bytes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(dir.path().to_path_buf(), 1 << 20);
        let key = CacheKey::commit_table(Path::new("/repo/.git"));
        assert_eq!(cache.get_bytes(&key), None);
        cache.put_bytes(&key, b"\x00\x01binary").unwrap();
        assert_eq!(
            cache.get_bytes(&key).as_deref(),
            Some(&b"\x00\x01binary"[..])
        );
        let other = CacheKey::commit_table(Path::new("/other/.git"));
        fs::copy(
            dir.path().join(key.bytes_file_name()),
            dir.path().join(other.bytes_file_name()),
        )
        .unwrap();
        assert_eq!(cache.get_bytes(&other), None, "열쇠가 다르면 읽지 않는다");
        // JSON 항목과 같은 열쇠여도 섞이지 않는다.
        assert_eq!(cache.get::<Chunk>(&key), None);
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
