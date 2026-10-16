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
   작업트리 blame만 (HEAD sha, 파일 내용 해시) 키를 쓴다(읽은 내용을 `git blame --contents -`로 준다).
   blame 결과를 바꾸는 설정(`blame.ignoreRevsFile`, `.mailmap`, 줄 끝)도 키에 넣고, 얕은 클론은 캐시하지 않는다.
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
scripts/           설치·확인 스크립트 (macOS shim, 앱 자동 확인 check-app.sh), 오픈 소스 라이선스 고지 생성
.github/workflows/ CI: Windows·macOS·Linux에서 검사(포맷·clippy·테스트)와 앱 확인(릴리스 빌드 + check-app.sh)
src/               React UI
bench/             참조 저장소 준비 스크립트, git 측정 하네스(plx-bench), 앱 지연 측정, 측정 결과(results.md)
fixtures/          테스트 저장소 생성 스크립트 (rename, 이동, merge, CRLF, 한글 경로·내용)
docs/              MILESTONES.md
```

## 반드시 지킬 것
- **인코딩:** 모든 파일은 UTF-8(BOM 없음)로 저장한다. 한글이 들어간 파일을 쓴 뒤에는 다시 읽어 깨지지 않았는지 확인한다.
- **한글 경로:** git CLI는 항상 `-c core.quotepath=false -c i18n.logOutputEncoding=UTF-8`로 호출하고(`plx_core::git::command`),
  지원하는 명령은 `-z` 출력으로 파싱한다.
  git 출력은 바이트로 받아 경계에서만 UTF-8로 디코드한다.
- **윈도우:** 모든 자식 프로세스에 `CREATE_NO_WINDOW`. git 스폰 횟수를 줄이는 선택을 우선한다.
- **플랫폼:** Windows(Git Bash)와 macOS 모두 1급 지원. 한쪽에서만 확인한 기능은 완료로 치지 않는다.
- **읽기 전용:** 저장소를 바꾸는 git 명령은 호출하지 않는다. 유일한 예외는 사용자가 확인한 "가속 설정"(commit-graph 작성).
- **파서 정확성:** git 출력 파서마다 fixtures 기반 골든 테스트(`git blame --porcelain` 등 git 결과와 비교)를 둔다.
- **라이선스:** 코드는 MIT OR Apache-2.0이다(`LICENSE-MIT`, `LICENSE-APACHE`). git처럼 GPL·LGPL인 프로젝트의 코드는
  다른 언어로 옮기지도 않는다: 동작은 문서와 그 도구의 출력(골든 테스트)으로 맞춘다. MIT·Apache 같은 허용 라이선스의
  코드를 옮기면 그 파일 맨 위에 원래 저작권 고지를 둔다(`plx_core::fuzzy`의 fzf). 새 의존성은 이 두 라이선스와 함께
  배포할 수 있는지 본다.

## 명령
- 처음 한 번: `npm install` (Node·npm 버전은 Volta가, Rust 버전은 rustup이 `rust-toolchain.toml`로 맞춘다)
- 개발: `npm run tauri dev`
- 릴리스 빌드: `npm run tauri build` (번들 없이 실행 파일만: `npm run tauri build -- --no-bundle` → `target/release/plx`)
- 프런트 타입 검사 + 빌드: `npm run build` / 프런트 단위 테스트(순수 로직, vitest): `npm test`
- 코어 테스트: `cargo test -p plx-core` / 전체: `cargo test --workspace`
  (fixtures 결정성 테스트가 `fixtures/make-repos.sh`를 실행하므로 bash와 git이 필요하다. Windows는 Git for Windows의 bash, `PLX_BASH`로 지정 가능)
- 테스트 저장소 만들기: `fixtures/make-repos.sh <출력 디렉터리>` → `history`, `encoding` 저장소 (어느 플랫폼에서든 같은 sha)
- 오픈 소스 라이선스 고지: `npm run notices` → `public/third-party-notices.txt`(릴리스 빌드가 먼저 돌려 실행 파일에 넣는다.
  앱에서는 명령 팔레트·도움말·첫 화면의 "오픈 소스 라이선스"로 본다. 개발 빌드에는 없다)
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
- 저장소가 공개라 표준 러너(Linux·Windows·macOS)는 Actions 사용 시간이 들지 않는다. 동시 실행 수에는 한도가 있어
  (macOS 5개) 같은 브랜치의 이전 실행은 자동 취소되고, 문서(`*.md`, `docs/`)만 바뀐 푸시는 CI를 돌리지 않는다.

## CLI 인터페이스 (목표)
```
plx                          현재 저장소 그래프
plx <file>[:<line>]          blame, 해당 줄 선택
plx log <file>               파일 히스토리
plx log -L <a>,<b>:<file>    라인 범위 히스토리
plx show <rev>               커밋 상세
```
`-h`/`--help`, `-V`/`--version`, `--`(뒤 인자를 파일 이름으로: `plx -- log`)도 받는다.
blame은 작업트리에 있는 파일만 받고(컴파일러·grep 형식 `<file>:<line>:<col>`도 받는다, 열은 버린다), `log`는 삭제된
경로도 받는다. 잘못된 인자, 저장소 밖의 대상, 없는 리비전(`plx show`)은 상주 인스턴스에 넘기지 않고 호출한 터미널에
알린 뒤 종료 코드 2로 끝난다(저장소는 앱과 같은 방법으로 열어 보되 저장소 밖의 시스템 설정은 읽지 않는다). 인자도
터미널도 없이 앱으로 띄웠으면(Finder·Dock·
시작 메뉴) 저장소가 아닌 곳이어도 뜨고 첫 화면 안내를 보인다.

macOS는 PATH에 셸 shim(`plx` → 앱 번들 실행 파일)을 둔다: `scripts/install-macos-shim.sh [앱 경로] [설치 디렉터리]`.

인스턴스가 없을 때 릴리스 빌드를 셸이나 다른 도구에서 실행하면(터미널이 아닌 파이프여도: 에이전트의 셸 도구, 에디터의
`:!`, lazygit) 스스로 분리 실행하고, 창을 띄울 준비가 되면 돌아온다. 분리한 앱이 뜨지 못하고 끝나면 그 기록
(인스턴스 디렉터리의 `launch.log`, 상주 앱의 표준 출력·오류)을 보이고 실패로 끝난다. Finder·Dock·`open`(macOS)과
탐색기·시작 메뉴(Windows)로 띄운 앱은 분리하지 않는다. 상주 앱은 처음 부른 디렉터리를 쥐고 있지 않는다(지우거나 볼륨을
꺼낼 수 있다).

환경 변수
- `PLX_DETACH=0|1`: 분리 실행 끄기·강제 (개발 빌드는 기본적으로 분리하지 않는다. `tauri dev`가 자식 프로세스로 잡고 있어야 한다)
- `PLX_T0`: 호출 시작 시각(epoch ms). 없으면 OS가 plx 프로세스를 만든 시각이 지연 측정 기준점이다.
- `PLX_PERF=1`: 릴리스 빌드에서도 성능 HUD를 켠다(개발 빌드는 항상 켜짐. 입력란 밖에서 `` ` `` 키로 보이기·숨기기).
  `PLX_PERF_LOG=<파일>`을 함께 주면 끝난 요청을 JSON 한 줄씩 기록한다(`bench/app-latency.sh`가 쓴다).
