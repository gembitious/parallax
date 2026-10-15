# 벤치 결과

성능 예산(`docs/MILESTONES.md`)의 baseline은 **앱과 같은 머신**에서 git CLI를 직접 잰 값이다.
그래서 Windows·macOS 개발 머신에서 직접 재서 채운다. Linux 컨테이너 수치는 도구가 동작하는지 보여 주는 참고값이다.

## 재는 법

1. `bench/setup.sh`: 참조 저장소를 받는다(디스크 약 2.5GB, `bench/repos/`). git/git `v2.56.0`, microsoft/vscode `1.142.0`.
   전체 이력 클론 후 고정 태그를 체크아웃하고, commit-graph와 changed-path Bloom 필터를 쓴다.
2. `bench/run.sh [이름] [반복 횟수]`: git baseline을 재서 아래 "git baseline"에 넣는다. 같은 이름이면 덮어쓴다.
3. `npm run tauri build -- --no-bundle` 다음 `bench/app-latency.sh`: 앱 지연을 재서 표를 출력한다. 아래 "앱 지연"에 붙인다.

## 조건과 항목

- **commit-graph 없음:** `-c core.commitGraph=false`. 클론 직후 상태다.
- **Bloom 있음:** commit-graph와 changed-path Bloom 필터를 읽는다(`git commit-graph write --reachable --changed-paths`).
  앱의 "가속 설정"(M2)이 만드는 상태와 같다.
- git은 앱과 같은 방식으로 띄운다: `-c core.quotepath=false`, 표준 출력은 파이프, Windows는 `CREATE_NO_WINDOW`.
- 셀은 `첫 출력 / 첫 N / 전체` (ms, 예열 1회 뒤 5회 중앙값).
  - 첫 출력: 표준 출력의 첫 바이트를 받은 때.
  - 첫 N: blame은 첫 블록(`filename` 줄까지), log는 커밋 50개, 그래프는 커밋 100개를 받은 때. 예산의 "첫 줄", "첫 50개", "첫 화면"과 맞춘다.
  - 전체: git이 끝난 때.
- 측정 항목과 대상 파일은 `bench/cases.txt`, 측정 코드는 `bench/src/main.rs`에 있다.

## git baseline

<!-- bench:linux-container -->
### linux-container (2026-10-15)

- 환경: linux x86_64, Intel(R) Xeon(R) Processor @ 2.10GHz (4코어)
- git version 2.43.0
- 반복: 예열 1회 후 5회 측정, 중앙값(ms)
- `git`: v2.56.0 (a018953688f1), HEAD까지 커밋 82327개
- `vscode`: 1.142.0 (d956e8a6ac5d), HEAD까지 커밋 167522개

| 저장소 | 항목 | 대상 | commit-graph 없음 | Bloom 있음 | 레코드 | 출력 |
|---|---|---|---|---|---|---|
| git | git 스폰 | — | 1.7 / — / 1.8 | 1.8 / — / 1.9 | — | 0 KB |
| git | blame --incremental (첫 블록) | `diff.c` | 17.6 / 17.6 / 1729 | 15.4 / 15.4 / 1158 | 3198 | 560 KB |
| git | blame --incremental (첫 블록) | `builtin/blame.c` | 16.3 / 16.3 / 730 | 11.2 / 11.2 / 362 | 643 | 134 KB |
| git | log -- <path> (첫 50개) | `Makefile` | 3.8 / 27.5 / 953 | 3.5 / 11.6 / 374 | 3276 | 354 KB |
| git | log -- <path> (첫 50개) | `diff.c` | 6.7 / 38.8 / 990 | 3.7 / 14.5 / 280 | 1701 | 187 KB |
| git | log --follow (첫 50개) | `Documentation/git-log.adoc` | 60.7 / 1673 / 2473 | 36.8 / 1110 / 1621 | 120 | 13 KB |
| git | log --follow (첫 50개) | `builtin/blame.c` | 29.6 / 421 / 2241 | 21.6 / 281 / 1440 | 407 | 46 KB |
| git | log -L | `1000,1060:diff.c` | 1469 / — / 2577 | 234 / — / 591 | — | 47 KB |
| git | show | `HEAD` | 3.6 / — / 3.9 | 3.5 / — / 3.9 | — | 0 KB |
| git | log --topo-order (첫 100개) | — | 976 / 977 / 1601 | 3.5 / 7.7 / 1926 | 82327 | 13364 KB |
| vscode | blame --incremental (첫 블록) | `src/vs/editor/common/config/editorOptions.ts` | 26.3 / 26.3 / 3429 | 16.7 / 16.7 / 1208 | 2824 | 680 KB |
| vscode | blame --incremental (첫 블록) | `src/vs/workbench/contrib/terminal/browser/terminalInstance.ts` | 13.4 / 13.4 / 3899 | 15.1 / 15.1 / 1204 | 1546 | 497 KB |
| vscode | log -- <path> (첫 50개) | `package.json` | 2.5 / 45.3 / 2096 | 2.9 / 11.4 / 549 | 5127 | 488 KB |
| vscode | log -- <path> (첫 50개) | `src/vs/workbench/browser/layout.ts` | 110 / 511 / 3242 | 10.1 / 41.0 / 375 | 680 | 75 KB |
| vscode | log --follow (첫 50개) | `src/vs/workbench/contrib/terminal/browser/terminalInstance.ts` | 3.2 / 755 / 5300 | 3.6 / 451 / 3878 | 1760 | 182 KB |
| vscode | log -L | `100,160:src/vs/editor/common/model/textModel.ts` | 2775 / — / 3792 | 272 / — / 633 | — | 52 KB |
| vscode | show | `HEAD` | 3.8 / — / 4.0 | 3.8 / — / 4.2 | — | 10 KB |
| vscode | log --topo-order (첫 100개) | — | 1895 / 1896 / 3275 | 2.8 / 7.4 / 3903 | 167522 | 25925 KB |

