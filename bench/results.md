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
| git | git 스폰 | — | 1.5 / — / 1.6 | 1.5 / — / 1.6 | — | 0 KB |
| git | blame --incremental (첫 블록) | `diff.c` | 17.1 / 17.1 / 1607 | 14.2 / 14.2 / 1101 | 3198 | 560 KB |
| git | blame --incremental (첫 블록) | `builtin/blame.c` | 17.0 / 17.0 / 743 | 10.1 / 10.1 / 317 | 643 | 134 KB |
| git | plx blame 세션 (첫 묶음) | `diff.c` | 2.6 / 18.2 / 1673 | 2.6 / 16.1 / 1099 | 3198 | 1069 KB |
| git | plx blame 세션 (첫 묶음) | `builtin/blame.c` | 2.0 / 17.4 / 736 | 1.8 / 12.3 / 330 | 643 | 234 KB |
| git | log -- <path> (첫 50개) | `Makefile` | 4.5 / 31.3 / 911 | 3.4 / 12.6 / 368 | 3276 | 354 KB |
| git | log -- <path> (첫 50개) | `diff.c` | 6.8 / 39.2 / 1006 | 4.3 / 16.1 / 286 | 1701 | 187 KB |
| git | log --follow (첫 50개) | `Documentation/git-log.adoc` | 54.1 / 1643 / 2345 | 35.4 / 1081 / 1499 | 120 | 13 KB |
| git | log --follow (첫 50개) | `builtin/blame.c` | 30.8 / 394 / 2171 | 26.1 / 270 / 1433 | 407 | 46 KB |
| git | plx 파일 히스토리 (첫 50개) | `Makefile` | 1.8 / 30.9 / 968 | 1.5 / 14.5 / 410 | 3276 | 1056 KB |
| git | plx 파일 히스토리 (첫 50개) | `Documentation/git-log.adoc` | 1.5 / 682 / 707 | 1.5 / 88.4 / 181 | 140 | 47 KB |
| git | plx 파일 히스토리 (첫 50개) | `builtin/blame.c` | 1.6 / 97.3 / 886 | 1.4 / 30.1 / 214 | 576 | 190 KB |
| git | log -L | `1000,1060:diff.c` | 1437 / — / 2541 | 229 / — / 551 | — | 47 KB |
| git | show | `HEAD` | 3.0 / — / 3.2 | 2.9 / — / 3.2 | — | 0 KB |
| git | plx 커밋 상세 | `HEAD` | 1.8 / — / 1.8 | 1.9 / — / 1.9 | — | 0 KB |
| git | log --topo-order (첫 100개) | — | 948 / 948 / 1568 | 2.7 / 7.2 / 1870 | 82327 | 13364 KB |
| vscode | blame --incremental (첫 블록) | `src/vs/editor/common/config/editorOptions.ts` | 20.1 / 20.1 / 3454 | 18.3 / 18.3 / 1145 | 2824 | 680 KB |
| vscode | blame --incremental (첫 블록) | `src/vs/workbench/contrib/terminal/browser/terminalInstance.ts` | 14.6 / 14.6 / 4071 | 16.4 / 16.4 / 1205 | 1546 | 497 KB |
| vscode | plx blame 세션 (첫 묶음) | `src/vs/editor/common/config/editorOptions.ts` | 4.8 / 23.7 / 3351 | 4.8 / 20.6 / 1101 | 2824 | 1180 KB |
| vscode | plx blame 세션 (첫 묶음) | `src/vs/workbench/contrib/terminal/browser/terminalInstance.ts` | 4.7 / 17.6 / 4004 | 4.5 / 18.0 / 1213 | 1546 | 781 KB |
| vscode | log -- <path> (첫 50개) | `package.json` | 2.7 / 50.7 / 2142 | 2.9 / 14.5 / 561 | 5127 | 488 KB |
| vscode | log -- <path> (첫 50개) | `src/vs/workbench/browser/layout.ts` | 108 / 490 / 3184 | 13.9 / 45.5 / 346 | 680 | 75 KB |
| vscode | log --follow (첫 50개) | `src/vs/workbench/contrib/terminal/browser/terminalInstance.ts` | 3.1 / 726 / 5201 | 3.6 / 452 / 3885 | 1760 | 182 KB |
| vscode | plx 파일 히스토리 (첫 50개) | `package.json` | 3.9 / 52.4 / 2207 | 3.9 / 18.2 / 613 | 5127 | 1590 KB |
| vscode | plx 파일 히스토리 (첫 50개) | `src/vs/workbench/contrib/terminal/browser/terminalInstance.ts` | 3.8 / 598 / 4356 | 3.9 / 50.2 / 663 | 1898 | 710 KB |
| vscode | log -L | `100,160:src/vs/editor/common/model/textModel.ts` | 2752 / — / 3790 | 274 / — / 645 | — | 52 KB |
| vscode | show | `HEAD` | 5.3 / — / 5.9 | 4.2 / — / 4.6 | — | 10 KB |
| vscode | plx 커밋 상세 | `HEAD` | 8.2 / — / 8.2 | 7.3 / — / 7.3 | — | 1 KB |
| vscode | log --topo-order (첫 100개) | — | 1948 / 1948 / 3148 | 2.8 / 9.5 / 3977 | 167522 | 25925 KB |