- `PLX_TRACE=1`: `plx` 호출이 상주 인스턴스에 넘기기까지의 구간(인자 · 인자 검사 · 저장소 열기 · 인스턴스 자리 · 넘기기,
  ms)을 표준 오류에 쓴다(측정용, `bench/app-latency.sh`가 쓴다).
- `PLX_CACHE_DIR=<디렉터리>`: 결과 캐시(blame·히스토리·커밋 표) 위치. 그 아래 `results/`에 쓴다(기본: OS 캐시 위치의
  `com.gembitious.parallax`, 한도 512MB).
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
- 명령 팔레트: 어느 화면에서든 `:`(입력란 밖이면)·Ctrl+Shift+P(macOS ⌘⇧P). 그 화면에서 할 수 있는 일(키가 없는
  그래프 범위·가속 설정, 성능 HUD 포함)을 이름·초성으로 찾아 `Enter`로 실행하고 옆에 단축키를 보인다.
- 에디터로 열기 `o`: blame은 커서 줄, 커밋 상세는 고른 파일의 첫 바뀐 줄, 히스토리는 그 파일(라인 히스토리면 범위의
  첫 줄), 그래프는 작업트리 폴더. 리비전의 줄은 작업트리 파일의 줄로 옮기고, 그 뒤 이름이 바뀌었으면 새 이름을 연다.