셀은 `첫 출력 / 첫 N / 전체` (ms). 첫 N은 항목 이름의 괄호 안 기준이고, 해당 없으면 —.
<!-- /bench:linux-container -->

<!-- bench:baseline-end -->

Windows·macOS: 아직 없다. 위 순서대로 재서 채운다.

## 앱 지연

`bench/app-latency.sh` 결과. 릴리스 빌드에서 잰다. 시작점은 OS가 plx 프로세스를 만든 시각이고, 끝점은 대상 뷰가 그려진 시각이다.
`PLX_CHECK_RECORD=<이름> scripts/check-app.sh`로 재면 여기에 그 이름으로 들어간다(같은 이름은 덮어씀).
CI(GitHub Actions)의 수치는 각 실행의 Summary에 나온다. CI 러너는 개발 머신이 아니므로 예산 판정에는 쓰지 않는다.

<!-- app:linux-container -->
### linux-container (2026-10-15)

Xvfb 소프트웨어 렌더링, 릴리스 빌드. 스크립트를 세 번 돌린 범위다(각 n=20).

| 항목 | 값 (ms) | 예산 |
|---|---|---|
| 콜드 스타트 → 첫 페인트 | 550 ~ 593 | ≤ 1000 |
| 상주 인스턴스에 plx 호출 → 대상 뷰 첫 페인트 (중앙값 / p90 / 최대) | 43~48 / 54~72 / 81~84 | ≤ 100 |
| 두 번째 plx 프로세스, 셸에서 본 시간 (중앙값) | 36 ~ 39 | — |
<!-- /app:linux-container -->

<!-- app:end -->

### CI 러너 (GitHub-hosted, 2026-10-15)