셀은 `첫 출력 / 첫 N / 전체` (ms). 첫 N은 항목 이름의 괄호 안 기준이고, 해당 없으면 —.
`plx blame 세션`의 첫 출력은 내용(meta), 레코드는 블록 수, 출력은 화면으로 보내는 JSON 양이다.
`plx 파일 히스토리`의 첫 출력은 meta, 첫 N은 50개째를 보낸 때, 레코드는 커밋 수(rename 앞 구간 포함)다.
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
| 콜드 스타트 → 첫 페인트, M2-a | 820 | 1206 | 2246 | ≤ 1000 |
| plx 호출 → 첫 페인트 중앙값 / p90, M2-a | 39.4 / 48.5 | 30.1 / 36.4 | 211.5 / 373.5 | ≤ 100 |
| 두 번째 plx, 셸에서 본 시간 중앙값, M2-a | 93.0 | 17.0 | 107.2 | — |
| blame 첫 줄 표시 중앙값 / p90, M2-a | 94.2 / 134.4 | 22.0 / 29.0 | 132.0 / 387.0 | git 첫 출력 + 30 |
| (git blame 첫 출력 중앙값, M2-a) | 77.0 | 8.9 | 13.4 | — |
| blame 캐시 히트 중앙값 / p90, M2-a | 25.7 / 32.3 | 16.0 / 21.0 | 126.0 / 163.0 | ≤ 16 |
| 커밋 상세(목록 + 첫 diff) 중앙값 / p90, M2-a | 73.8 / 138.6 | 31.0 / 47.0 | 248.0 / 578.0 | ≤ 50 |

M2-a는 PR #5 실행 38793678019(커밋 상세가 들어간 빌드)다. Linux 러너는 이 실행에서 유독 느렸다(소프트웨어 렌더링,
plx 호출 최대 2.7초): 변화를 볼 때 Linux 열은 빼고 본다.

## 메모

linux-container는 클라우드 VM(4코어, git 2.43.0)에서 잰 참고값이다. 사용자 머신의 git은 대개 더 새 버전이고
디스크·CPU도 다르므로, 아래 관찰은 Windows·macOS 수치로 다시 확인한다.

- **그래프:** commit-graph가 없으면 `log --topo-order`의 첫 100개가 1~2초 걸린다(전체 이력을 다 걸은 뒤에야 출력).
  commit-graph가 있으면 7ms다. M3의 gix 리비전 워크도 commit-graph(세대 번호)를 써야 한다.
