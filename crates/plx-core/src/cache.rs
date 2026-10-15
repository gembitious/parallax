//! 이력 질의 결과(blame, 파일 히스토리) 디스크 캐시.
//!
//! 커밋 고정 결과는 바뀌지 않으므로 (저장소, 커밋, 경로)로 영구히 쓴다. 파일 히스토리는 시작 커밋
//! (요청할 때의 HEAD)으로 고정한다. 작업트리 blame은
//! (저장소, HEAD, 경로, 파일 크기, 수정 시각)으로 쓰되, 막 고친 파일은 넣지 않는다: 수정 시각의 단위
//! 안에서 다시 고치면 크기와 시각이 같아 낡은 결과를 돌려줄 수 있다(git의 racy-git과 같은 문제).
//!
//! 결과 하나가 파일 하나다. 이름은 열쇠의 해시이고 파일 안에 열쇠 전체를 적어 해시 충돌을 걸러 낸다.
//! 임시 파일에 쓰고 rename하므로 도중에 끊겨도 깨진 파일을 읽지 않는다. 크기가 한도를 넘으면 오래 안 쓴 것부터 지운다.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// 형식이 바뀌면 올린다. 열쇠에 들어가므로 예전 파일은 읽지 않고 오래된 순으로 지워진다.
const FORMAT: &str = "plx-cache 2";
/// 이보다 최근에 고친 작업트리 파일은 캐시에 넣지 않는다.
const RACY: Duration = Duration::from_secs(2);
/// 이만큼 쓸 때마다 한도를 확인한다.
const PRUNE_EVERY: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    text: String,
    /// 넣어도 되는가(작업트리 파일을 막 고쳤으면 아니다)
    storable: bool,
}

impl CacheKey {
    /// 커밋 고정 blame. `repo`는 작업트리 경로, `commit`은 sha 전체.
    pub fn commit(repo: &Path, commit: &str, path: &str) -> CacheKey {
        CacheKey {
            text: format!("{FORMAT}\ncommit\n{}\n{commit}\n{path}", repo.display()),
            storable: true,
        }
    }

    /// `start` 커밋부터의 파일 히스토리. `start`는 sha 전체.
    pub fn history(repo: &Path, start: &str, path: &str) -> CacheKey {
        CacheKey {
            text: format!("{FORMAT}\nhistory\n{}\n{start}\n{path}", repo.display()),
            storable: true,
        }
    }

    /// 작업트리 blame. `head`가 바뀌거나 파일의 크기·수정 시각이 바뀌면 다른 열쇠다.
    pub fn worktree(
        repo: &Path,
        head: &str,
        path: &str,
        meta: &fs::Metadata,
        now: SystemTime,
    ) -> CacheKey {
        let modified = meta.modified().ok();
        let nanos = modified
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let racy = modified.is_none_or(|t| now.duration_since(t).is_ok_and(|age| age < RACY))
            || modified.is_some_and(|t| t > now);
        CacheKey {
            text: format!(
                "{FORMAT}\nworktree\n{}\n{head}\n{path}\n{}\n{nanos}",
                repo.display(),
                meta.len()
            ),
            storable: !racy,
        }
    }

    pub fn storable(&self) -> bool {
        self.storable
    }

    fn file_name(&self) -> String {
        format!("{:016x}.json", fnv1a64(self.text.as_bytes()))
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
        if !key.storable {
            return Ok(());
        }
        fs::create_dir_all(&self.dir)?;
        let entry = EntryRef {
            key: &key.text,
            value,
        };
        let path = self.dir.join(key.file_name());
        let tmp = self
            .dir
            .join(format!("{}.{}.tmp", key.file_name(), std::process::id()));
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

/// 파일 이름용 해시. 충돌은 파일 안의 열쇠로 걸러 내므로 빠르고 안정적이면 된다(std 해시는 버전마다 다를 수 있다).
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
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
        let key = CacheKey::commit(Path::new("/repo"), &"1".repeat(40), "a.rs");
        assert_eq!(cache.get::<Chunk>(&key), None);
        cache.put(&key, &chunk("첫 커밋")).unwrap();
        assert_eq!(cache.get(&key), Some(chunk("첫 커밋")));

        // 다른 열쇠, 그리고 같은 파일 이름에 다른 열쇠가 들어 있는 경우(해시 충돌 흉내)
        let other = CacheKey::commit(Path::new("/repo"), &"2".repeat(40), "a.rs");
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
    fn worktree_keys() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "a\n").unwrap();
        let meta = fs::metadata(&file).unwrap();
        let modified = meta.modified().unwrap();
        let repo = Path::new("/repo");

        let fresh = CacheKey::worktree(
            repo,
            "h",
            "a.txt",
            &meta,
            modified + Duration::from_millis(500),
        );
        assert!(!fresh.storable(), "막 고친 파일은 넣지 않는다");
        let settled = CacheKey::worktree(
            repo,
            "h",
            "a.txt",
            &meta,
            modified + Duration::from_secs(10),
        );
        assert!(settled.storable());
        assert_eq!(fresh.text, settled.text, "넣을 수 있는지와 열쇠는 따로다");
        let future = CacheKey::worktree(
            repo,
            "h",
            "a.txt",
            &meta,
            modified - Duration::from_secs(10),
        );
        assert!(
            !future.storable(),
            "시계가 어긋나 미래 시각이면 넣지 않는다"
        );

        let other_head = CacheKey::worktree(
            repo,
            "h2",
            "a.txt",
            &meta,
            modified + Duration::from_secs(10),
        );
        assert_ne!(settled.text, other_head.text);

        let cache = DiskCache::new(dir.path().join("cache"), 1 << 20);
        cache.put(&fresh, &chunk("x")).unwrap();
        assert_eq!(cache.get::<Chunk>(&fresh), None);
        cache.put(&settled, &chunk("x")).unwrap();
        assert_eq!(
            cache.get(&fresh),
            Some(chunk("x")),
            "읽기는 열쇠만 같으면 된다"
        );
    }

    #[test]
    fn prune_removes_least_recently_used_first() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(dir.path().to_path_buf(), 0);
        let keys: Vec<CacheKey> = (0..5)
            .map(|i| CacheKey::commit(Path::new("/r"), &i.to_string(), "a.rs"))
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
    fn fnv_is_stable() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
