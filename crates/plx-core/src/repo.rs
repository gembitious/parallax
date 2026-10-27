//! 저장소 읽기(gix). refs·커밋·트리·blob을 프로세스를 띄우지 않고 읽는다.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use gix::ObjectId;
use gix::bstr::{BStr, ByteSlice};
use serde::Serialize;

/// 저장소 작업의 오류. 화면에 그대로 보여 줄 문장이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn err(context: &str, e: impl fmt::Display) -> Error {
    Error(format!("{context}: {e}"))
}

/// 요청이 가리키는 파일.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePath {
    /// 터미널에서 준 경로(절대 경로). 실제 경로(심볼릭 링크·대소문자·`/private/var` 같은 별칭)로 맞춰
    /// 저장소 기준 경로를 찾는다.
    Os(PathBuf),
    /// 화면이 준 저장소(작업트리 경로)와 저장소 기준 경로(`/` 구분). git이 준 경로라 그대로 쓴다: 리비전의
    /// 경로를 오늘의 파일 시스템으로 풀면 그 사이 대소문자만 바꾼 rename이나 심볼릭 링크로 바뀐 파일이 다른 파일이 된다.
    InRepo { workdir: PathBuf, path: String },
}

impl FilePath {
    /// 화면 요청: `repo`가 있으면 `path`는 그 저장소 기준 경로, 없으면 절대 경로다.
    pub fn from_request(repo: Option<&str>, path: &str) -> FilePath {
        match repo {
            Some(repo) => FilePath::InRepo {
                workdir: PathBuf::from(repo),
                path: path.to_owned(),
            },
            None => FilePath::Os(PathBuf::from(path)),
        }
    }
}

impl From<&Path> for FilePath {
    fn from(path: &Path) -> Self {
        FilePath::Os(path.to_path_buf())
    }
}

impl From<PathBuf> for FilePath {
    fn from(path: PathBuf) -> Self {
        FilePath::Os(path)
    }
}

impl From<&PathBuf> for FilePath {
    fn from(path: &PathBuf) -> Self {
        FilePath::Os(path.clone())
    }
}

/// 화면이 준 저장소 기준 경로를 확인한다. 저장소 밖(`..`, 절대 경로)은 받지 않는다. `""`·`.`은 저장소 전체다.
fn checked_relative(path: &str) -> Result<String> {
    let invalid = || Error(format!("저장소 기준 경로가 아니다: {path}"));
    if path.is_empty() || path == "." {
        return Ok(path.to_owned());
    }
    if path.starts_with('/') || path.contains('\0') || Path::new(path).has_root() {
        return Err(invalid());
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(invalid());
        }
        // Windows의 드라이브(`C:`)나 UNC 앞머리
        if cfg!(windows) && (part.contains(':') || part.contains('\\')) {
            return Err(invalid());
        }
    }
    Ok(path.to_owned())
}

/// 작업트리가 있는 저장소. 여러 스레드에서 나눠 쓴다.
pub struct Repo {
    sync: gix::ThreadSafeRepository,
    /// 심볼릭 링크를 푼 작업트리 경로. 사용자 경로와 비교할 때 기준이 된다(Windows는 `\\?\` 형식).
    canonical: PathBuf,
    /// 화면과 git 작업 디렉터리에 쓰는 경로. Windows에서 `\\?\`를 뗄 수 있으면 뗀다
    /// (Git for Windows 같은 자식 프로세스가 그 형식의 작업 디렉터리를 다루지 못할 수 있다).
    workdir: PathBuf,
    /// 열 때 읽은 설정 파일(과 나중에 생길 수 있는 자리)의 크기·수정 시각. 바뀌면 [`RepoCache`]가 다시 연다.
    config: ConfigStamp,
}

impl fmt::Debug for Repo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Repo")
            .field("workdir", &self.workdir)
            .finish()
    }
}

impl Repo {
    /// `path`(파일이나 디렉터리, 지금은 없는 경로도 된다)를 담은 저장소를 찾는다.
    pub fn discover(path: &Path) -> Result<Repo> {
        Self::discover_with(path, Default::default())
    }

    /// [`discover`](Self::discover)와 같되 저장소 밖의 시스템·git 설치 설정은 읽지 않는다. `plx`가 상주 인스턴스에
    /// 넘기기 전에 저장소를 열어 볼 때 쓴다: Windows에서는 시스템 설정의 자리를 gix가 git을 띄워 찾아(`EXEPATH`가
    /// 없으면) 호출마다 30~50ms가 들었다(CI, PLX_TRACE). 그 확인(저장소인지, 리비전이 있는지)은 시스템 설정과
    /// 상관없다. 상주 앱은 그 값을 프로세스마다 한 번 치르므로 [`discover`](Self::discover)로 모두 읽는다.
    pub fn discover_for_check(path: &Path) -> Result<Repo> {
        let mut permissions = gix::open::Permissions::all();
        permissions.config.system = false;
        permissions.config.git_binary = false;
        permissions.attributes.system = false;
        permissions.attributes.git_binary = false;
        let defaults = gix::sec::trust::Mapping::<gix::open::Options>::default();
        Self::discover_with(
            path,
            gix::sec::trust::Mapping {
                full: defaults.full.permissions(permissions),
                reduced: defaults.reduced.permissions(permissions),
            },
        )
    }