- **커밋 그래프 구현(M3-a):** 2026-10-15 linux-container, 3회 중앙값. 셀은 첫 화면(100행) / 레이아웃 끝(ms).

  | 저장소, 범위 | `git log --topo-order` 첫 100개 | commit-graph | 없음(처음) | 없음(커밋 표) |
  |---|---|---|---|---|
  | git/git, HEAD (82,327개) | 7.3 (없으면 966) | 20.5 / 65 | 852 / 908 | 12.8 / 55 |
  | vscode, HEAD (167,522개) | 9.9 (없으면 1,891) | 50.4 / 110 | 1,766 / 1,837 | 41.9 / 102 |

  - gix의 topo 워크(`gix::traverse::commit::topo`)를 먼저 썼다. 순서는 git과 같았지만(동시각 시작점의 순서만 반대)
    세대 번호 v1(위상 깊이)만 읽어서, 오래된 브랜치가 시작점에 있으면 첫 커밋을 내기 전에 이력 대부분의 indegree를
    센다. vscode 원격 브랜치 5,559개를 다 시작점으로 주면 첫 100개가 0.3~0.45초, git/git(`todo` 브랜치의 따로 시작한
    이력)도 75ms였다. git은 세대 번호 v2(보정한 커밋 시각)로 피하지만 gix-commitgraph는 v2를 읽지 않는다.
  - 그래서 시작점에서 닿는 DAG 전체를 commit-graph의 위치 번호로 읽고(객체를 풀지 않는다) git의
    `sort_in_topological_order`와 같은 LIFO 정렬을 한다. 같은 조건에서 git/git 17ms, vscode 45~50ms다.
    순서는 4가지(git/git·vscode × 브랜치·원격 모두) 모두 `git log --topo-order`와 sha까지 같다.
  - commit-graph가 없으면 세대 번호가 없어 첫 커밋 순서를 정하려면 이력 전체를 풀어야 한다(git도 같다).
    한 번 푼 (sha, 부모, 시각)을 앱 캐시의 커밋 표(git/git 3.0MB, vscode 6.1MB)로 두고 다음부터 번호로 따라간다.
  - 레인: 같은 커밋을 기다리는 레인은 바로 왼쪽 레인에 합친다(git의 graph.c와 같다). 합치기 전에는 오래된 커밋에서
    갈라진 주제 브랜치마다 레인이 열려 git/git 너비가 280이었다. 합친 뒤 열린 레인 수는 `git log --graph`의 열 수와
    같다(git/git 중앙값 64, p99 140). 빈 레인은 당기지 않으므로(선이 옆으로 밀리지 않는다) 가장 오른쪽 레인 번호는
    중앙값 116이다. 화면은 너비를 잘라 그린다. 최근 이력(첫 100행)은 git/git 13, vscode 7레인이다.
- **M1~M3 리뷰 수정(2026-10-16):** linux-container, 3회 중앙값, 앞 표와 같은 조건에서 다시 쟀다. 바뀐 것은 측정 잡음 안이다.
  - 작업트리 blame은 파일을 먼저 읽어 그 내용을 git에 준다(`--contents -`, 캐시 열쇠가 내용이다): 첫 묶음 `diff.c`
    17.4ms(git 첫 블록 14.3), `builtin/blame.c` 13.1ms(Bloom 있음).
  - 파일 히스토리는 merge의 다른 부모에서도 옛 이름을 찾는다(merge마다 부모 트리 조회): 첫 50개 `git-log.adoc` 101ms,
    `builtin/blame.c` 26ms, `terminalInstance.ts` 57ms. 곁가지에서 옛 이름을 고친 커밋이 더해져 576 → 578,
    1,898 → 1,919개다.
  - 그래프는 태그 객체를 가리키는 시작점만 벗긴다(commit-graph·커밋 표에 있는 시작점은 위치 조회 한 번 그대로):
    vscode 원격 모두(5,596개) 첫 화면 commit-graph 57.5ms, 커밋 표 47ms, 처음 2.0초.