- blame의 `i`: 커서 줄의 커밋 카드를 고정해 보이고 다시 누르면 닫는다(마우스의 거터 클릭과 같다).
- AI 출처: 저장소에 git-ai 노트(`refs/notes/ai`)가 있으면 blame 거터의 AI가 쓴 줄에 보라 줄과 `✦ 도구` 이름이 붙고,
  카드에 에이전트·모델·시킨 사람, 머리줄에 `✦ AI n줄`이 보인다. 따로 켜는 키는 없다.
- 파일 찾기(Quick Open): 어느 화면에서든 `f`(입력란 밖이면)·Ctrl+P(macOS ⌘P). 추적 파일을 퍼지 검색한다
  (파일 이름 일치 먼저, 초성 `ㅍㅇ`, 공백으로 나눈 낱말은 모두 맞아야 한다, `파일:줄`이면 그 줄로). `↑`/`↓`·Ctrl+N/J/K로
  고르고 `Enter`는 작업트리 blame, `Shift+Enter`는 파일 히스토리. 빈 검색어는 최근 연 파일부터 보인다.
  작업트리에서 지운(아직 커밋하지 않은) 파일은 "지움" 표시이고 `Enter`는 HEAD 버전 blame이다.

## 결정 기록
- 2026-10-15: 범위를 읽기 전용 git 뷰어로 한정. 편집·터미널·LSP는 제외.
- 2026-10-15: blame·라인 히스토리는 git CLI로 시작. gix-blame은 골든 테스트 통과 시 전환 검토.
- 2026-10-15: 코드 뷰는 Monaco 대신 CodeMirror 6.
- 2026-10-15: 결과 캐시(blame, 파일 히스토리)는 파일(JSON) 하나에 결과 하나, 이름은 열쇠의 FNV 해시이고 파일 안의 열쇠로
  충돌을 거른다. 작업트리 blame은 읽은 내용을 git에 주고(`--contents -`) 그 내용의 해시를 열쇠로 쓴다(2026-10-16:
  크기·수정 시각 열쇠와 "수정 2초 안의 파일은 넣지 않는다"를 대신한다). 파일 히스토리는 시작 커밋(요청할 때의 HEAD)으로
  고정해 넣고, 커밋 2만 개가 넘는 결과는 넣지 않는다.
- 2026-10-15: 파일 히스토리는 `git log --follow` 대신 `git log -- <경로>` 구간을 잇는다(`plx_core::history`).
  구간 안에서 파일이 생긴 커밋(상태 A)마다 rename 원본을 찾아, 그 커밋의 첫 부모부터 옛 이름으로 다음 구간을 돌린다.
  이유(bench/results.md): `--follow`는 rename 앞 구간에서 Bloom 필터를 거의 못 써서 git/git `Documentation/git-log.adoc`
  첫 50개가 1.1초(Bloom 있음)인데 구간 잇기는 85ms다. `--follow`는 diff가 없는 merge(merge에서 바뀐 버전)를 빼고,
  날짜 순 워크 도중 경로를 바꿔 다른 브랜치 커밋이 빠지거나 낀다. 구간마다 git의 기본 이력 단순화를 그대로 따른다.
  rename 원본은 그 파일 하나만 git과 같은 기준의 유사도(`plx_core::similarity`, 50%)로 찾는다.
  gix의 rename 추적은 짝 수(추가×삭제)가 1,000을 넘으면 유사도 비교를 건너뛰어 파일을 많이 옮긴 커밋을 놓친다.
