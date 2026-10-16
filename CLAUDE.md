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
  - Rust 공통: `serde` 1.0.229, `serde_json` 1.0.154, `libc` 0.2.190(Unix), `windows-sys` 0.61.2(Windows, Tauri와 같은 버전),
    `tempfile` 3.27.0(테스트)
  - git·텍스트(M1): `gix` 0.89.0 (`default-features = false`, `basic`·`sha1`·`max-performance-safe`. diff는 gix에 든 imara-diff),
    `encoding_rs` 0.8.42 (EUC-KR 커밋 메시지·파일 내용), `dunce` 1.0.5 (Windows `\\?\` 경로 정리, Tauri가 이미 쓰는 버전)
  - 코드 뷰(M1): `@codemirror/state` 6.7.6, `@codemirror/view` 6.43.14, `@codemirror/language` 6.13.1,
    `@codemirror/language-data` 6.5.2(언어 모듈은 필요할 때 불러온다), `@codemirror/search` 6.7.2, `@codemirror/commands` 6.11.1,
    `@lezer/highlight` 1.2.5. `codemirror` 묶음 패키지(basicSetup)는 쓰지 않는다(편집 기능이 필요 없다).
  - 프런트 테스트: `vitest` 5.0.3 (순수 로직만, `vitest.config.ts`가 `src/**/*.test.ts`만 본다)
  - diff 뷰(M2): `@codemirror/merge` 6.12.2 (분할 MergeView, 통합 unifiedMergeView)
  - 감시(M4): `notify` 8.2.0 (`default-features = false`, `macos_fsevent`. 9.0은 RC라 보류). Windows에서 `windows-sys`
    0.60을 따로 끌어온다(선언뿐인 크레이트라 크기 영향은 작다). 2026-10-15 확인.

## 디렉터리
```
crates/plx-core/   gix·git CLI 래퍼, 출력 파서, 캐시, 그래프 레인 레이아웃, plx 인자 파서, 인스턴스 간 전달(로컬 소켓)
                   (UI 무관, 단위 테스트 필수)
src-tauri/         Tauri 명령·채널, 단일 인스턴스·CLI 호출을 창·트레이에 연결
scripts/           설치·확인 스크립트 (macOS shim, 앱 자동 확인 check-app.sh)
.github/workflows/ CI: Windows·macOS·Linux에서 검사(포맷·clippy·테스트)와 앱 확인(릴리스 빌드 + check-app.sh)
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
- 프런트 타입 검사 + 빌드: `npm run build` / 프런트 단위 테스트(순수 로직, vitest): `npm test`
- 코어 테스트: `cargo test -p plx-core` / 전체: `cargo test --workspace`
  (fixtures 결정성 테스트가 `fixtures/make-repos.sh`를 실행하므로 bash와 git이 필요하다. Windows는 Git for Windows의 bash, `PLX_BASH`로 지정 가능)
- 테스트 저장소 만들기: `fixtures/make-repos.sh <출력 디렉터리>` → `history`, `encoding` 저장소 (어느 플랫폼에서든 같은 sha)
- 앱 자동 확인(릴리스 빌드 후): `scripts/check-app.sh [실행 파일]` → 단일 인스턴스 전달·CLI 오류·동시 실행·분리 실행,
  blame이 끝까지 와서 거터를 그리는지(한글 경로 저장소 포함) 확인과 지연 측정.
  `PLX_CHECK_RECORD=<이름>`이면 지연 표를 `bench/results.md`에 넣는다. CI도 이 스크립트를 돌린다.
- 앱 지연만 측정: `bench/app-latency.sh [실행 파일] [반복 횟수]` → 콜드 스타트, plx 호출 → 첫 페인트, blame 첫 줄 표시
- git baseline: `bench/setup.sh`(참조 저장소 준비, 한 번) → `bench/run.sh [이름]`(측정 → `bench/results.md`)

## 작업 방식
- 적당한 작업 단위마다 PR을 만든다. 앞 PR이 아직 머지되지 않았으면 그 브랜치 위에 쌓고 PR 설명에 머지 순서를 적는다.
- CI(`.github/workflows/ci.yml`)가 Windows·macOS·Linux에서 검사와 앱 확인을 돌린다. 초록이어야 머지한다.
  CI가 확인한 기능은 그 플랫폼에서 확인한 것으로 친다. 화면을 봐야 하는 확인(창 포커스, 닫기 = 숨기기, 트레이 등)은
  PR 설명의 "수동 확인"에 남기고, 사용자가 따라 할 단계는 `docs/MANUAL_CHECKS.md`에 모아 갱신한다.
- 저장소가 비공개라 Actions 사용 시간이 든다(macOS 러너는 보통 10배로 계산). 같은 브랜치의 이전 실행은 자동 취소되고,
  문서(`*.md`, `docs/`)만 바뀐 푸시는 CI를 돌리지 않는다.

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
- `PLX_CACHE_DIR=<디렉터리>`: blame 캐시 위치(기본: OS 캐시 위치의 `com.gembitious.parallax/blame`, 한도 512MB).
  측정·자동 확인 스크립트는 임시 디렉터리를 줘서 사용자 캐시와 섞지 않는다.
- `PLX_EDITOR`: `o`(에디터로 열기)가 띄울 에디터. 이름·경로(`code`, `zed`, `cursor`, `subl`, `idea` …, 줄 넘기는 법을
  안다) 또는 `{file}`·`{line}`·`{col}`을 넣은 명령 틀(`"my-editor --line {line} {file}"`). 없으면 `VISUAL`·`EDITOR`가
  아는 GUI 에디터일 때 그것, 다음은 PATH의 `code`·`cursor`·`windsurf`·`zed`·`subl`, 그다음 설치 위치(Windows VS Code,
  macOS VS Code 앱)를 쓴다. 이 변수들과 PATH는 마지막으로 `plx`를 부른 셸의 값을 먼저 본다(`plx`가 상주 인스턴스에
  넘긴다. Finder·시작 메뉴에서 뜬 앱의 PATH에는 `code`가 없을 수 있다). 바꾼 값은 다음 `plx` 호출부터 쓴다.

## 키맵
- 탐색은 tig와 같게: `j`/`k` 이동, `Enter` 커밋 상세, `,` 부모 리비전 blame, `/` 검색, `q` 뒤로
- 추가 키: `<`/`>` 이전·다음 리비전, `H` 파일 히스토리, `L` 라인 히스토리, `G` 그래프, `W` 워크트리, `f`·Ctrl+P(⌘P) 파일 찾기,
  `y` sha 복사, `o` 에디터로 열기, `?` 도움말
- 탐색 스택: `,`·`<`·`>`는 새 화면을 쌓고 `q`가 하나씩 돌아간다(아래 화면은 숨긴 채 유지해 커서·스크롤이 남는다).
  `<`로 온 화면에서 `>`(또는 반대)는 새로 쌓지 않고 돌아간다. 맨 아래에서 `q`는 창을 숨긴다.
- 리비전 이동은 첫 부모를 따라 파일을 바꾼 커밋 단위다(`git log --first-parent --follow`). merge가 바꾼 버전도 포함한다.
- 파일 히스토리 화면: `j`/`k` 이동, `Enter` 커밋 상세(그 파일 diff만, 상세에서 `a`로 모든 파일), `b` 그 리비전 blame,
  `,` 바꾸기 전 리비전 blame, `y` sha 복사, `/`·`n`·`N` 검색. blame의 `H`는 그 리비전부터의 히스토리다.
- 라인 히스토리: blame에서 `J`/`K`로 줄 선택을 넓히고 `L`(선택이 없으면 커서 줄). 같은 목록 화면에 고른 커밋의
  그 범위 hunk가 아래에 보인다. 작업트리 blame의 줄 번호는 HEAD 버전의 줄로 옮겨 `git log -L`에 준다.
- 그래프: 어느 화면에서든 `G`는 그 커밋(blame은 커서 줄의 커밋)을 고른 그래프다. 그래프에서 `Enter` 커밋 상세,
  `y` sha 복사, `R` 원격 브랜치 모두 ↔ 로컬 브랜치와 upstream만, `/` 검색(메시지·작성자·sha, `path:경로`는 그 경로를
  바꾼 커밋), `n`/`N` 다음·이전 결과, `Esc` 강조 지우기.
- 명령 팔레트: 어느 화면에서든 `:`·Ctrl+Shift+P(macOS ⌘⇧P). 그 화면에서 할 수 있는 일(키가 없는 그래프 범위·가속 설정,
  성능 HUD 포함)을 이름·초성으로 찾아 `Enter`로 실행하고 옆에 단축키를 보인다.
- 에디터로 열기 `o`: blame은 커서 줄, 커밋 상세는 고른 파일의 첫 바뀐 줄, 히스토리는 그 파일(라인 히스토리면 범위의
  첫 줄), 그래프는 작업트리 폴더. 리비전의 줄은 작업트리 파일의 줄로 옮기고, 그 뒤 이름이 바뀌었으면 새 이름을 연다.
- blame의 `i`: 커서 줄의 커밋 카드를 고정해 보이고 다시 누르면 닫는다(마우스의 거터 클릭과 같다).
- AI 출처: 저장소에 git-ai 노트(`refs/notes/ai`)가 있으면 blame 거터의 AI가 쓴 줄에 보라 줄과 `✦ 도구` 이름이 붙고,
  카드에 에이전트·모델·시킨 사람, 머리줄에 `✦ AI n줄`이 보인다. 따로 켜는 키는 없다.
- 파일 찾기(Quick Open): 어느 화면에서든 `f`(입력란·코드 뷰 밖이면)·Ctrl+P(macOS ⌘P). 추적 파일을 퍼지 검색한다
  (파일 이름 일치 먼저, 초성 `ㅍㅇ`, 공백으로 나눈 낱말은 모두 맞아야 한다, `파일:줄`이면 그 줄로). `↑`/`↓`·Ctrl+N/J/K로
  고르고 `Enter`는 작업트리 blame, `Shift+Enter`는 파일 히스토리. 빈 검색어는 최근 연 파일부터 보인다.
  작업트리에서 지운(아직 커밋하지 않은) 파일은 "지움" 표시이고 `Enter`는 HEAD 버전 blame이다.

## 결정 기록
- 2026-10-15: 범위를 읽기 전용 git 뷰어로 한정. 편집·터미널·LSP는 제외.
- 2026-10-15: blame·라인 히스토리는 git CLI로 시작. gix-blame은 골든 테스트 통과 시 전환 검토.
- 2026-10-15: 코드 뷰는 Monaco 대신 CodeMirror 6.
- 2026-10-15: 결과 캐시(blame, 파일 히스토리)는 파일(JSON) 하나에 결과 하나, 이름은 열쇠의 FNV 해시이고 파일 안의 열쇠로
  충돌을 거른다. 작업트리 blame은 수정 2초 안의 파일을 넣지 않는다(크기·수정 시각이 같은 채로 바뀌는 racy 문제).
  파일 히스토리는 시작 커밋(요청할 때의 HEAD)으로 고정해 넣고, 커밋 2만 개가 넘는 결과는 넣지 않는다.
- 2026-10-15: 파일 히스토리는 `git log --follow` 대신 `git log -- <경로>` 구간을 잇는다(`plx_core::history`).
  구간 안에서 파일이 생긴 커밋(상태 A)마다 rename 원본을 찾아, 그 커밋의 첫 부모부터 옛 이름으로 다음 구간을 돌린다.
  이유(bench/results.md): `--follow`는 rename 앞 구간에서 Bloom 필터를 거의 못 써서 git/git `Documentation/git-log.adoc`
  첫 50개가 1.1초(Bloom 있음)인데 구간 잇기는 85ms다. `--follow`는 diff가 없는 merge(merge에서 바뀐 버전)를 빼고,
  날짜 순 워크 도중 경로를 바꿔 다른 브랜치 커밋이 빠지거나 낀다. 구간마다 git의 기본 이력 단순화를 그대로 따른다.
  rename 원본은 그 파일 하나만 git과 같은 유사도(diffcore의 spanhash를 옮긴 `plx_core::similarity`, 50%)로 찾는다.
  gix의 rename 추적은 짝 수(추가×삭제)가 1,000을 넘으면 유사도 비교를 건너뛰어 파일을 많이 옮긴 커밋을 놓친다.
- 2026-10-15: "가속 설정"은 히스토리 머리줄의 버튼을 눌러 확인한 뒤 `git commit-graph write --reachable --changed-paths`만
  실행한다(`plx_core::commit_graph`). git 2.46 이상이면 `-c commitGraph.changedPathsVersion=2`로 필터 v2를 쓴다
  (v1은 0x80 이상 바이트가 든 경로, 즉 한글 경로의 해시가 `char` 부호에 따라 달라진다). 저장소 설정은 바꾸지 않는다:
  MILESTONES 초안의 `commitGraph.changedPaths`는 git에 없는 설정이고, `--changed-paths`로 쓴 뒤의 commit-graph 쓰기
  (git gc 포함)는 필터를 유지한다. 상태는 commit-graph 파일 머리(청크 표, BDAT 버전)와 HEAD가 그래프에 있는지로 본다.
- 2026-10-15: 커밋 그래프는 gix의 topo 워크 대신 시작점에서 닿는 DAG를 직접 읽어 git의 `--topo-order`와 같은
  LIFO 정렬을 한다(`plx_core::graph`). 커밋은 commit-graph의 위치 번호 → 앱 캐시의 커밋 표(`plx_core::commit_table`)
  → 커밋 객체 순으로 찾는다. 이유(bench/results.md): gix-commitgraph는 세대 번호 v2를 읽지 않아 원격 브랜치가 많으면
  첫 화면이 0.3~0.45초였다. 커밋 표는 commit-graph가 없는 저장소를 위해 한 번 푼 (sha, 부모, 시각)을 저장소 밖에
  둔다(얕은 클론은 넣지 않는다). 기본 범위는 HEAD + 로컬 브랜치 + 그 upstream이고 원격 브랜치 모두·HEAD만은 화면에서 고른다.
  레인은 같은 커밋을 기다리면 왼쪽으로 합치고(git의 graph.c와 같은 열 수) 빈 레인은 당기지 않는다(선이 옆으로 밀리지 않는다).
- 2026-10-15: 라이브 갱신은 `notify`로 `.git`의 HEAD(워크트리별)·`refs/`·`packed-refs`와 작업트리 blame으로 연 파일만
  본다(`plx_core::watch`). objects/·index 쓰기는 이름으로 거르고, 120ms 조용해지면 HEAD·참조 지문(파일은 크기·수정 시각)을
  비교해 달라졌을 때만 화면에 알린다. 그래프는 새 세션의 첫 화면이 오면 바꿔 끼워 깜빡이지 않는다(맨 위를 보고 있었으면
  새 커밋을, 아니면 고른 커밋을 화면의 같은 자리에). 결과 캐시는 커밋 sha·HEAD·파일 크기·수정 시각을 열쇠로 써서
  따로 지울 것이 없다.
- 2026-10-15: Quick Open의 퍼지 매칭은 크레이트(`nucleo-matcher` 0.3.1) 대신 직접 구현한다(`plx_core::fuzzy`).
  점수는 fzf v2(Smith-Waterman, 경계 보너스)와 같은 방식이고, VS Code처럼 낱말이 파일 이름에서 맞으면 경로에서만 맞는
  것보다 언제나 위다(이름 전체 > 확장자 뺀 이름 > 이름 앞부분 > 이름 안 > 경로). 이유: 한글 초성(`ㅍㅇ` → `파일`)과
  IME가 조합 중인 마지막 글자(`팡` → `파ㅇ`, `파` → `팔`)를 글자 비교에 넣어야 하는데 nucleo는 비교를 바꿀 수 없고,
  grapheme의 첫 코드 포인트만 봐서 NFD로 저장된 한글 경로는 초성만 남는다(우리는 NFD 한글을 음절로 합쳐 비교하고
  맞은 자리는 원래 자모 전부를 칠한다. 한글 밖의 NFD는 합치지 않는다). 등급이 점수를 언제나 이기므로 앞 50개에
  들 수 없는 등급의 경로는 점수를 세지 않고, 이어 친 검색어는 앞 결과 안에서만 찾는다(bench/results.md: vscode
  2만 파일 글자마다 8ms 이하, 49만 파일 32ms 이하). 목록은 gix로 읽은 인덱스이고(skip-worktree·서브모듈 제외)
  인덱스 파일 크기·수정 시각이 바뀌면 다시 읽는다.
- 2026-10-15: 에디터로 열기는 에디터를 띄우기만 하고 기다리지 않는다(`plx_core::editor`). 읽기 전용 원칙과 맞게
  작업트리 파일을 여는 것뿐이고 편집은 그 에디터의 몫이다. VS Code 계열은 `--goto 파일:줄:열`, Zed·Sublime은
  `파일:줄:열`, JetBrains는 `--line`·`--column`. vim 같은 터미널 에디터는 GUI 앱이 띄울 터미널이 없어 자동으로
  고르지 않는다(명령 틀로 터미널 에뮬레이터를 감싸면 쓸 수 있다). 과거 리비전의 파일이 작업트리에 없으면 그 뒤의
  rename을 `git log --diff-filter=D --ancestry-path`로 찾은 지점에서만 따라간다(리비전마다 걷지 않는다).
- 2026-10-15: AI 출처(M5)는 git-ai가 커밋마다 붙이는 `refs/notes/ai` 노트(Git AI Standard v3.0.0, `authorship/3.0.0`)를
  읽어 blame에 겹친다(`plx_core::ai_notes`). 노트는 그 커밋 버전의 줄 번호라 `git-ai blame`과 같이 blame 블록의 원래 커밋·
  원래 경로·원래 줄로 찾고, 뒤 항목이 이기며 메타데이터에 없는 키는 건너뛴다(`s_…::t_…` 세션, `h_` 사람, 16자 옛
  프롬프트는 다른 노트에서도 찾는다). 규칙은 git-ai 저장소의 표준 문서와 `get_line_attribution`을 읽어 맞췄다(git-ai를
  빌드·실행하지는 않았다). 노트 트리는 gix로 직접 읽고(fanout 무관, git 스폰 없음) notes 참조가 바뀌면 다시 읽는다.
  해석한 노트는 blob id로 기억해(git-ai는 커밋마다 참조를 옮기지만 그대로인 노트는 blob도 같다) 바뀐 노트만 다시 읽고,
  옛 프롬프트 표도 새 노트만 더한다(노트가 빠지거나 고쳐지면 처음부터 다시 모은다).
  아직 커밋하지 않은 줄의 AI 작업 상태(`.git/ai`)는 git-ai 내부 형식이라 보지 않는다.
- 2026-10-15: 단일 인스턴스는 `tauri-plugin-single-instance` 대신 직접 구현한다(`plx_core::instance`).
  `main()` 맨 앞, Tauri 초기화 전에 로컬 소켓(Windows named pipe, Unix 소켓)으로 argv·cwd·t0를 넘기고 끝낸다.
  primary 판정은 락 파일(`File::try_lock`). 이유: 두 번째 호출이 WebView 초기화를 거치지 않는다,
  터미널에서 처음 실행할 때 셸을 돌려주려면(분리 재실행) Tauri 전에 판정해야 한다, 측정용 t0를 함께 넘길 수 있다.