    fn discover_with(
        path: &Path,
        trust: gix::sec::trust::Mapping<gix::open::Options>,
    ) -> Result<Repo> {
        let start = existing_ancestor(path)
            .ok_or_else(|| Error(format!("경로가 없다: {}", path.display())))?;
        let start = if start.is_dir() {
            start
        } else {
            start.parent().unwrap_or(start)
        };
        let repo: gix::Repository =
            gix::ThreadSafeRepository::discover_opts(start, Default::default(), trust)
                .map_err(|e| err(&format!("git 저장소가 아니다 ({})", start.display()), e))?
                .into();
        let workdir = repo
            .workdir()
            .ok_or_else(|| Error("작업트리가 없는(bare) 저장소는 지원하지 않는다".to_owned()))?;
        let canonical = std::fs::canonicalize(workdir).map_err(|e| err("작업트리 경로", e))?;
        let config = ConfigStamp::of(&repo);
        Ok(Repo {
            sync: repo.into_sync(),
            workdir: dunce::simplified(&canonical).to_path_buf(),
            canonical,
            config,
        })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    pub fn local(&self) -> gix::Repository {
        self.sync.to_thread_local()
    }

    /// 절대 경로를 저장소 기준 상대 경로(`/`로 구분)로 바꾼다.
    /// 심볼릭 링크·대소문자(Windows·macOS)·`/private/var` 같은 별칭은 실제 경로로 맞춘 뒤 비교한다.
    pub fn relative_path(&self, path: &Path) -> Result<String> {
        let canonical = canonicalize_lenient(path)?;
        let relative = canonical.strip_prefix(&self.canonical).map_err(|_| {
            Error(format!(
                "저장소({}) 밖의 경로다: {}",
                self.workdir.display(),
                path.display()
            ))
        })?;
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => {
                    parts.push(part.to_str().ok_or_else(|| {
                        Error(format!("경로가 UTF-8이 아니다: {}", path.display()))
                    })?)
                }
                other => return Err(Error(format!("예상하지 못한 경로 구성 요소: {other:?}"))),
            }
        }
        Ok(parts.join("/"))
    }

    /// 작업트리의 `rel`이 심볼릭 링크면 가리키는 파일의 저장소 기준 경로(저장소 밖이면 오류), 아니면 그대로.
    pub fn resolve_link(&self, rel: &str) -> Result<String> {
        let path = self.workdir.join(rel);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => self.relative_path(&path),
            _ => Ok(rel.to_owned()),
        }
    }

    /// 작업트리 파일의 내용(바이트 그대로). 없으면 `None`.
    pub fn worktree_bytes(&self, rel: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.workdir.join(rel)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error(format!("파일을 읽지 못했다 ({rel}): {e}"))),
        }
    }

    /// 작업트리 파일을 git에 넣을 때의 모양(clean)으로 읽는다. 없으면 `None`. blob과 비교하거나 줄을 옮길 때 쓴다.
    pub fn worktree_clean(&self, rel: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .worktree_bytes(rel)?
            .map(|bytes| match self.to_git(rel, &bytes) {
                Cow::Borrowed(_) => bytes,
                Cow::Owned(clean) => clean,
            }))
    }

    /// 작업트리 내용 `bytes`를 git에 넣을 때의 모양(clean)으로 바꾼다: 줄 끝(`core.autocrlf`, `text`·`eol` 속성)과
    /// `ident`. 작업트리가 CRLF(Git for Windows 기본값)여도 blob(LF)과 줄 단위로 맞는다. 외부 필터 드라이버
    /// (LFS 같은 `filter` 속성)는 띄우지 않는다. 바꿀 수 없으면(속성을 읽지 못하는 등) 그대로 둔다.
    pub fn to_git<'a>(&self, rel: &str, bytes: &'a [u8]) -> Cow<'a, [u8]> {
        // clean 변환은 CR을 지우거나 `$Id: …$`를 접을 뿐이다. 둘 다 없으면 바뀌지 않는다(인덱스·속성을 읽지 않는다).
        if !bytes.contains(&b'\r') && bytes.windows(3).all(|w| w != b"$Id") {
            return Cow::Borrowed(bytes);
        }
        match self.convert_to_git(rel, bytes) {
            Ok(Some(clean)) => Cow::Owned(clean),
            Ok(None) => Cow::Borrowed(bytes),
            Err(e) => {
                eprintln!("plx: 줄 끝 변환을 하지 못해 그대로 쓴다 ({rel}): {e}");
                Cow::Borrowed(bytes)
            }
        }
    }

    /// 바뀌었으면 `Some`.
    fn convert_to_git(&self, rel: &str, bytes: &[u8]) -> Result<Option<Vec<u8>>> {
        use gix::filter::plumbing::pipeline::CrlfRoundTripCheck;
        use gix::filter::plumbing::pipeline::convert::ToGitOutcome;
        use std::io::Read;

        let repo = self.local();
        let (pipeline, index) = repo
            .filter_pipeline(None)
            .map_err(|e| err("변환 설정", e))?;
        let (mut pipeline, mut attributes) = pipeline.into_parts();
        let options = pipeline.options_mut();
        options.drivers.clear();
        // 섞인 줄 끝(CRLF와 LF)도 실패하지 않고 바꾼다. git도 경고만 한다(core.safecrlf 기본값).
        options.crlf_roundtrip_check = CrlfRoundTripCheck::Skip;
        let path = Path::new(rel);
        let entry = attributes
            .at_path(path, None, &repo.objects)
            .map_err(|e| err("속성", e))?;
        let outcome = pipeline
            .convert_to_git(
                bytes,
                path,
                &mut |_, attrs| {
                    entry.matching_attributes(attrs);
                },
                // `text=auto`는 인덱스의 blob에 CR이 있으면 바꾸지 않는다(git의 has_crlf_in_index).
                &mut |buf| {
                    use gix::prelude::Find;
                    let Some(entry) = index.entry_by_path(rel.into()) else {
                        return Ok(None);
                    };
                    Ok(repo
                        .objects
                        .try_find(&entry.id, buf)
                        .ok()
                        .flatten()
                        .filter(|object| object.kind == gix::object::Kind::Blob)
                        .map(|_| ()))
                },
            )
            .map_err(|e| err("줄 끝 변환", e))?;
        match outcome {
            ToGitOutcome::Unchanged(_) => Ok(None),
            ToGitOutcome::Buffer(clean) => Ok(Some(clean.to_vec())),
            ToGitOutcome::Process(mut read) => {
                let mut clean = Vec::new();
                read.read_to_end(&mut clean)
                    .map_err(|e| err("줄 끝 변환", e))?;
                Ok(Some(clean))
            }
        }
    }

    /// 인덱스에 `rel`이 있는가(충돌 중인 단계도 친다). 인덱스가 없으면 `false`.
    pub fn in_index(&self, rel: &str) -> Result<bool> {
        let index = self
            .local()
            .index_or_empty()
            .map_err(|e| err("인덱스", e))?;
        Ok(index.entry_range(rel.into()).is_some())
    }

    /// 얕은 클론인가. 경계 너머의 이력을 모르므로 이력 결과를 캐시에 넣지 않는다(이력을 더 받으면 바뀐다).
    pub fn is_shallow(&self) -> bool {
        self.local().is_shallow().unwrap_or(true)
    }

    /// git이 객체와 다른 부모·내용을 보는 커밋이 있는가: `git replace`로 바꾼 객체(`core.useReplaceRefs=false`·
    /// `GIT_NO_REPLACE_OBJECTS`로 끄지 않았으면)나 `info/grafts`. git은 이때 commit-graph를 쓰지 않고
    /// (commit_graph_compatible), 이력 결과가 커밋 sha만으로 정해지지 않는다(바꾸는 참조를 더하거나 빼면 달라진다).
    pub fn has_rewrites(&self) -> bool {
        let repo = self.local();
        !read_grafts(repo.common_dir()).is_empty() || has_replace_refs(&repo)
    }

    /// 이력 질의 결과(blame, 히스토리)를 캐시에 넣어도 되는가: 얕은 클론과 [`Self::has_rewrites`]는 넣지 않는다.
    pub fn cacheable(&self) -> bool {
        !self.is_shallow() && !self.has_rewrites()
    }

    /// `git blame` 결과를 바꾸는 설정의 지문. 결과 캐시의 열쇠에 넣는다: 설정이나 설정이 가리키는 파일
    /// (`blame.ignoreRevsFile`, `.mailmap`)이 바뀌면 다른 열쇠가 된다.
    pub fn blame_settings(&self) -> String {
        self.settings(BLAME_SETTINGS, true)
    }

    /// 파일 히스토리(`git log`) 결과를 바꾸는 설정의 지문: 작성자 이름·메일(`%aN`·`%aE`)을 바꾸는 mailmap.
    pub fn history_settings(&self) -> String {
        self.settings(MAILMAP_SETTINGS, false)
    }

    /// 라인 히스토리(`git log -L`) 결과를 바꾸는 설정의 지문: mailmap과 줄을 따라가는 diff·rename 설정.
    pub fn line_history_settings(&self) -> String {
        self.settings(LINE_LOG_SETTINGS, false)
    }

    /// `keys` 설정 값과 mailmap(·`ignore_revs`면 `blame.ignoreRevsFile`) 파일 내용의 지문.
    fn settings(&self, keys: &[&str], ignore_revs: bool) -> String {
        use std::fmt::Write;

        let repo = self.local();
        let config = repo.config_snapshot();
        let config = config.plumbing();
        let mut out = String::new();
        for key in keys {
            for value in config.strings(*key).unwrap_or_default() {
                let _ = writeln!(out, "{key}={value}");
            }
        }
        // 설정이 가리키는 파일은 내용으로 본다. 빈 값은 앞의 목록을 비운다(git과 같다).
        let mut files = Vec::new();
        if ignore_revs {
            for value in config.strings("blame.ignoreRevsFile").unwrap_or_default() {
                if value.is_empty() {
                    files.clear();
                } else {
                    files.push(self.config_path(&value.to_str_lossy()));
                }
            }
        }
        files.push(self.workdir.join(".mailmap"));
        if let Some(value) = config.string("mailmap.file") {
            files.push(self.config_path(&value.to_str_lossy()));
        }
        for path in files {
            let hash = std::fs::read(&path).map_or_else(
                |_| "-".to_owned(),
                |bytes| format!("{:016x}", crate::cache::fnv1a64(&bytes)),
            );
            let _ = writeln!(out, "{}={hash}", path.display());
        }
        if let Some(spec) = config.string("mailmap.blob") {
            let id = repo
                .rev_parse_single(spec.as_bstr())
                .map_or_else(|_| "-".to_owned(), |id| id.to_string());
            let _ = writeln!(out, "mailmap.blob@{id}");
        }
        out
    }

    /// 설정 값의 경로: `~/`은 홈 디렉터리, 상대 경로는 작업트리 기준(git을 작업트리에서 띄운다).
    fn config_path(&self, value: &str) -> PathBuf {
        if let Some(rest) = value.strip_prefix("~/")
            && let Some(home) = gix::path::env::home_dir()
        {
            return home.join(rest);
        }
        self.workdir.join(value)
    }

    /// HEAD가 가리키는 커밋. 커밋이 하나도 없는 저장소면 `None`.
    pub fn head(&self) -> Result<Option<ObjectId>> {
        let repo = self.local();
        let mut head = repo.head().map_err(|e| err("HEAD", e))?;
        match head.try_peel_to_id() {
            Ok(Some(id)) => Ok(Some(id.detach())),
            Ok(None) => Ok(None),
            Err(e) => Err(err("HEAD", e)),
        }
    }

    /// HEAD가 가리키는 브랜치 이름(`refs/heads/` 뒤). detached면 `None`. 커밋이 없는 브랜치도 이름이 있다.
    pub fn head_branch(&self) -> Result<Option<String>> {
        let name = self.local().head_name().map_err(|e| err("HEAD", e))?;
        Ok(name.and_then(|name| {
            name.as_bstr()
                .strip_prefix(b"refs/heads/")
                .map(|b| b.to_str_lossy().into_owned())
        }))
    }

    /// 리비전 표현(`HEAD~2`, 태그, sha 일부 등)을 커밋으로 푼다.
    pub fn resolve_commit(&self, rev: &str) -> Result<ObjectId> {
        let repo = self.local();
        let id = repo.rev_parse_single(rev).map_err(|e| {
            // 커밋이 하나도 없으면(HEAD가 아직 없는 브랜치를 가리킨다) gix는 HEAD를 "rev-spec is malformed"라고 답한다.
            if (rev.starts_with("HEAD") || rev.starts_with('@'))
                && let Ok(head) = repo.head()
                && head.is_unborn()
            {
                let branch = head
                    .referent_name()
                    .map_or_else(|| "HEAD".to_owned(), |name| name.shorten().to_string());
                return Error(format!(
                    "커밋이 아직 없다({rev}): HEAD가 가리키는 {branch} 브랜치에 커밋이 하나도 없다"
                ));
            }
            err(&format!("리비전을 찾을 수 없다 ({rev})"), e)
        })?;
        let commit = id
            .object()
            .and_then(|object| object.peel_to_commit())
            .map_err(|e| err(&format!("커밋이 아니다 ({rev})"), e))?;
        Ok(commit.id)
    }

    /// `commit`의 `path` 파일 내용. 그 커밋에 파일이 없으면 `None`.
    pub fn blob_at(&self, commit: ObjectId, path: &str) -> Result<Option<Vec<u8>>> {
        let repo = self.local();
        let tree = repo
            .find_commit(commit)
            .and_then(|commit| commit.tree())
            .map_err(|e| err("커밋 트리", e))?;
        let Some(entry) = tree
            .lookup_entry_by_path(path)
            .map_err(|e| err("트리 조회", e))?
        else {
            return Ok(None);
        };
        if !entry.mode().is_blob() {
            return Err(Error(format!("파일이 아니다: {path}")));
        }
        let blob = entry.object().map_err(|e| err("blob 읽기", e))?;
        Ok(Some(blob.detach().data))
    }

    /// `commit` 시점의 폴더 `dir`(저장소 기준, `/` 구분, 루트는 빈 문자열) 바로 아래의 (폴더, 파일) 이름. 리비전
    /// 둘러보기(M7-4)의 파일 트리다. 서브모듈(커밋 항목)과 UTF-8이 아닌 이름은 뺀다(열 수 없다: 작업트리의 파일
    /// 트리와 같다). 그 커밋에 폴더가 없으면 `None`.
    pub fn dir_at(
        &self,
        commit: ObjectId,
        dir: &str,
    ) -> Result<Option<(Vec<String>, Vec<String>)>> {
        let repo = self.local();
        let mut tree = repo
            .find_commit(commit)
            .and_then(|commit| commit.tree())
            .map_err(|e| err("커밋 트리", e))?;
        if !dir.is_empty() {
            let Some(entry) = tree
                .lookup_entry_by_path(dir)
                .map_err(|e| err("트리 조회", e))?
            else {
                return Ok(None);
            };
            if !entry.mode().is_tree() {
                return Ok(None);
            }
            tree = entry.object().map_err(|e| err("트리 읽기", e))?.into_tree();
        }
        let decoded = tree.decode().map_err(|e| err("트리 읽기", e))?;
        let (mut dirs, mut files) = (Vec::new(), Vec::new());
        for entry in &decoded.entries {
            let Ok(name) = std::str::from_utf8(entry.filename) else {
                continue;
            };
            if entry.mode.is_tree() {
                dirs.push(name.to_owned());
            } else if !entry.mode.is_commit() {
                files.push(name.to_owned());
            }
        }
        Ok(Some((dirs, files)))
    }

    /// `commit`에 `path` 파일(blob·실행 파일·심볼릭 링크)이 있는가. 내용은 읽지 않는다.
    pub fn has_file(&self, commit: ObjectId, path: &str) -> Result<bool> {
        let repo = self.local();
        let tree = repo
            .find_commit(commit)
            .and_then(|commit| commit.tree())
            .map_err(|e| err("커밋 트리", e))?;
        let entry = tree
            .lookup_entry_by_path(path)
            .map_err(|e| err("트리 조회", e))?;
        Ok(entry.is_some_and(|entry| !entry.mode().is_tree() && !entry.mode().is_commit()))
    }

    /// 커밋 상세(hover 카드). 변경 파일 수는 첫 부모(없으면 빈 트리)와 비교한다. 카드는 빨리 떠야 하므로 유사도로
    /// 견줄 rename 짝은 [`HOVER_RENAME_LIMIT`]²까지만 본다(파일을 아주 많이 옮긴 커밋은 커밋 상세와 수가 다를 수 있다).
    pub fn commit_info(&self, id: ObjectId) -> Result<CommitInfo> {
        self.commit_changes(id, HOVER_RENAME_LIMIT)
            .map(|(info, _)| info)
    }

    /// 커밋 정보와 첫 부모(없으면 빈 트리)와 비교한 파일 변경. 디렉터리(트리) 항목은 뺀다.
    /// rename은 사용자 설정과 관계없이 git 기본값(유사도 50%)으로, git과 같은 단계로 찾는다([`crate::renames`]).
    /// `rename_limit`: 유사도로 견줄 짝 수의 한도(그 제곱, git의 `diff.renameLimit`).
    pub(crate) fn commit_changes(
        &self,
        id: ObjectId,
        rename_limit: usize,
    ) -> Result<(CommitInfo, Changes)> {
        let repo = self.local();
        let commit = repo.find_commit(id).map_err(|e| err("커밋 읽기", e))?;
        let mut info = commit_header(&commit)?;
        let tree = commit.tree().map_err(|e| err("커밋 트리", e))?;
        // 얕은 클론의 경계 커밋은 부모 객체가 없다. git처럼 루트 커밋으로 보고 빈 트리와 비교한다.
        let parent_tree = match commit.parent_ids().next() {
            Some(parent) => match repo.try_find_object(parent) {
                Ok(Some(object)) => Some(object.peel_to_tree().map_err(|e| err("부모 트리", e))?),
                Ok(None) => None,
                Err(e) => return Err(err("부모 커밋", e)),
            },
            None => None,
        };
        let changes = tree_changes(&repo, parent_tree.as_ref(), &tree, rename_limit)?;
        info.changed_files = changes.list.len();
        Ok((info, changes))
    }

    /// 커밋 정보만(바뀐 파일 수는 세지 않아 0이다).
    pub fn commit_header(&self, id: ObjectId) -> Result<CommitInfo> {
        let repo = self.local();
        let commit = repo.find_commit(id).map_err(|e| err("커밋 읽기", e))?;
        commit_header(&commit)
    }
}