- 2026-10-15: "가속 설정"은 히스토리 머리줄의 버튼을 눌러 확인한 뒤 `git commit-graph write --reachable --changed-paths`만
  실행한다(`plx_core::commit_graph`). 필터 버전은 git에 맡긴다(2026-10-16 리뷰 수정). 저장소 설정은 바꾸지 않는다:
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
  새 커밋을, 아니면 고른 커밋을 화면의 같은 자리에). 결과 캐시는 커밋 sha·HEAD·파일 내용을 열쇠로 써서
  따로 지울 것이 없다. 화면은 열기를 시작할 때 요청 대상으로 구독하고(다시 열다 실패해도, 예: 체크아웃으로 파일이
  사라져도 감시가 남는다), 구독을 끝낸 뒤 감시를 2초 더 둬 다시 연 화면이 그 변화 때의 지문을 이어받는다(다시 여는
  사이의 변화를 새 지문에 묻지 않게). 그래서 열기 바로 전의 커밋(`git commit && plx log …`)이 화면이 열린 뒤에 알려질
  수 있다. blame·히스토리는 알림의 HEAD가 화면이 읽은 HEAD(blame 메타의 `head`, 히스토리의 시작 커밋)와 같으면 다시 열지
  않는다(2026-10-16, 커밋 직후 `plx log` 10번에 9번 한 번 더 열었다). 라인 히스토리는 다시 열 때 줄 범위를 새
  작업트리의 줄로 옮기고, 다른 화면 아래 숨은 그래프는 다시 보일 때 읽는다. 그래프 이름표가 다른 워크트리의 체크아웃에
  달라지므로 `<공통>/worktrees`도 재귀로 보고(첫 `git worktree add`로 생기면 그때부터), 다른 워크트리의 HEAD를 지문에
  넣어 참조 변화로 알린다.
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
- 2026-10-16: 시작·단일 인스턴스 리뷰 수정. 분리 실행은 터미널이 붙었는지가 아니라 부른 쪽이 기다릴 수 있는지로 정한다:
  macOS는 launchd가 부모일 때(Finder·Dock·`open`), Windows는 표준 핸들이 없을 때(탐색기·시작 메뉴)만 분리하지 않는다.
  이유: 파이프로 부르는 도구(에이전트의 셸 도구, 에디터의 `:!`, lazygit)가 앱이 끝날 때까지 멈췄다. 분리한 부모는 자식이
  Tauri setup을 마칠 때까지(준비 파일, 10초 한도) 기다리고 자식이 실패로 끝나면 그 기록을 보인다. 셸 복귀가 setup만큼
  (Linux 약 0.2초) 늦는 대신 뜨지 못한 까닭이 보인다. 락·소켓 자리는 환경 변수가 아니라 사용자로 정한다(TMPDIR이 다른
  `nix develop`, launchd가 띄운 앱이 서로를 찾지 못해 인스턴스가 둘이 됐다): macOS `confstr(_CS_DARWIN_USER_TEMP_DIR)`,
  Linux `/run/user/<uid>`(이 사용자 전용일 때) 다음 `/tmp`, Windows `SHGetKnownFolderPath(LocalAppData)`. Windows pipe
  이름은 토큰의 사용자 SID와 세션으로 만들고, 이 사용자만 여는 DACL로 만들며, 클라이언트는 `SECURITY_IDENTIFICATION`으로
  열고 서버 프로세스의 사용자를 확인한다. 분리한 자식은 셸이 넘긴 표준 핸들을 물려받지 않는다(Windows는 Rust가 상속
  가능한 핸들을 모두 물려줘 `$(plx …)`가 앱이 끝날 때까지 돌아오지 않았다). 상주 인스턴스는 실행 파일 디렉터리로 작업
  디렉터리를 옮긴다(상대 경로로 준 `PLX_CACHE_DIR`·`PLX_PERF_LOG`는 먼저 절대 경로로). 요청을 보내고 응답을 받기는
  5초 한도다(Windows pipe에는 읽기 시간 제한이 없어 스레드로 기다린다). 터미널에 쓰는 문장의 제어 문자는 바꿔 쓴다.
  넘기기 전에 대상 저장소를 앱과 같은 `Repo::discover`로 열어 보고 실패는 터미널에 알린다(인자도 터미널도 없는 앱 실행은
  예외: 알릴 곳이 없고 작업 디렉터리가 `/`라 뜨지 못하게 된다). 이 확인은 저장소 밖의 시스템·git 설치 설정을 읽지
  않는다(`Repo::discover_for_check`, 2026-10-16 측정 뒤): Windows에서는 시스템 설정의 자리를 gix가 git을 띄워 찾아
  (`EXEPATH`가 없으면) plx 호출마다 30~50ms였다(CI의 PLX_TRACE: 저장소 열기 53.5ms, macOS·Linux 1ms 안). 저장소인지,
  리비전이 있는지는 시스템 설정과 상관없다. 상주 앱은 프로세스마다 한 번이라 그대로 모두 읽는다.