- **커밋 상세 리뷰 수정(2026-10-16):** linux-container, 5회 중앙값. 셀은 git은 `show --stat --patch`의 첫 출력, 앱은
  상세(파일 목록 + 25ms 안에 센 줄 수)를 낸 때(ms). 앱은 줄 수를 25ms까지만 세고 나머지는 화면이 나눠 받는다.

  | 커밋 | git `show` | 앱 |
  |---|---|---|
  | git/git `1f010d6bdf` (.txt → .adoc, 파일 919개, rename 906개) | 107 | 26 |
  | vscode `3296e952c88` (추가 127개, 삭제 114개) | 23.8 | 22.6 |
  | 만든 저장소: 이름이 다른 파일 990개를 줄 1/3씩 고쳐 옮김(rename 990개, 짝 98만) | 877 (packed 762) | 136 (packed 59) |

  - git의 rename 단계(diffcore-rename)를 그대로 밟는다. 셋 다 짝은 `git diff-tree -M`과 같다.
  - 990 × 990은 처음에 짝마다 두 파일의 조각 목록을 끝까지 걸어 git과 같은 882ms였다. 새 파일을 4스레드로 나누면
    315ms, 원본 조각의 해시 색인으로 새 파일마다 함께 가진 조각만 세면 한 스레드로 136ms다(스레드는 뺐다).
    남은 시간의 대부분(93ms)은 낱개 객체(loose) blob 1,980개를 푸는 데 쓴다. `git gc` 뒤에는 59ms다.
- **그래프 스크롤(M3):** 2026-10-16 linux-container, Xvfb(소프트웨어 렌더링, GPU 없음), 릴리스 빌드에 `PLX_PERF=1`.
  휠 300번(8ms 간격, 약 1,075행)과 PageDown 60번(약 650행). 셀은 스크롤 이벤트 → 그 위치를 그림(JS, 중앙값 / p95) /
  프레임 간격(중앙값 / p95), ms. 성능 HUD의 `그래프 … 스크롤` 줄과 같은 값이다(`src/scrollMeter.ts`).

  | 저장소, 스크롤 | 처음(스크롤마다 다시 그림) | 미리 그린 범위 안에서는 그리지 않음 |
  |---|---|---|
  | git/git (82,327개), 휠 | 20 / 41 · 45 / 108 | 0 / 23~26 · 23~25 / 69 |
  | git/git, PageDown | 15 / 21 · 28 / 34 | 11 / 25~27 · 26~33 / 46~48 |
  | vscode (167,522개), 휠 | | 0 / 17~21 · 22~24 / 56~62 |
  | vscode, PageDown | | 10 / 21~22 · 23~24 / 38~43 |
  | 파일 히스토리 git/git `Makefile` (3,276개), 휠 | 7~8 / 16 · 31~34 / 44~45 | 0 / 6 · 17~18 / 38~41 |
  | 파일 히스토리 `Makefile`, PageDown | 6~7 / 10~11 · 16 / 29~30 | 5 / 12 · 17~19 / 32~33 |

  - 처음에는 스크롤 이벤트마다 그래프 화면 전체를 다시 렌더하고 보이는 행의 레인을 캔버스에 다시 그렸다.
  - 이제 화면 위아래로 1,000px(화면 높이까지)를 미리 그리고, 행과 레인 캔버스를 스크롤 내용 안에 둔다. 그 범위 안의
    스크롤은 JS 없이 브라우저가 옮기기만 하고(휠 프레임의 약 2/3), 범위를 벗어날 때만 그 자리를 가운데로 다시 그린다.
    다시 그리는 프레임은 행 렌더·배치가 15~20ms, 캔버스가 3~7ms다.
  - 미리 그리는 높이를 400px로 줄이면 다시 그리는 프레임이 늘어 더 나빴다(p95 32ms). 행에 `contain: strict`를 주어도
    차이가 없어 넣지 않았다.
  - 파일 히스토리 목록도 같은 방식이다(`src/scrollWindow.ts`). 행이 가벼워 휠 스크롤의 프레임 간격 중앙값이
    17~18ms(60fps 가까이)가 됐고 빠진 프레임이 64 → 22~28개(140프레임 중)로 줄었다.
  - 그래프는 JS가 없는 프레임도 간격이 22~25ms다. 행(이름표)과 레인 캔버스를 Xvfb가 소프트웨어로 칠하는 시간이
    바닥이라 여기서는 60fps(16.7ms)를 잴 수 없다. GPU 합성이 있는 Windows·macOS에서 HUD로 다시 잰다
    (`docs/MANUAL_CHECKS.md` 8번).
  - 행이 비어 보인 프레임은 없었다(앞뒤 60행을 미리 읽는다).