/// 커밋 정보(`changed_files`는 0). 메시지·이름은 커밋의 encoding 헤더로 읽는다.
fn commit_header(commit: &gix::Commit<'_>) -> Result<CommitInfo> {
    let decoded = commit.decode().map_err(|e| err("커밋 해석", e))?;
    let encoding = decoded.encoding;
    let message = decode_text(decoded.message, encoding);
    let author = person(commit.author().map_err(|e| err("작성자", e))?, encoding);
    let committer = person(commit.committer().map_err(|e| err("커미터", e))?, encoding);
    Ok(CommitInfo {
        sha: commit.id.to_string(),
        parents: commit.parent_ids().map(|id| id.to_string()).collect(),
        summary: message.lines().next().unwrap_or("").trim().to_owned(),
        message: message.trim_end().to_owned(),
        author,
        committer,
        changed_files: 0,
    })
}

/// 두 트리(`old`가 없으면 빈 트리)의 파일 변경. 디렉터리(트리) 항목은 뺀다.
/// rename은 사용자 설정과 관계없이 git 기본값(유사도 50%)으로, git과 같은 단계로 찾는다([`crate::renames`]).
pub(crate) fn tree_changes(
    repo: &gix::Repository,
    old: Option<&gix::Tree<'_>>,
    new: &gix::Tree<'_>,
    rename_limit: usize,
) -> Result<Changes> {
    // rename은 아래에서 따로 찾는다(gix는 짝 한도가 git과 다르고 blob이 없으면 비교 전체가 실패한다).
    let options = gix::diff::Options::default().with_rewrites(None);
    // 트리 비교는 바뀐 파일을 담은 디렉터리(트리) 항목도 변경으로 돌려준다. 파일만 남긴다.
    let raw: Vec<TreeChange> = repo
        .diff_tree_to_tree(old, Some(new), options)
        .map_err(|e| err("트리 비교", e))?
        .into_iter()
        .filter(|change| !change.entry_mode().is_tree())
        .collect();
    Ok(pair_renames(repo, raw, rename_limit, &HashMap::new()))
}