- 2026-10-16: 커밋 그래프는 orrery(gembitious/orrery)의 SVG 렌더링을 재사용하지 않고 캔버스로 그린다. orrery는 git 상태
  전이를 보여 주는 모형이라 커밋·선·이름표마다 SVG 요소를 만들고(커밋 수십 개), 선을 자식 → 부모 곡선 하나로 그려
  보이는 행만 잘라 그릴 수 없으며, 레인을 커밋 시각 순서(시뮬레이션 시계)로 정한다. 우리는 수십만 커밋을 git의 topo
  순서로 보이고, 레인을 행마다 반 칸 선으로 나눠 행 하나만 보고 그린다. 화면 위아래 1,000px(화면 높이까지)만 그리고
  행과 레인 캔버스를 스크롤 내용 안에 둬, 그 안의 스크롤은 브라우저가 옮기기만 한다(bench/results.md: 휠 프레임의
  약 2/3에 JS가 없다). HEAD가 브랜치에 붙었는지(detached면 `HEAD` 이름표)는 orrery처럼 구분해 보인다.
  WebKit(macOS·Linux)에서는 레인 캔버스에 CPU 캔버스(`willReadFrequently`)를 청한다. macOS WebKit은 GPU 캔버스를 처음 쓸
  때 준비하느라 앱이 뜬 뒤 처음 연 그래프가 CI에서 0.9~3.1초였다(그리기에 메인 스레드 0.26~0.38초, 그 뒤 화면까지
  1.2~2.7초). CPU 캔버스로 0.27~0.44초다(첫 그리기 0.18~0.30초는 남는다). 레인은 24개까지만 그려 CPU로 충분하고, Linux의
  git/git 스크롤은 프레임 간격이 오히려 줄었다. Chromium(Windows)은 GPU 캔버스로 둔다: 그린 뒤 화면까지가 GPU 36·38·40·40·97ms,
  CPU 77·111ms였다(bench/results.md).
- 2026-10-16: blame 파이프라인 리뷰 수정. 화면이 준 경로는 저장소 기준 경로 그대로 쓰고(`repo::FilePath::InRepo`),
  터미널에서 준 절대 경로만 실제 경로(심볼릭 링크·대소문자)로 푼다. 이유: 리비전의 경로를 오늘의 파일 시스템으로 풀면
  그 사이 대소문자만 바꾼 rename이나 심볼릭 링크로 바뀐 파일이 다른 파일이 된다. 작업트리 파일을 blob과 비교하거나 줄을
  옮길 때는 gix의 clean 변환(줄 끝·ident, 외부 필터 드라이버는 띄우지 않는다)을 거친다: Git for Windows 기본값
  (`core.autocrlf=true`)의 CRLF 작업트리는 그대로 비교하면 모든 줄이 바뀐 줄이다. gix는 저장소를 열 때 설정을 한 번
  읽으므로 `RepoCache`가 설정 파일의 크기·수정 시각을 보고 바뀌었으면 다시 연다. 인덱스에만 있는 파일(`git mv`, 새로
  add)도 git이 blame한다(옮긴 파일은 원래 이력을 찾는다). blame 출력은 읽기 스레드로 받아 git이 조용해도 받은 블록을
  16ms 안에 넘긴다.
- 2026-10-16: 파일 히스토리 리뷰 수정. 구간은 옛 이름마다 따라가고(한 구간에 옛 이름이 여럿이어도), 같은 이름의 경계는
  모두 한 구간의 시작으로 합친다(두 브랜치가 같은 rename). rename으로 생긴 뒤 같은 이름이 지워진 커밋이 나오면 그 아래는
  그 이름을 전에 쓴 다른 파일이라 구간을 나눠 옛 이름의 이력 뒤에 보인다(A → B → A가 `git log --follow`와 같은 순서·상태).
  merge는 파일이 없는 부모마다 rename 원본을 찾는다(곁가지에서 옮기는 동안 본줄기가 옛 이름을 고친 커밋, git blame도 본다).
  rename 원본의 이름 지름길(75%)은 지운 파일 가운데 같은 이름이 하나뿐일 때만 쓴다(git의 find_basename_matches).
  `plx log .`은 경로 없이 `git log`다(`-- .`은 이력 단순화로 빈 커밋·`-s ours` merge를 뺀다). 이력 질의의 경로는 모두
  `--literal-pathspecs`로 준다(`[slug]` 같은 글롭 글자, Bloom 필터). 얕은 클론과 rename 원본을 찾지 못한 결과(부분 클론)는
  캐시하지 않는다. git 2.54부터 `-L`은 보통 diff 출력기로 나와 공백이 든 경로 뒤에 TAB이 붙으므로 떼고,
  `diff.suppressBlankEmpty`를 끈다.