- **Quick Open(M4-c):** 2026-10-16 linux-container, 15회 중앙값(`plx-files`). 셀은 처음 검색(인덱스 읽기 포함) /
  목록이 있을 때 전체 검색 / 한 글자씩 칠 때 가장 느린 글자(ms). 결과 100개. 같은 측정을 되풀이하면 ±1ms쯤 흔들린다.

  | 저장소, 검색어 | 처음 | 목록 있음 | 치는 중 최악 | 맞은 파일 |
  |---|---|---|---|---|
  | git/git (4,854개), `t` | 2.2 | 1.2 | 1.1 | 4,550 |
  | git/git, `blame` | 1.6 | 0.5 | 0.7 | 37 |
  | vscode (20,355개), `t` | 8.3 | 2.1 | 2.0 | 20,169 |
  | vscode, `editoroptions` | 9.0 | 2.4 | 2.4 | 331 |
  | vscode, `src/vs/editor` | 10.8 | 4.2 | 7.9 | 3,608 |

  - 처음 잰 "목록 있음"(git/git `blame` 0.1ms, vscode `editoroptions` 1.1ms)은 빈 검색어가 앞 결과를 지우지 않아
    같은 검색어를 앞 결과 안에서 다시 찾은 값이었다. 빈 검색어가 앞 결과를 지우게 고치고 다시 쟀다.
  - 결과마다 작업트리 파일이 있는지 본다(지운 파일 표시). stat 100번이 0.2ms라(Linux) 흔들림 안이다.
    Windows의 stat은 더 비싸므로 그쪽 측정에서 다시 본다(앱은 결과 50개).

  - 처음 만든 것은 맞은 경로마다 동적 계획법 점수를 셌다. `edit`처럼 짧은 검색어는 1만 개가 맞아 글자마다 7~16ms였다.
    파일 이름 등급이 점수를 언제나 이기므로(등급 무게 2^20 > 낱말 16개의 점수 합) 등급만 먼저 가리고 앞 N개에 들 수
    있는 등급의 경로만 점수를 세자 1~2ms가 됐다. 경로 조각(`src/vs/editor`)은 모두 경로 등급이라 다 세야 해서,
    글자마다 놓일 수 있는 열(앞·뒤에서 욕심껏 맞춘 자리 사이)만 보는 두 줄 계산과 스레드 나누기로 25 → 7ms로 줄였다.
  - 49만 파일(vscode 경로를 24번 겹친 합성 목록)에서 한 글자씩 칠 때 최악 32ms(`ed`), 경로 조각 전체 검색 72ms다.
    새 글자가 오면 하던 검색은 그만두므로 입력이 밀리지 않는다.
  - 앱(Xvfb, 소프트웨어 렌더링)에서 글자 하나 → 결과 페인트는 30~80ms였고 대부분이 페인트다(검색 IPC 5~15ms,
    React 4~7ms). 결과를 100 → 50개로 줄였다. GPU가 있는 Windows·macOS에서 다시 잰다.