/// hover 카드의 rename 짝 한도(그 제곱). 커밋 상세는 git과 같은 [`crate::renames::RENAME_LIMIT`]다.
const HOVER_RENAME_LIMIT: usize = 100;

/// 커밋 하나의 파일 변경(rename을 짝지은 뒤).
pub(crate) struct Changes {
    pub list: Vec<TreeChange>,
    /// blob이 없어 rename을 다 견주지 못했다(부분 클론).
    pub renames_incomplete: bool,
    /// 지운 파일 × 새 파일이 한도²를 넘어 유사도로 찾지 않았다(git도 건너뛴다).
    pub renames_limited: bool,
}

/// 지운 항목과 새 항목을 git처럼 rename으로 짝짓는다([`crate::renames`]).
///
/// `extra`는 저장소에 없는 blob(작업트리 파일을 git처럼 해시한 것)의 내용이다.
pub(crate) fn pair_renames(
    repo: &gix::Repository,
    raw: Vec<TreeChange>,
    limit: usize,
    extra: &HashMap<ObjectId, Vec<u8>>,
) -> Changes {
    let (mut deletions, mut additions) = (Vec::new(), Vec::new());
    for (i, change) in raw.iter().enumerate() {
        match change {
            TreeChange::Deletion { .. } => deletions.push(i),
            TreeChange::Addition { .. } => additions.push(i),
            _ => {}
        }
    }
    let candidate = |i: usize| match &raw[i] {
        TreeChange::Deletion {
            location,
            entry_mode,
            id,
            ..
        }
        | TreeChange::Addition {
            location,
            entry_mode,
            id,
            ..
        } => crate::renames::Candidate {
            path: location.as_slice(),
            id: *id,
            kind: entry_mode.kind(),
        },
        _ => unreachable!("지운 항목과 새 항목만 고른다"),
    };
    let sources: Vec<_> = deletions.iter().map(|&i| candidate(i)).collect();
    let dests: Vec<_> = additions.iter().map(|&i| candidate(i)).collect();
    let found = crate::renames::find(repo, &sources, &dests, limit, extra);
    drop((sources, dests));
    let mut slots: Vec<Option<TreeChange>> = raw.into_iter().map(Some).collect();
    for &(s, d, _) in &found.pairs {
        let (
            Some(TreeChange::Deletion {
                location: source_location,
                entry_mode: source_entry_mode,
                relation: source_relation,
                id: source_id,
            }),
            Some(TreeChange::Addition {
                location,
                entry_mode,
                relation,
                id,
            }),
        ) = (slots[deletions[s]].take(), slots[additions[d]].take())
        else {
            unreachable!("짝은 지운 항목과 새 항목이다");
        };
        slots[additions[d]] = Some(TreeChange::Rewrite {
            source_location,
            source_entry_mode,
            source_relation,
            source_id,
            diff: None,
            entry_mode,
            id,
            location,
            relation,
            copy: false,
        });
    }
    Changes {
        list: slots.into_iter().flatten().collect(),
        renames_incomplete: found.incomplete,
        renames_limited: found.limited,
    }
}