- 2026-10-16: 커밋 그래프 리뷰 수정. git처럼(commit_graph_compatible) 얕은 클론이거나 `git replace`·`info/grafts`로
  부모를 바꾼 커밋이 있으면 commit-graph와 커밋 표를 쓰지 않고 객체에서 바꾼 부모를 읽는다(다 받은 뒤 얕아진 클론은
  commit-graph가 경계 너머까지 걸었다). gix 0.89는 `core.useReplaceRefs`를 거꾸로 읽어(설정이 없거나 true면 끈다)
  객체를 바꾸지 않으므로 바꾸는 참조를 직접 읽는다. 객체를 읽지 못한 커밋이 있으면 커밋 표를 쓰지 않는다. 태그 객체를
  가리키는 시작점(느슨한 `refs/remotes/origin/tags/*` 등)은 커밋까지 벗기고 이름표도 옮긴다.
- 2026-10-16: 커밋 상세 리뷰 수정. rename은 gix의 추적을 끄고 git diffcore-rename의 단계(내용이 같은 파일 → 이름이
  양쪽에 하나뿐인 짝 75% → 남은 짝 모두를 견줘 새 파일마다 후보 넷, 모두를 비슷한 순으로)를 그대로 밟는다
  (`plx_core::renames`, 유사도는 `plx_core::similarity`). 이유: gix는 짝 수를 한도 1,000과 견줘(git은 1,000²)
  새 파일이 수백 개인 커밋의 rename을 추가 + 삭제로 보였고, 짝마다 줄 diff를 해 느리다. 짝마다 두 파일을 견주지 않고
  원본 조각의 해시 색인으로 새 파일마다 함께 가진 조각만 센다(값은 짝마다 견준 것과 같다. bench/results.md: 990 × 990 이동에서
  git 0.88초, 앱 0.14초). blame 카드의 바뀐 파일 수는 한도 100²로 센다(카드는 바로 떠야 한다). 줄 수는 25ms 안에 센
  만큼만 상세에 넣고 나머지는 화면이 묶음으로 받는다(`file_stats`, 화면 갱신 20번 안팎). 바이너리는 앞 8000바이트의
  NUL과 `.gitattributes`의 `binary`·`-diff`로 정한다(git numstat의 `-`). 두 쪽 합이 4MB를 넘는 파일은 크기만 보내고
  "그래도 보기"로 다시 받는다. 줄 diff에 보이지 않는 바뀜(모드, 줄 끝, 끝 개행, BOM, 인코딩)은 diff 위에 알린다.
  부분 클론에서 받지 않은 blob은 받아 오지 않고(저장소를 바꾸는 일이다) 그 파일만 "받지 않음"으로 보이며, 그래서
  rename을 다 견주지 못했으면 알린다. UTF-8이 아닌 경로는 깨진 글자로 보이고 blame·에디터로 열지 않는다(그 경로로는
  파일을 찾을 수 없다). 목록은 보이는 행만 그리고, 두 버전은 한 번에 하나만 받아(j를 누르고 있어도 쌓이지 않는다)
  48개·3,200만 글자까지만 기억한다.
