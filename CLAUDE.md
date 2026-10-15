# parallax (가칭) — 로컬 git 이력 뷰어

작업을 시작할 때마다 이 파일과 `docs/MILESTONES.md`의 "진행 중" 마일스톤을 먼저 읽는다.
마일스톤을 끝내거나 범위·설계를 바꾸면 두 문서를 함께 갱신한다.

## 정의
로컬 git 이력을 생각하는 속도로 보는 **읽기 전용** 데스크탑 뷰어.
GitLens의 핵심 흐름(줄 → 커밋 → 부모 리비전 blame → 파일·라인 히스토리 → 그래프)을 터미널 명령 한 번으로 연다.

## 범위
- 한다: blame, 파일·라인 히스토리, 커밋 상세와 diff, 커밋 그래프, 저장소 변경 실시간 반영
- 안 한다: 편집, LSP, 내장 터미널, 스테이징·커밋·푸시 등 저장소를 바꾸는 작업 (Claude Code와 lazygit의 몫)
- 범위를 넓히려면 아래 "결정 기록"에 이유를 먼저 남긴다.

## 최우선 원칙: 속도
1. **상주 + CLI 전달.** 앱은 단일 인스턴스로 상주한다(창 닫기 = 숨기기, 종료는 트레이 메뉴).
   `plx` 호출은 argv와 cwd만 실행 중인 인스턴스에 넘긴다. 콜드 스타트는 하루 한 번만 겪는 비용으로 만든다.
2. **읽기는 인프로세스, 이력 질의는 git CLI 스트리밍.**
   - refs·커밋·트리·blob 읽기, 리비전 워크, 트리 diff → `gix` (프로세스 생성 없음. 윈도우에서 특히 중요)
   - blame → `git blame --incremental`, 파일 히스토리 → `git log`, 라인 히스토리 → `git log -L`
     (git의 changed-path Bloom 필터 혜택을 그대로 받는다)
   - `gix-blame`은 실험 플래그 뒤에만 둔다. git 결과와의 골든 테스트를 통과하기 전에는 기본값으로 바꾸지 않는다.
3. **불변 결과는 영구 캐시.** 특정 커밋 기준 blame·이력은 바뀌지 않는다 → (저장소, 커밋 sha, 경로) 키로 디스크 캐시.
   작업트리 blame만 (HEAD sha, 파일 크기 + mtime) 키를 쓴다.
4. **UI를 막지 않는다.** 결과는 `tauri::ipc::Channel`로 청크 단위 스트리밍하고 첫 청크를 바로 그린다. 긴 목록은 가상화한다.
5. **측정 없이 최적화하지 않는다.** 모든 요청의 소요 시간을 개발 빌드의 성능 HUD에 표시한다.
   목표치는 `docs/MILESTONES.md`의 성능 예산을 따른다.

## 스택
- Tauri 2, Rust stable, React + TypeScript + Vite
- 코드 뷰: CodeMirror 6 (읽기 전용 + 커스텀 거터), diff: `@codemirror/merge`. Monaco는 쓰지 않는다(초기화·번들 비용).
- git: `gix` + 시스템 `git` CLI
- 파일 감시: `notify` (`.git`의 HEAD, refs/, packed-refs, logs/HEAD)
- Node는 Volta로 고정한다. 각 의존성 버전은 M0에서 확인해 아래에 기록한다.
  - (M0에서 기록)

## 디렉터리
```
crates/plx-core/   gix·git CLI 래퍼, 출력 파서, 캐시, 그래프 레인 레이아웃 (UI 무관, 단위 테스트 필수)
src-tauri/         Tauri 명령·채널, 단일 인스턴스, 트레이, CLI 인자 처리
src/               React UI
bench/             참조 저장소 준비 스크립트와 측정 결과(results.md)
fixtures/          테스트 저장소 생성 스크립트 (rename, 이동, merge, CRLF, 한글 경로·내용)
docs/              MILESTONES.md
```

## 반드시 지킬 것
- **인코딩:** 모든 파일은 UTF-8(BOM 없음)로 저장한다. 한글이 들어간 파일을 쓴 뒤에는 다시 읽어 깨지지 않았는지 확인한다.
- **한글 경로:** git CLI는 항상 `-c core.quotepath=false`로 호출하고, 지원하는 명령은 `-z` 출력으로 파싱한다.
  git 출력은 바이트로 받아 경계에서만 UTF-8로 디코드한다.
- **윈도우:** 모든 자식 프로세스에 `CREATE_NO_WINDOW`. git 스폰 횟수를 줄이는 선택을 우선한다.
- **플랫폼:** Windows(Git Bash)와 macOS 모두 1급 지원. 한쪽에서만 확인한 기능은 완료로 치지 않는다.
- **읽기 전용:** 저장소를 바꾸는 git 명령은 호출하지 않는다. 유일한 예외는 사용자가 확인한 "가속 설정"(commit-graph 작성).
- **파서 정확성:** git 출력 파서마다 fixtures 기반 골든 테스트(`git blame --porcelain` 등 git 결과와 비교)를 둔다.

## 명령 (스캐폴딩 후 실제 값으로 갱신)
- 개발: `npm run tauri dev`
- 코어 테스트: `cargo test -p plx-core`
- 벤치: `bench/` 스크립트 (M0에서 작성)

## CLI 인터페이스 (목표)
```
plx                          현재 저장소 그래프
plx <file>[:<line>]          blame, 해당 줄 선택
plx log <file>               파일 히스토리
plx log -L <a>,<b>:<file>    라인 범위 히스토리
plx show <rev>               커밋 상세
```
macOS는 PATH에 셸 shim(`plx` → 앱 번들 실행 파일)을 둔다.

## 키맵
- 탐색은 tig와 같게: `j`/`k` 이동, `Enter` 커밋 상세, `,` 부모 리비전 blame, `/` 검색, `q` 뒤로
- 추가 키: `<`/`>` 이전·다음 리비전, `H` 파일 히스토리, `L` 라인 히스토리, `G` 그래프, `y` sha 복사, `o` 에디터로 열기, `?` 도움말

## 결정 기록
- 2026-10-15: 범위를 읽기 전용 git 뷰어로 한정. 편집·터미널·LSP는 제외.
- 2026-10-15: blame·라인 히스토리는 git CLI로 시작. gix-blame은 골든 테스트 통과 시 전환 검토.
- 2026-10-15: 코드 뷰는 Monaco 대신 CodeMirror 6.