- **라인 히스토리:** `log -L`의 첫 출력이 Bloom 필터 없이 1.5~2.8초, 있으면 0.23~0.27초다. M2 "가속 설정"의 효과가 가장 큰 항목이다.
- **파일 히스토리:** `log --follow`는 Bloom 필터가 있어도 첫 50개까지 0.3~1.1초로, `log -- <path>`(12~45ms)보다 훨씬 느리다.
- **파일 히스토리 구현(M2-b):** rename 앞도 `--follow` 대신 옛 이름의 `log -- <path>` 구간으로 잇기로 했다(`plx-log`).
  첫 50개(Bloom 있음)가 `Documentation/git-log.adoc` 1081 → 88ms, `builtin/blame.c` 270 → 30ms,
  `terminalInstance.ts` 452 → 50ms다. commit-graph가 없어도 1643 → 682, 394 → 97, 726 → 598ms다.
  rename이 없는 파일은 git의 첫 50개보다 2~4ms 늦다(`Makefile` 12.6 → 14.5, `package.json` 14.5 → 18.2).
  - 커밋 수가 다르다(120 → 140, 407 → 576, 1,760 → 1,898). `--follow`는 diff가 없는 merge(양쪽 브랜치가 바꾼 것을
    합친 merge)를 빼고, 구간 잇기는 구간마다 git의 기본 이력 단순화를 따른다(결과에 기여하지 않은 곁가지 커밋은 빠진다).
  - `git-log.adoc`의 첫 50개는 앞 구간(5개)이 저장소 끝까지 걷기를 기다린다(Bloom 있음 87ms, 없음 0.7초).
    다음 구간 git은 rename을 찾자마자 미리 띄우므로 그동안 출력이 쌓여 있다.
  - rename 원본 찾기는 gix의 rename 추적(짝 1,000개 한도라 문서 906개를 옮긴 커밋에서 놓쳤다) 대신 파일 하나만
    git과 같은 유사도로 본다. 그 커밋에서 3ms다.
  - 화면으로 보내는 JSON은 git 출력의 약 3배(`Makefile` 1.0MB)다. 커밋마다 부모 sha, 메일, 경로가 들어 있다.
- **라인 히스토리 구현(M2-c):** 앱 요청(`plx-line-log`: `git log -L` + hunk 해석 + 줄 내용 디코드)의 첫 커밋은
  git의 첫 출력과 측정 잡음 안에서 같다(2026-10-15 linux-container, 3회 중앙값, Bloom 있음: `diff.c` 258 → 194ms,
  `textModel.ts` 294 → 283ms, commit-graph 없음: 1463 → 1399, 2887 → 2965ms). 걸리는 시간은 거의 다 git이다.
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
- **콜드 스타트 구간(M2-a, CI):** Windows는 Tauri·WebView 477ms · JS·첫 렌더 191ms · 페인트 49ms로 예산 안(820ms)이다.
  macOS는 Tauri·WebView 523ms에 **페인트 597ms**가 더해져 1.2초다. 첫 내용을 그린 뒤 창을 보이는데(빈 화면 깜빡임
  방지), 숨긴 창의 WKWebView는 그리지 않으므로 창을 보인 뒤에야 첫 페인트가 시작된다(콜드 스타트의 blame도
  내용 표시 618ms). 창을 먼저 보이고(배경색을 테마와 같게) 내용을 이어 그리는 쪽을 macOS에서 재 본다.
- **macOS 창 먼저 보이기 실험(PR #12, 닫음):** 첫 내용을 그린 뒤가 아니라 처음부터 창을 테마 배경색으로 보이게 했다.
  CI macOS 콜드 스타트 2,454 → 1,444ms였지만 줄어든 것은 Tauri·WebView 구간(1,395 → 442ms, 러너마다 다르다)이고,
  노린 페인트 구간은 655 → 788ms로 줄지 않았다. WKWebView의 첫 페인트가 창을 보이든 말든 0.7~0.8초 걸린다.
  다음은 첫 화면 번들을 줄이고(CodeMirror를 첫 화면 뒤로) 페인트 구간을 더 잘게 재 본다.
- **M2 확인 사항:** changed-path Bloom 필터 v1은 경로에 0x80 이상 바이트가 있으면 murmur3 구현 차이(`char` 부호)로
  해시가 어긋날 수 있어, 최근 git에 v2(`commitGraph.changedPathsVersion`)가 생겼다. 한글 경로가 기본 시나리오이므로
  "가속 설정"이 v2로 쓰는지, gix로 필터를 읽을 때 버전을 맞추는지 확인한다.