- 2026-10-16: 저장소를 공개하고 라이선스는 MIT OR Apache-2.0(Rust 생태계의 기본)으로 정한다. 저작권자는 GitHub 계정
  이름(gembitious)이다. 이유: 비공개 저장소는 Actions 사용 시간 한도에 걸려 CI가 돌지 않았다(공개 저장소의 표준 러너는
  무료다). rename 유사도의 조각 해시는 git의 `diffcore-delta.c`를 옮긴 것이라 GPL-2.0-only의 파생물이었다. 이 라이선스로
  낼 수 없고, Tauri의 `tao`와 `gix-imara-diff`·`unicode-bom`이 Apache-2.0 전용이라 GPL-2.0으로도 함께 배포할 수 없어
  (FSF와 Apache 재단의 입장) 해시를 따로 정한 것(FNV-1a 64비트)으로 바꿨다. 조각 나누기(줄, 64바이트)·기준(50%)·단계는
  그대로라 짝은 git과 같다(골든 테스트, git/git·vscode의 큰 rename 커밋). git은 해시를 107,927칸으로 모아 다른 조각이 같은
  칸이면 함께 가진 것으로 세므로 유사도가 조금 높고(300KB 파일에서 1%p 안쪽, 파일이 클수록 커진다), 기준이나 다른 후보와
  그만큼 붙은 짝만 다를 수 있다. 속도는 측정 잡음 안이다(bench/results.md). 옛 커밋과 PR 기록에는 옮긴 코드가 남는다
  (PR은 지울 수 없다). fzf(MIT)를 따른 퍼지 점수는 fzf의 고지를 `plx_core::fuzzy` 맨 위에 둔다. 바이너리를 배포하기 전에
  의존성의 저작권 고지를 앱에 묶고 Tauri 기본 아이콘을 바꾼다(docs/MILESTONES.md "리스크와 대응").
- 2026-10-16: 바이너리에 넣는 제3자 저작권 고지는 `scripts/third-party-notices.mjs`가 릴리스 빌드마다 만든다(`tauri build`의
  beforeBuildCommand. cargo-about 같은 도구를 더하지 않는다: Node와 `cargo metadata`로 충분하다). 실행 파일에 링크되는
  크레이트(Windows·macOS·Linux × x86_64·aarch64의 합, 빌드·테스트 의존성 제외)와 dev가 아닌 npm 패키지마다 패키지에 든
  라이선스·NOTICE 파일을 그대로 싣고, 같은 문구는 한 번만 싣는다. 파일이 없는 패키지는 같은 저장소·같은 라이선스 표기인
  다른 패키지의 파일, 아니면 선언한 라이선스로 채운다(MIT는 작성자로 저작권 줄을 만들고, Apache-2.0·MPL-2.0처럼 저작권
  줄이 없는 것은 표준 문구). 그래도 못 채우면 빌드를 실패시킨다. 결과(크레이트 402개·npm 57개, 문구 223개, 732KB)는
  Vite가 dist로 옮겨 실행 파일에 묻히고, 앱의 "오픈 소스 라이선스" 창이 읽는다. 커밋하지 않는다(의존성을 바꿀 때 따로
  갱신할 것이 없다). 창은 App이 그리고 열려 있는 동안 키를 모두 먹는다(화면들은 window에서 키를 받는다).
- 2026-10-16: 가속 설정·결과 캐시 리뷰 수정. 필터 v2를 강제하지 않는다(`-c commitGraph.changedPathsVersion=2`를 뺐다).
  git 기본값대로 그래프에 있는 버전, 없으면 v1이다. 이유: git 2.39~2.45는 v2 그래프를 다시 쓰다(gc, fetch.writeCommitGraph)
  SIGSEGV로 죽는다. Finder로 띄운 macOS 앱의 `/usr/bin/git`(Apple Git 2.39), Ubuntu 24.04(2.43)처럼 흔한 git이고 앱이
  쓰는 git도 띄운 방법에 따라 다르다. v1의 한글 경로 해시는 `char`가 부호 있는 플랫폼(x86-64, Windows·macOS의 ARM64)끼리
  같고, Linux ARM64와 .git을 나눠 쓸 때만 어긋난다(확인 창에 알린다). 그래프에 v2가 있는데 git이 2.46보다 낮으면 쓰지 않는다.
  상태는 git이 보는 대로 본다: 객체 디렉터리마다(저장소, alternates) 단일 파일을 사슬보다 먼저, 층마다 BDAT 머리(버전,
  해시 수, 비트)가 맨 위 층과 같아야 필터로 친다. git이 commit-graph를 쓰지 않는 저장소(얕은 클론, grafts, `git replace`,
  `core.commitGraph=false`)와 필터를 읽지 않는 설정, 트리를 받지 않은 부분 클론(`--filter=tree:0`: 필터를 만들려면 커밋마다
  트리를 받아 온다)은 까닭을 보이고 권하지 않는다. 쓰기는 `GIT_NO_LAZY_FETCH=1`로 하고, git이 성공으로 끝나도 필터가 없거나
  파일이 그대로면(Windows에서 다른 프로그램이 연 파일) 실패로 알린다. 한 저장소에는 한 번에 하나만 쓰고(화면 둘에서
  눌러도), 상태는 쓰기 시작·끝과 창으로 돌아올 때 다시 읽는다. HEAD가 그래프에 없어도(새 커밋, 브랜치에서 닿지 않는
  detached HEAD) 가속됨으로 본다(나머지는 필터를 쓴다. 다시 쓰기는 팔레트로). 결과 캐시: 파일·라인 히스토리 열쇠에 mailmap
  (`%aN`)과 `-L`의 diff·rename 설정을 넣고, `git replace`·grafts가 있으면 얕은 클론처럼 넣지 않는다. 커밋 표(`.bin`)는 값의
  FNV 해시를 머리에 둬 끝이 0으로 채워진 파일을 버리고, 쓰기에 실패하면 임시 파일을 지우며, 한도 정리는 따로 스레드에서 한다.