/// rename 원본 후보를 이만큼만 내용까지 비교한다(크기로 거른 뒤, 이름이 비슷한 것부터).
const MAX_RENAME_CANDIDATES: usize = 4000;

impl Repo {
    /// `id` 커밋이 첫 부모의 다른 파일을 옮겨(rename) `path`를 만들었다면 그 원래 경로.
    ///
    /// `git log --follow`처럼 이 파일 하나의 원본만 찾는다: 그 커밋에서 지운 파일 가운데 내용이 같은 것,
    /// 없으면 git과 같은 기준의 유사도([`crate::similarity`])가 50% 이상인 것 중 가장 비슷한 것(같으면 파일 이름이
    /// 같은 것). 커밋 전체의 rename을 찾지 않으므로 파일 수백 개를 옮긴 커밋에서도 빠르다.
    /// 복사(copy)는 따라가지 않는다. 얕은 클론의 경계처럼 부모가 없으면 `None`.
    pub fn rename_source(&self, id: ObjectId, path: &str) -> Result<Option<String>> {
        let repo = self.local();
        let commit = repo.find_commit(id).map_err(|e| err("커밋 읽기", e))?;
        let Some(parent) = commit.parent_ids().next().map(|id| id.detach()) else {
            return Ok(None);
        };
        self.rename_source_from(id, parent, path)
    }

