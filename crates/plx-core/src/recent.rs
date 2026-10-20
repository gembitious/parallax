//! 최근에 연 저장소(첫 화면·명령 팔레트). 앱 데이터 디렉터리의 `recent.json` 하나에 최근 순으로 둔다.
//!
//! 파일은 사람이나 다른 버전의 앱이 고칠 수 있는 외부 입력이다. 읽을 수 있는 항목만 쓰고(깨진 항목·상대 경로·
//! 같은 경로는 건너뛴다) 어떤 바이트에도 패닉하지 않는다(`tests/fuzz.rs`). 쓰기는 임시 파일에 쓴 뒤 이름을 바꾼다.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use serde::Serialize;

/// 기억하는 저장소 수
pub const LIMIT: usize = 20;
/// 데이터 디렉터리 안의 파일 이름
pub const FILE_NAME: &str = "recent.json";
/// 파일 형식 버전. 읽을 때는 보지 않는다(새 버전이 더한 필드는 무시하고 `repos`의 `path`만 읽는다).
const VERSION: u32 = 1;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// `recent.json`을 읽는다. 최근 순, 같은 경로는 처음 것만, 많아야 [`LIMIT`]개다. 형식이 다르거나 깨졌으면
/// 읽을 수 있는 항목만 돌려준다.
pub fn parse(bytes: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let Some(repos) = value.get("repos").and_then(|repos| repos.as_array()) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = Vec::new();
    for repo in repos {
        let Some(path) = repo.get("path").and_then(|path| path.as_str()) else {
            continue;
        };
        if !Path::new(path).is_absolute() || paths.iter().any(|p| p == path) {
            continue;
        }
        paths.push(path.to_owned());
        if paths.len() == LIMIT {
            break;
        }
    }
    paths
}

/// [`parse`]가 읽는 형식으로 쓴다.
pub fn serialize(paths: &[String]) -> Vec<u8> {
    #[derive(Serialize)]
    struct File<'a> {
        version: u32,
        repos: Vec<Repo<'a>>,
    }
    #[derive(Serialize)]
    struct Repo<'a> {
        path: &'a str,
    }
    let file = File {
        version: VERSION,
        repos: paths.iter().map(|path| Repo { path }).collect(),
    };
    let mut bytes = serde_json::to_vec_pretty(&file).expect("문자열만 든 값");
    bytes.push(b'\n');
    bytes
}

/// 최근 저장소 목록. 앱이 하나를 들고 요청마다 고친다. 경로는 저장소 작업트리(맨 바깥 폴더)의 절대 경로다.
pub struct Recent {
    /// 없으면 기억만 하고 파일에 쓰지 않는다(데이터 디렉터리를 모를 때)
    file: Option<PathBuf>,
    paths: Mutex<Vec<String>>,
}

impl Recent {
    /// `dir`의 `recent.json`을 읽는다. 파일이 없거나 읽지 못하면 빈 목록으로 시작한다.
    pub fn load(dir: Option<&Path>) -> Recent {
        let file = dir.map(|dir| dir.join(FILE_NAME));
        let paths = file
            .as_deref()
            .and_then(|file| fs::read(file).ok())
            .map(|bytes| parse(&bytes))
            .unwrap_or_default();
        Recent {
            file,
            paths: Mutex::new(paths),
        }
    }

    pub fn list(&self) -> Vec<String> {
        self.lock().clone()
    }

    /// 맨 앞에 둔다(이미 있으면 옮긴다). 목록이 바뀌었는지 돌려준다: 이미 맨 앞이면 그대로 두고 쓰지 않는다
    /// (같은 저장소의 요청이 이어진다).
    pub fn add(&self, path: &str) -> io::Result<bool> {
        let mut paths = self.lock();
        if paths.first().is_some_and(|first| first == path) {
            return Ok(false);
        }
        paths.retain(|p| p != path);
        paths.insert(0, path.to_owned());
        paths.truncate(LIMIT);
        self.save(&paths).map(|()| true)
    }