- 2026-10-16: 화면 공통 동작 리뷰 수정. 작업트리 blame·HEAD부터의 히스토리를 다시 열 때(화면을 새로 만든다) 보이는
  화면에 창(도움말·워크트리·Quick Open·팔레트·고정 카드·가속 설정 확인)이나 검색 칸이 열려 있으면 닫을 때까지 미루고(머리줄에
  `바뀜`), 검색어는 새 화면으로 넘긴다(`n`/`N`이 이어서 찾는다). 이유: 에이전트가 파일을 저장할 때마다 그 창과 치던
  글자가 사라지고 남은 글자가 화면 키가 됐다(`o`가 에디터를 띄우고 `q`가 창을 숨겼다). 그래프처럼 제자리에서 다시
  읽으려면 blame의 에디터·거터·AI 출처를 새로 짜야 해 미루기를 골랐다. 숨은 화면은 그대로 바로 다시 연다. 라이선스
  창이 떠 있는 동안의 다시 열기도 닫을 때까지 미루고, 그사이 늦게 온 이동은 버린다. 늦게 온 `<`·`>`·`,`의 답은
  그사이 창을 열었으면 버리고, 화면이 숨을 때는 그 화면의 창을 닫는다(숨은 화면에 남은 창은 돌아왔을 때 포커스 없이
  떠 있었다).
- 2026-10-16: 콜드 스타트 측정(bench/results.md, CI 세 플랫폼과 linux-container). WebView를 만드는 구간이 가장 크다(Windows
  0.43초, macOS CI VM 2.1초, Linux 0.25초)라 앱이 줄일 수 있는 것부터 고쳤다. blame은 git을 따로 스레드에서 띄우고 내용을
  먼저 보낸다(전에는 git 프로세스를 만든 뒤 내용을 보냈다: CI의 macOS에서 첫 git 실행이 0.9초, Windows 60ms). 상주 앱은
  뜰 때 WebView를 만드는 동안 git을 한 번 띄워 둔다(`plx_core::git::warm_up`, 그 버전을 기억해 가속 설정이 쓴다).
  JS·첫 렌더 구간(macOS CI 0.53초)은 나눠 재 보니(linux-container) 묶음을 읽고 실행하는 것은 절반 남짓이고 빈 첫 화면 렌더와
  차례로 도는 시작 IPC 셋(그중 요청 듣기 37ms)이 나머지였다. 그래서 시작 IPC를 JS 묶음 맨 앞(`src/boot.ts`)에서 함께 보내
  모듈을 실행하는 동안 오가게 하고, 첫 렌더는 답을 받은 뒤 요청한 화면으로 바로 한다. 듣기 전에 온 요청은 듣기 시작한 뒤
  current_request를 한 번 더 물어 받는다. 화면별로 나눠 불러오기는 blame 화면이 CodeMirror 대부분을 써서 덜 수 있는 것이
  첫 묶음의 12% 남짓이라 하지 않았다.