    /// [`Repo::rename_source`]와 같고 첫 부모 대신 `parent`와 비교한다(merge의 다른 부모: 곁가지에서 옮긴 파일).
    pub fn rename_source_from(
        &self,
        id: ObjectId,
        parent: ObjectId,
        path: &str,
    ) -> Result<Option<String>> {
        use crate::similarity::{self, MINIMUM_SCORE, Spans};

        let repo = self.local();
        let commit = repo.find_commit(id).map_err(|e| err("커밋 읽기", e))?;
        let Some(parent) = repo
            .try_find_object(parent)
            .map_err(|e| err("부모 커밋", e))?
        else {
            return Ok(None);
        };
        let parent_tree = parent.peel_to_tree().map_err(|e| err("부모 트리", e))?;
        let tree = commit.tree().map_err(|e| err("커밋 트리", e))?;
        let Some(dest) = tree
            .lookup_entry_by_path(path)
            .map_err(|e| err("트리 조회", e))?
        else {
            return Ok(None);
        };
        // git처럼 일반 파일만 내용으로 비교한다(심볼릭 링크는 내용이 같을 때만 rename).
        let dest_regular = dest.mode().is_blob();
        let dest_id = dest.object_id();
        if parent_tree
            .lookup_entry_by_path(path)
            .map_err(|e| err("트리 조회", e))?
            .is_some()
        {
            return Ok(None);
        }

        let options = gix::diff::Options::default().with_rewrites(None);
        let deleted: Vec<(String, ObjectId, bool)> = repo
            .diff_tree_to_tree(Some(&parent_tree), Some(&tree), options)
            .map_err(|e| err("트리 비교", e))?
            .into_iter()
            .filter_map(|change| match change {
                TreeChange::Deletion {
                    location,
                    entry_mode,
                    id,
                    ..
                } if !entry_mode.is_tree() && !entry_mode.is_commit() => Some((
                    location.to_str_lossy().into_owned(),
                    id,
                    entry_mode.is_blob(),
                )),
                _ => None,
            })
            .collect();
        let name = file_name(path);
        let same_name = |candidate: &str| file_name(candidate) == name;
        // git(diffcore-rename의 find_basename_matches)은 지운 파일 가운데 이름이 같은 것이 하나뿐일 때만
        // 이름으로 짝을 짓는다. 둘 이상이면 모두 내용으로 비교해 가장 비슷한 것을 고른다.
        let unique_name = deleted.iter().filter(|(p, ..)| same_name(p)).count() == 1;

        // 1. 내용이 같은 파일(파일 이름이 같은 것을 먼저)
        let mut exact = deleted.iter().filter(|(_, id, _)| *id == dest_id);
        if let Some(first) = exact.next() {
            let best = std::iter::once(first)
                .chain(exact)
                .find(|(p, ..)| same_name(p))
                .unwrap_or(first);
            return Ok(Some(best.0.clone()));
        }
        if !dest_regular {
            return Ok(None);
        }

        // 2. 비슷한 파일. 이름이 닮은 것부터, 크기로 먼저 거르고(헤더만 읽는다) 내용을 비교한다.
        let dest_data = repo
            .find_object(dest_id)
            .map_err(|e| err("blob 읽기", e))?
            .detach()
            .data;
        if dest_data.is_empty() {
            return Ok(None);
        }
        let dest_size = dest_data.len() as u64;
        let dest_spans = Spans::new(&dest_data, !crate::text::is_binary(&dest_data));
        let mut candidates: Vec<(u8, &str, ObjectId)> = deleted
            .iter()
            .filter(|(.., regular)| *regular)
            .map(|(candidate, id, _)| (affinity(candidate, path), candidate.as_str(), *id))
            .collect();
        candidates.sort_by_key(|(affinity, ..)| std::cmp::Reverse(*affinity));

        let mut best: Option<(u64, bool, &str)> = None;
        let mut compared = 0;
        for (affinity, candidate, id) in candidates {
            let size = repo
                .find_header(id)
                .map_err(|e| err("blob 크기", e))?
                .size();
            if !similarity::sizes_may_match(size, dest_size) {
                continue;
            }
            compared += 1;
            if compared > MAX_RENAME_CANDIDATES {
                break;
            }
            let data = repo
                .find_object(id)
                .map_err(|e| err("blob 읽기", e))?
                .detach()
                .data;
            let score = similarity::score(
                &Spans::new(&data, !crate::text::is_binary(&data)),
                &dest_spans,
            );
            if score < MINIMUM_SCORE {
                continue;
            }
            let named = affinity == AFFINITY_SAME_NAME;
            // 지운 파일 가운데 하나뿐인 같은 이름이고 아주 비슷하면(75% 이상) 다른 후보를 보지 않는다
            // (git의 파일 이름 지름길과 같은 조건·기준).
            if named && unique_name && score >= (MINIMUM_SCORE + similarity::MAX_SCORE) / 2 {
                return Ok(Some(candidate.to_owned()));
            }
            if best.is_none_or(|(s, n, _)| score > s || (score == s && named && !n)) {
                best = Some((score, named, candidate));
            }
        }
        Ok(best.map(|(.., candidate)| candidate.to_owned()))
    }
}