릴리스 빌드, 각 n=20, 콜드 스타트는 1회. 러너는 공유 VM이고(Windows·Linux는 GPU 없음) 개발 머신과 다르므로 예산 판정이
아니라 **변화를 보는 기준**으로 쓴다. M0은 빈 화면(요청 요약), M1-b부터 실제 blame 화면이다(PR #1 실행 3, PR #3 실행 6).

| 항목 | Windows | macOS (arm64) | Linux (Xvfb) | 예산 |
|---|---|---|---|---|
| 콜드 스타트 → 첫 페인트, M0 | 1186 | 905 | — | ≤ 1000 |
| 콜드 스타트 → 첫 페인트, M1-b | 1540 | 2156 | 918 | ≤ 1000 |
| plx 호출 → 첫 페인트 중앙값 / p90, M0 | 14.8 / 17.6 | 21.2 / 98.8 | — | ≤ 100 |
| plx 호출 → 첫 페인트 중앙값 / p90, M1-b | 48.4 / 56.2 | 28.4 / 34.4 | 131.2 / 185.2 | ≤ 100 |
| 두 번째 plx, 셸에서 본 시간 중앙값, M0 | 73.1 | 22.2 | — | — |
| 두 번째 plx, 셸에서 본 시간 중앙값, M1-b | 133.2 | 18.9 | 57.3 | — |
| blame 첫 줄 표시 중앙값 / p90, M1-b | 132.9 / 186.7 | 26.0 / 30.0 | 70.0 / 100.0 | git 첫 출력 + 30 |
| (git blame 첫 출력 중앙값) | 113.9 | 8.7 | 6.4 | — |

## 메모

linux-container는 클라우드 VM(4코어, git 2.43.0)에서 잰 참고값이다. 사용자 머신의 git은 대개 더 새 버전이고
디스크·CPU도 다르므로, 아래 관찰은 Windows·macOS 수치로 다시 확인한다.

- **그래프:** commit-graph가 없으면 `log --topo-order`의 첫 100개가 1~2초 걸린다(전체 이력을 다 걸은 뒤에야 출력).
  commit-graph가 있으면 7ms다. M3의 gix 리비전 워크도 commit-graph(세대 번호)를 써야 한다.
- **라인 히스토리:** `log -L`의 첫 출력이 Bloom 필터 없이 1.5~2.8초, 있으면 0.23~0.27초다. M2 "가속 설정"의 효과가 가장 큰 항목이다.
- **파일 히스토리:** `log --follow`는 Bloom 필터가 있어도 첫 50개까지 0.3~1.1초로, `log -- <path>`(12~45ms)보다 훨씬 느리다.
  "먼저 `log -- <path>`로 첫 페이지를 띄우고 rename 이전은 `--follow`로 이어 붙인다"는 M2 계획을 뒷받침한다.
- **blame:** 첫 블록은 Bloom 필터와 관계없이 11~26ms지만 전체는 0.4~3.9초다. incremental 스트리밍과 커밋 고정 캐시가 필요하다.
- **blame 세션 오버헤드(M1):** 앱의 blame 요청(`plx-blame`: 저장소 찾기·git 실행·파일 읽기·파싱·JSON 직렬화)은
  내용(meta)을 1~2ms에 내고, 첫 묶음은 git의 첫 블록과 측정 잡음 안에서 같다(−4~+0.5ms, 2026-10-15 linux-container,
  위 네 파일). 전체 시간도 git과 같다. 따라서 예산(첫 출력 + 30ms)의 나머지는 IPC와 화면 그리기 몫이다.
  화면으로 보내는 JSON은 git 출력의 1.6~1.9배(`diff.c` 1.0MB)다. 블록마다 경로·`previous`가 반복되기 때문이라
  M1-b에서 IPC 시간을 재 보고 예산을 넘으면 줄인다.
- **두 번째 plx의 비용:** Linux에서 plx 호출 → 첫 페인트의 대부분(약 35ms)은 두 번째 plx 프로세스가 뜨는 시간이다.
  앱 실행 파일이 GTK·WebKit 공유 라이브러리 129개를 로드하기 때문이다(`plx --version`만 해도 약 30ms).
  GUI 의존성 없이 `plx_core::instance`로 전달만 하는 작은 바이너리(431KB)를 실험으로 만들어 보니 4.5ms였고,
  plx 호출 → 첫 페인트는 11~25ms였다. Windows·macOS에서 앱 실행 파일 시작 비용을 잰 뒤 예산을 넘으면
  전달 전용 `plx` 바이너리를 따로 두는 구조를 검토한다(전달 로직이 이미 `plx-core`에 있어 옮기기 쉽다).
- **CI 러너에서 본 것(M1-b):**
  - Windows의 blame은 git 자체가 첫 출력까지 114ms다(프로세스 생성·git 시작). 우리 오버헤드는 19ms로 예산 안이지만
    절대 시간이 크다. 다시 여는 파일은 캐시(M1-c)가 git을 건너뛴다. 처음 여는 파일까지 줄이려면 gix-blame
    (골든 테스트 통과 전에는 실험 플래그) 또는 git 프로세스를 미리 띄워 두는 방법을 검토한다.
  - Windows에서 두 번째 plx가 셸을 붙잡는 시간이 M0 73ms → M1-b 133ms로 늘었다. 실행 파일이 커진(gix) 만큼
    로드·종료가 느려진 것으로 보인다(백신 검사 포함). 상주 인스턴스가 그리는 시점(48ms)과 무관하게 터미널이
    늦게 돌아온다. 전달만 하는 작은 plx 실행 파일을 따로 두는 구조를 우선 검토한다(Linux 실험에서 4.5ms).
  - 콜드 스타트가 Windows 1.5s, macOS 2.2s로 예산을 넘는다. M0보다 늘어난 몫(메인 JS 번들이 React만 → React +
    CodeMirror 600kB, 첫 화면이 blame 내용을 기다림)을 구간별로 잰 뒤 줄인다(app-latency.sh가 구간을 남긴다).
- **M2 확인 사항:** changed-path Bloom 필터 v1은 경로에 0x80 이상 바이트가 있으면 murmur3 구현 차이(`char` 부호)로
  해시가 어긋날 수 있어, 최근 git에 v2(`commitGraph.changedPathsVersion`)가 생겼다. 한글 경로가 기본 시나리오이므로
  "가속 설정"이 v2로 쓰는지, gix로 필터를 읽을 때 버전을 맞추는지 확인한다.