    /// 목록에서 뺀다. 목록이 바뀌었는지 돌려준다.
    pub fn remove(&self, path: &str) -> io::Result<bool> {
        let mut paths = self.lock();
        let before = paths.len();
        paths.retain(|p| p != path);
        if paths.len() == before {
            return Ok(false);
        }
        self.save(&paths).map(|()| true)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<String>> {
        // 고치는 동안 패닉해도 목록은 늘 온전한 값이다(바꾼 뒤에 쓴다).
        self.paths.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 잠금을 쥔 채 쓴다: 쓰기가 겹쳐 오래된 목록이 나중에 남지 않는다.
    fn save(&self, paths: &[String]) -> io::Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = file.with_file_name(format!(
            "{FILE_NAME}.{}.{}.tmp",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        if let Err(e) = fs::write(&tmp, serialize(paths)).and_then(|()| fs::rename(&tmp, file)) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(name: &str) -> String {
        if cfg!(windows) {
            format!(r"C:\code\{name}")
        } else {
            format!("/code/{name}")
        }
    }

    #[test]
    fn round_trip() {
        let paths = vec![abs("a"), abs("한글 저장소"), abs("b")];
        assert_eq!(parse(&serialize(&paths)), paths);
    }

    #[test]
    fn keeps_what_it_can_read() {
        let json = format!(
            r#"{{"version":9,"extra":1,"repos":[{{"path":{a}}},{{"path":"relative"}},{{"nope":1}},7,
            {{"path":{b},"pinned":true}},{{"path":{a}}},{{"path":""}},{{"path":null}}]}}"#,
            a = serde_json::to_string(&abs("a")).unwrap(),
            b = serde_json::to_string(&abs("b")).unwrap(),
        );
        assert_eq!(parse(json.as_bytes()), vec![abs("a"), abs("b")]);
        for broken in ["", "{", "[]", "null", r#"{"repos":{}}"#, "\u{feff}{}"] {
            assert!(parse(broken.as_bytes()).is_empty(), "{broken:?}");
        }
    }

    #[test]
    fn limit() {
        let many: Vec<String> = (0..LIMIT + 5).map(|i| abs(&i.to_string())).collect();
        assert_eq!(parse(&serialize(&many)), many[..LIMIT]);
    }

    #[test]
    fn add_moves_to_front_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("데이터");
        let recent = Recent::load(Some(&data));
        assert!(recent.list().is_empty());
        assert!(recent.add(&abs("a")).unwrap());
        assert!(recent.add(&abs("b")).unwrap());
        assert!(recent.add(&abs("a")).unwrap());
        assert!(!recent.add(&abs("a")).unwrap());
        assert_eq!(recent.list(), vec![abs("a"), abs("b")]);
        assert_eq!(Recent::load(Some(&data)).list(), vec![abs("a"), abs("b")]);

        assert!(recent.remove(&abs("a")).unwrap());
        assert!(!recent.remove(&abs("없음")).unwrap());
        assert_eq!(Recent::load(Some(&data)).list(), vec![abs("b")]);
        for i in 0..LIMIT + 3 {
            recent.add(&abs(&format!("n{i}"))).unwrap();
        }
        assert_eq!(Recent::load(Some(&data)).list().len(), LIMIT);
        // 임시 파일은 남지 않는다.
        let names: Vec<_> = fs::read_dir(&data)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(FILE_NAME)]);
    }

    #[test]
    fn broken_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE_NAME), b"\xff\xfe garbage").unwrap();
        let recent = Recent::load(Some(dir.path()));
        assert!(recent.list().is_empty());
        recent.add(&abs("a")).unwrap();
        assert_eq!(Recent::load(Some(dir.path())).list(), vec![abs("a")]);
    }

    #[test]
    fn without_dir_only_remembers() {
        let recent = Recent::load(None);
        recent.add(&abs("a")).unwrap();
        assert_eq!(recent.list(), vec![abs("a")]);
    }
}