const AFFINITY_SAME_NAME: u8 = 2;

/// 이름이 얼마나 닮았나: 파일 이름이 같으면 2, 확장자를 뺀 이름이 같으면 1(`git-log.txt` → `git-log.adoc`).
fn affinity(candidate: &str, path: &str) -> u8 {
    let (a, b) = (file_name(candidate), file_name(path));
    if a == b {
        AFFINITY_SAME_NAME
    } else if stem(a) == stem(b) {
        1
    } else {
        0
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// 트리 비교 결과 하나(gix).
pub(crate) type TreeChange = gix::object::tree::diff::ChangeDetached;

/// `git blame --incremental`의 출력을 바꾸는 설정. 화면에만 영향을 주는 것(`blame.date` 등)도 넣어 둔다:
/// 빠뜨리면 틀린 결과가 남지만 더 넣으면 캐시를 한 번 다시 채울 뿐이다.
const BLAME_SETTINGS: &[&str] = &[
    "blame.ignoreRevsFile",
    "blame.markUnblamableLines",
    "blame.markIgnoredLines",
    "blame.showRoot",
    "blame.blankBoundary",
    "diff.indentHeuristic",
    "diff.algorithm",
    "core.autocrlf",
    "core.eol",
    "core.safecrlf",
    "mailmap.file",
    "mailmap.blob",
];

/// 작성자(`%aN`·`%aE`)를 바꾸는 설정. 저장소의 `.mailmap`은 설정이 없어도 늘 본다.
const MAILMAP_SETTINGS: &[&str] = &["mailmap.file", "mailmap.blob"];

/// `git log -L`의 결과를 바꾸는 설정: 줄 범위를 옛 버전으로 옮기는 diff와 옛 이름을 찾는 rename.
const LINE_LOG_SETTINGS: &[&str] = &[
    "diff.algorithm",
    "diff.indentHeuristic",
    "diff.interHunkContext",
    "diff.context",
    "diff.renames",
    "diff.renameLimit",
    "mailmap.file",
    "mailmap.blob",
];

/// git처럼 `core.useReplaceRefs=false`나 `GIT_NO_REPLACE_OBJECTS`면 바꾸는 참조를 보지 않는다.
pub(crate) fn replace_enabled(repo: &gix::Repository) -> bool {
    std::env::var_os("GIT_NO_REPLACE_OBJECTS").is_none()
        && repo.config_snapshot().boolean("core.useReplaceRefs") != Some(false)
}

/// 바꾸는 참조가 놓이는 곳(`GIT_REPLACE_REF_BASE`, 기본 `refs/replace/`)
pub(crate) fn replace_base() -> String {
    let mut base = std::env::var("GIT_REPLACE_REF_BASE")
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "refs/replace/".to_owned());
    if !base.ends_with('/') {
        base.push('/');
    }
    base
}

/// `git replace`로 바꾼 객체가 있는가(바꾸는 참조를 보지 않게 했으면 없는 것으로 본다).
pub(crate) fn has_replace_refs(repo: &gix::Repository) -> bool {
    if !replace_enabled(repo) {
        return false;
    }
    let base = replace_base();
    let Ok(platform) = repo.references() else {
        return false;
    };
    let Ok(refs) = platform.prefixed(base.as_str()) else {
        return false;
    };
    refs.flatten().any(|reference| {
        reference
            .name()
            .as_bstr()
            .strip_prefix(base.as_bytes())
            .is_some_and(|hex| ObjectId::from_hex(hex).is_ok())
    })
}

/// `<공통 디렉터리>/info/grafts`: 줄마다 `<커밋> <부모>*`, `#`은 주석
pub(crate) fn read_grafts(common_dir: &Path) -> HashMap<ObjectId, Vec<ObjectId>> {
    let Ok(text) = std::fs::read_to_string(common_dir.join("info").join("grafts")) else {
        return HashMap::new();
    };
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| {
            let mut ids = line
                .split_whitespace()
                .map(|hex| ObjectId::from_hex(hex.as_bytes()).ok());
            let commit = ids.next()??;
            let parents: Option<Vec<ObjectId>> = ids.collect();
            Some((commit, parents?))
        })
        .collect()
}

/// 설정 파일들의 크기·수정 시각. gix는 저장소를 열 때 설정을 한 번 읽으므로, 상주하는 앱이 `git config`로
/// 바꾼 값(줄 끝, blame 설정)을 보려면 파일이 바뀌었을 때 다시 열어야 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigStamp(Vec<(PathBuf, Option<(u64, SystemTime)>)>);

impl ConfigStamp {
    fn of(repo: &gix::Repository) -> ConfigStamp {
        let mut paths: Vec<PathBuf> = repo
            .config_snapshot()
            .plumbing()
            .sections()
            .filter_map(|section| section.meta().path.clone())
            .collect();
        // 열 때 없던 파일이 생겨도 알아차리도록 늘 보는 자리
        paths.push(repo.common_dir().join("config"));
        paths.push(repo.git_dir().join("config.worktree"));
        if let Some(home) = gix::path::env::home_dir() {
            paths.push(home.join(".gitconfig"));
        }
        if let Some(xdg) = gix::path::env::xdg_config("config", &mut |name| std::env::var_os(name))
        {
            paths.push(xdg);
        }
        paths.sort();
        paths.dedup();
        ConfigStamp(
            paths
                .into_iter()
                .map(|path| {
                    let stamp = file_stamp(&path);
                    (path, stamp)
                })
                .collect(),
        )
    }

