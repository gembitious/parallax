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
  정확한 버전은 `Cargo.lock`·`package-lock.json`이 고정한다(`.npmrc`의 `save-exact`). 2026-10-15 확인.
  - 툴체인: Rust 1.97.0 + edition 2024 (`rust-toolchain.toml`), Node 24.22.0 LTS + npm 11.19.1 (`package.json`의 `volta`), Volta 2.0.2
  - Tauri: `tauri` 2.12.4, `tauri-build` 2.7.3, `@tauri-apps/api` 2.12.4, `@tauri-apps/cli` 2.12.3 (3.0은 alpha라 보류)
  - 프런트: React 19.3.1, Vite 8.3.4, `@vitejs/plugin-react` 6.1.2, TypeScript 6.0.3 (7.0은 Go 포트라 도구 호환성 확인 전까지 보류)
  - Rust 공통: `serde` 1.0.229, `serde_json` 1.0.154
  - 도입 예정 (그 마일스톤에서 다시 확인): `gix` 0.89.0 (M1), `codemirror` 6.0.2·`@codemirror/view` 6.43.14·`@codemirror/merge` 6.12.2 (M1·M2), `notify` 8.2.0 (M4, 9.0은 RC)

## 디렉터리
```
crates/plx-core/   gix·git CLI 래퍼, 출력 파서, 캐시, 그래프 레인 레이아웃, plx 인자 파서, 인스턴스 간 전달(로컬 소켓)
                   (UI 무관, 단위 테스트 필수)
src-tauri/         Tauri 명령·채널, 단일 인스턴스·CLI 호출을 창·트레이에 연결
scripts/           설치 보조 스크립트 (macOS shim)
src/               React UI
bench/             참조 저장소 준비 스크립트, git 측정 하네스(plx-bench), 앱 지연 측정, 측정 결과(results.md)
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

## 명령
- 처음 한 번: `npm install` (Node·npm 버전은 Volta가, Rust 버전은 rustup이 `rust-toolchain.toml`로 맞춘다)
- 개발: `npm run tauri dev`
- 릴리스 빌드: `npm run tauri build` (번들 없이 실행 파일만: `npm run tauri build -- --no-bundle` → `target/release/plx`)
- 프런트 타입 검사 + 빌드: `npm run build`
- 코어 테스트: `cargo test -p plx-core` / 전체: `cargo test --workspace`
  (fixtures 결정성 테스트가 `fixtures/make-repos.sh`를 실행하므로 bash와 git이 필요하다. Windows는 Git for Windows의 bash, `PLX_BASH`로 지정 가능)
- 테스트 저장소 만들기: `fixtures/make-repos.sh <출력 디렉터리>` → `history`, `encoding` 저장소 (어느 플랫폼에서든 같은 sha)
- 앱 지연 측정(릴리스 빌드 후): `bench/app-latency.sh [실행 파일] [반복 횟수]` → 콜드 스타트, plx 호출 → 첫 페인트
- git baseline: `bench/setup.sh`(참조 저장소 준비, 한 번) → `bench/run.sh [이름]`(측정 → `bench/results.md`)

## CLI 인터페이스 (목표)
```
plx                          현재 저장소 그래프
plx <file>[:<line>]          blame, 해당 줄 선택
plx log <file>               파일 히스토리
plx log -L <a>,<b>:<file>    라인 범위 히스토리
plx show <rev>               커밋 상세
```
`-h`/`--help`, `-V`/`--version`, `--`(뒤 인자를 파일 이름으로: `plx -- log`)도 받는다.
blame은 작업트리에 있는 파일만 받고, `log`는 삭제된 경로도 받는다. 잘못된 인자는 상주 인스턴스에 넘기지 않고
호출한 터미널에 알린 뒤 종료 코드 2로 끝난다.

macOS는 PATH에 셸 shim(`plx` → 앱 번들 실행 파일)을 둔다: `scripts/install-macos-shim.sh [앱 경로] [설치 디렉터리]`.

인스턴스가 없을 때 릴리스 빌드를 셸에서 실행하면 스스로 분리 실행하고 셸을 바로 돌려준다.

환경 변수
- `PLX_DETACH=0|1`: 분리 실행 끄기·강제 (개발 빌드는 기본적으로 분리하지 않는다. `tauri dev`가 자식 프로세스로 잡고 있어야 한다)
- `PLX_T0`: 호출 시작 시각(epoch ms). 없으면 OS가 plx 프로세스를 만든 시각이 지연 측정 기준점이다.
- `PLX_PERF=1`: 릴리스 빌드에서도 성능 HUD를 켠다(개발 빌드는 항상 켜짐. `` ` `` 키로 보이기·숨기기).
  `PLX_PERF_LOG=<파일>`을 함께 주면 끝난 요청을 JSON 한 줄씩 기록한다(`bench/app-latency.sh`가 쓴다).

## 키맵
- 탐색은 tig와 같게: `j`/`k` 이동, `Enter` 커밋 상세, `,` 부모 리비전 blame, `/` 검색, `q` 뒤로
- 추가 키: `<`/`>` 이전·다음 리비전, `H` 파일 히스토리, `L` 라인 히스토리, `G` 그래프, `y` sha 복사, `o` 에디터로 열기, `?` 도움말

## 결정 기록
- 2026-10-15: 범위를 읽기 전용 git 뷰어로 한정. 편집·터미널·LSP는 제외.
- 2026-10-15: blame·라인 히스토리는 git CLI로 시작. gix-blame은 골든 테스트 통과 시 전환 검토.
- 2026-10-15: 코드 뷰는 Monaco 대신 CodeMirror 6.
- 2026-10-15: 단일 인스턴스는 `tauri-plugin-single-instance` 대신 직접 구현한다(`plx_core::instance`).
  `main()` 맨 앞, Tauri 초기화 전에 로컬 소켓(Windows named pipe, Unix 소켓)으로 argv·cwd·t0를 넘기고 끝낸다.
  primary 판정은 락 파일(`File::try_lock`). 이유: 두 번째 호출이 WebView 초기화를 거치지 않는다,
  터미널에서 처음 실행할 때 셸을 돌려주려면(분리 재실행) Tauri 전에 판정해야 한다, 측정용 t0를 함께 넘길 수 있다.