    fn is_current(&self) -> bool {
        self.0
            .iter()
            .all(|(path, stamp)| file_stamp(path) == *stamp)
    }
}

fn file_stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// 열어 둔 저장소 캐시. 같은 저장소를 다시 열지 않아 gix의 객체 캐시를 이어 쓰고,
/// 디렉터리 → 저장소 찾기 결과도 기억한다(하위 모듈처럼 중첩된 저장소도 디렉터리마다 정확하다).
/// 설정 파일이 바뀐 저장소는 다시 연다.
#[derive(Default)]
pub struct RepoCache {
    inner: Mutex<CacheInner>,
}

#[derive(Default)]
struct CacheInner {
    by_dir: HashMap<PathBuf, PathBuf>,
    repos: HashMap<PathBuf, Arc<Repo>>,
}

impl RepoCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// `path`(파일·디렉터리, 지금은 없는 경로도 된다)를 담은 저장소.
    pub fn for_path(&self, path: &Path) -> Result<Arc<Repo>> {
        let dir = existing_ancestor(path)
            .map(|p| {
                if p.is_dir() {
                    p
                } else {
                    p.parent().unwrap_or(p)
                }
            })
            .ok_or_else(|| Error(format!("경로가 없다: {}", path.display())))?
            .to_path_buf();
        let opened = {
            let inner = self.inner.lock().unwrap();
            inner
                .by_dir
                .get(&dir)
                .and_then(|w| inner.repos.get(w))
                .cloned()
        };
        // 설정 파일 확인과 저장소 열기는 파일을 읽으므로 잠금 밖에서 한다.
        if let Some(repo) = &opened
            && repo.config.is_current()
        {
            return Ok(repo.clone());
        }
        let repo = Arc::new(Repo::discover(&dir)?);
        let mut inner = self.inner.lock().unwrap();
        let workdir = repo.workdir().to_path_buf();
        inner.by_dir.insert(dir, workdir.clone());
        // 같은 설정으로 이미 연 것(다른 디렉터리로 찾았거나 그사이 다른 스레드가 열었다)이 있으면 그것을 쓴다.
        if let Some(current) = inner.repos.get(&workdir)
            && current.config == repo.config
        {
            return Ok(current.clone());
        }
        inner.repos.insert(workdir, repo.clone());
        Ok(repo)
    }

    /// 요청한 파일의 저장소와 저장소 기준 경로(`/` 구분).
    pub fn locate(&self, file: &FilePath) -> Result<(Arc<Repo>, String)> {
        match file {
            FilePath::Os(path) => {
                let repo = self.for_path(path)?;
                let rel = repo.relative_path(path)?;
                Ok((repo, rel))
            }
            FilePath::InRepo { workdir, path } => {
                let rel = checked_relative(path)?;
                Ok((self.for_path(workdir)?, rel))
            }
        }
    }
}

/// 사람(작성자·커미터)과 시각.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub email: String,
    /// Unix 시각(초)
    pub time: i64,
    /// UTC와의 차이(초)
    pub tz_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub sha: String,
    pub parents: Vec<String>,
    pub summary: String,
    pub message: String,
    pub author: Person,
    pub committer: Person,
    /// 첫 부모와 비교해 바뀐 파일 수
    pub changed_files: usize,
}

pub(crate) fn person(signature: gix::actor::SignatureRef<'_>, encoding: Option<&BStr>) -> Person {
    let signature = signature.trim();
    let time = signature.time().unwrap_or_default();
    Person {
        name: decode_text(signature.name, encoding),
        email: decode_text(signature.email, encoding),
        time: time.seconds,
        tz_offset: time.offset,
    }
}

/// 커밋의 `encoding` 헤더(예: euc-kr)에 맞춰 문자열로 바꾼다. 헤더가 없으면 UTF-8이다.
/// 디코드할 수 없는 바이트는 U+FFFD로 바꾼다.
pub fn decode_text(bytes: &BStr, encoding: Option<&BStr>) -> String {
    if let Some(label) = encoding
        && let Some(encoding) = encoding_rs::Encoding::for_label(label.trim())
        && encoding != encoding_rs::UTF_8
    {
        let (text, _, _) = encoding.decode(bytes);
        return text.into_owned();
    }
    bytes.to_str_lossy().into_owned()
}

/// `path`부터 위로 올라가며 처음 존재하는 경로.
fn existing_ancestor(path: &Path) -> Option<&Path> {
    path.ancestors().find(|p| p.exists())
}

/// 존재하는 가장 가까운 조상까지 canonicalize하고 나머지(아직 없거나 삭제된 경로)는 그대로 붙인다.
fn canonicalize_lenient(path: &Path) -> Result<PathBuf> {
    let base =
        existing_ancestor(path).ok_or_else(|| Error(format!("경로가 없다: {}", path.display())))?;
    let mut out = std::fs::canonicalize(base).map_err(|e| err("경로", e))?;
    if let Ok(rest) = path.strip_prefix(base) {
        out.push(rest);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_euc_kr_message() {
        let bytes = b"\xbe\xc8\xb3\xe7\xc7\xcf\xbc\xbc\xbf\xe4";
        assert_eq!(
            decode_text(bytes.as_bstr(), Some(b"euc-kr".as_bstr())),
            "안녕하세요"
        );
        assert_eq!(decode_text("안녕".as_bytes().as_bstr(), None), "안녕");
        assert_eq!(decode_text(b"\xff".as_bstr(), None), "\u{fffd}");
    }
}
