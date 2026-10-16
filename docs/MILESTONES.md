# parallax 마일스톤

**M0~M5 구현 완료** (모두 Windows·macOS 화면 확인 대기. 그래프 스크롤 60fps는 GPU가 있는 Windows·macOS에서 재고,
M5는 실제 git-ai와의 대조가 남음) — 마일스톤을 끝낼 때마다 이 줄과 체크박스를 갱신한다.

## 성능 예산
baseline은 M0에서 같은 머신·같은 저장소로 git CLI를 직접 잰 값이다. 이력 질의 목표는 "baseline + 우리 오버헤드"로 정의한다.
참조 저장소는 `git/git`(중대형)과 `microsoft/vscode`(대형 프론트엔드). 측정은 commit-graph Bloom 필터 있음/없음 두 조건 모두.

| 항목 | 목표 |
|---|---|
| 상주 인스턴스에 `plx` 호출 → 대상 뷰 첫 페인트 | ≤ 100ms |
| 콜드 스타트 → 첫 페인트 | ≤ 1s (M0 측정 후 플랫폼별 조정) |
| blame 첫 줄 표시 | `git blame --incremental` 첫 출력 + 30ms 이내 |
| 파일 히스토리 첫 50개 | `git log` 첫 50개 + 30ms 이내 |
| 커밋 상세 (파일 목록 + diff) | ≤ 50ms (gix 인프로세스) |
| 캐시 히트 재표시 | ≤ 16ms |
| Quick Open 글자마다 결과 표시 | ≤ 50ms (백엔드 검색은 vscode 2만 파일에서 ≤ 10ms) |
| 그래프 스크롤 | 60fps (참조 저장소 전체 이력) |

## M0 — 뼈대와 측정
- [x] Tauri 2 + React + Vite 스캐폴딩, `crates/plx-core` 분리 — Windows·macOS 실기 확인 필요
- [x] 단일 인스턴스 + argv/cwd 전달, 창 닫기 = 숨기기, 트레이(종료). 파이프로 불러도 분리 실행하고 뜨지 못하면 알린다,
      인스턴스 자리는 사용자로 정한다(환경 변수와 무관), Windows pipe는 이 사용자 전용 — Windows·macOS 실기 확인 필요
- [x] `plx` 인자 파싱 (CLAUDE.md의 CLI 인터페이스), macOS shim. 넘기기 전에 대상 저장소·리비전 확인 — Windows·macOS 실기 확인 필요
- [x] 개발 빌드 성능 HUD: 요청별 소요 ms — Windows·macOS 실기 확인 필요
- [x] `bench/`: 참조 저장소 클론 스크립트, git baseline 측정 → `bench/results.md` — Windows·macOS 실기 확인 필요
- [x] `fixtures/`: rename·이동·merge·CRLF·한글 경로/내용을 포함한 테스트 저장소 생성 스크립트 — Windows·macOS 실기 확인 필요

**완료 기준:** 두 번째 `plx` 호출이 100ms 안에 기존 창을 앞으로 가져오고 인자를 반영한다(Windows·macOS 모두).
baseline 표가 `bench/results.md`에 있다.

## M1 — Blame 뷰 (핵심)
- [x] CodeMirror 6 읽기 전용 뷰 + blame 거터: 작성자·상대 시간·요약, 커밋 나이 히트맵 — Windows·macOS 실기 확인 필요
- [x] `git blame --incremental` 스트리밍 파서 → Channel 청크 → 거터 점진 채움 — Windows·macOS 실기 확인 필요
- [x] 미커밋 줄 표시 (zero sha) — Windows·macOS 실기 확인 필요
- [x] hover 카드: 전체 메시지, 날짜, 변경 파일 수, sha 복사 — Windows·macOS 실기 확인 필요
- [x] `,` 부모 리비전 blame: porcelain의 `previous <sha> <path>` 사용, 커밋 diff로 줄 위치 매핑
      (그 커밋에서 추가된 줄은 해당 hunk 시작으로) — Windows·macOS 실기 확인 필요
- [x] `<`/`>` 리비전 이동, 탐색 스택(`q`로 뒤로) — Windows·macOS 실기 확인 필요
- [x] 캐시: 커밋 고정 blame은 디스크 영구 캐시, 작업트리 blame은 HEAD + 파일 내용(git에 준 내용). 둘 다 blame 설정을
      열쇠에 넣고 얕은 클론은 넣지 않는다 — Windows·macOS 실기 확인 필요

**완료 기준:** blame 성능 예산 충족. 부모 이동 줄 매핑이 fixtures(이동·rename·merge 포함) 골든 테스트를 통과한다.

## M2 — 히스토리
- [x] 파일 히스토리: `git log -- <path>`를 스트리밍하고, 파일이 rename으로 생긴 커밋에서 옛 이름의 `git log` 구간을 이어 붙인다
      (두 방식을 측정해 `--follow`는 쓰지 않기로 확정: CLAUDE.md 결정 기록, bench/results.md) — Windows·macOS 실기 확인 필요
- [x] 라인/범위 히스토리 (`git log -L`), 코드 뷰 선택 영역에서 바로 진입(`L`, `J`/`K`로 선택) — Windows·macOS 실기 확인 필요
- [x] 커밋 상세: gix 트리 diff + `@codemirror/merge` (분할/통합 전환), 히스토리에서 열면 해당 파일 diff만.
      rename은 git과 같은 단계로 찾고(`plx_core::renames`), 큰 커밋은 목록을 먼저 보이고 줄 수를 나눠 받는다.
      큰 파일(4MB 넘음)은 "그래도 보기", 줄 diff에 보이지 않는 바뀜(모드·줄 끝·끝 개행·BOM·인코딩)은 알림
      — Windows·macOS 실기 확인 필요
- [x] Bloom 필터 감지 → 없으면 "가속 설정" 버튼
      (사용자 확인 후 `git commit-graph write --reachable --changed-paths`. 필터 버전은 git 기본값(그래프에 있는 버전,
      없으면 v1)이고, 얕은 클론·`git replace`·트리 없는 부분 클론처럼 쓸 수 없는 저장소는 까닭을 보인다.
      `commitGraph.changedPaths` 설정은 git에 없어 쓰지 않는다: CLAUDE.md 결정 기록) — Windows·macOS 실기 확인 필요

**완료 기준:** 히스토리·커밋 상세 예산 충족. Bloom 필터 유무별 측정을 `bench/results.md`에 추가.

## M3 — 커밋 그래프
- [x] 커밋 DAG(commit-graph 위치 번호 → 앱 캐시의 커밋 표 → gix 객체) + `git log --topo-order`와 같은 순서 +
      레인 레이아웃은 Rust에서 계산, 가시 영역 행만 전송 (gix topo 워크는 세대 번호 v1만 읽어 원격 브랜치가 많으면 느려
      쓰지 않는다: CLAUDE.md 결정 기록, bench/results.md) — Windows·macOS 실기 확인 필요
- [x] 가상 스크롤, 브랜치·태그·HEAD 라벨. 인자 없는 `plx`가 cwd 저장소의 그래프를 열고(HEAD 커밋을 골라 둔다),
      blame·히스토리·커밋 상세의 `G`는 그 커밋을 고른 그래프로 간다. 범위는 HEAD / 브랜치(로컬 + upstream, 기본) /
      원격 포함(`R`) — Windows·macOS 실기 확인 필요
- [x] 검색: 메시지·작성자·경로. `/`에 글을 넣으면 메시지(모든 줄)·작성자·메일·sha 앞부분을 행 순서대로 찾고,
      `path:경로`면 그 경로를 바꾼 커밋(`git log --full-history --no-merges`, Bloom 필터)을 모두 강조한다. `n`/`N` 다음·이전
      — Windows·macOS 실기 확인 필요
- [x] orrery의 SVG 렌더링 재사용 검토 (성능 미달이면 canvas) → 재사용하지 않고 캔버스로 그린다(CLAUDE.md 결정 기록).
      orrery는 커밋·선·이름표마다 SVG 요소를 만들고 선을 자식 → 부모 곡선 하나로 그린다(커밋 수십 개의 상태 전이
      애니메이션용). 우리는 화면 위아래 1,000px만 그리고 행과 레인 캔버스를 스크롤 내용 안에 둬 그 안의 스크롤은 다시
      그리지 않는다 — Windows·macOS에서 스크롤 프레임 확인 필요

**완료 기준:** 참조 저장소 전체 이력에서 첫 화면 ≤ 150ms, 스크롤 60fps.
→ 첫 화면 git/git 17ms, vscode 45~50ms(Linux). 스크롤은 미리 그린 범위 안이면 JS가 없고(휠 프레임의 약 2/3),
범위를 옮기는 프레임은 p95 17~27ms다. Xvfb는 소프트웨어 페인트라 JS가 없는 프레임도 22~25ms여서 60fps는 GPU가 있는
Windows·macOS의 성능 HUD(`그래프 … 스크롤`)로 확인한다(bench/results.md).

## M4 — 라이브 갱신과 동선
- [x] `.git` 감시 → 에이전트 커밋 자동 반영 + 캐시 무효화: 열린 그래프는 깜빡이지 않고 다시 읽고, 작업트리 blame은
      HEAD가 바뀌거나 파일을 고치면(커밋 전에도), HEAD부터의 히스토리는 HEAD가 바뀌면 다시 연다(창·검색 칸이 열려
      있으면 닫을 때, 검색어는 넘긴다). 캐시는 불변 열쇠라 지울 것이 없다(CLAUDE.md 결정 기록). Linux에서 외부 커밋 →
      그래프 반영 약 0.2초 — Windows·macOS 실기 확인 필요
- [x] 워크트리 목록·전환: 어느 화면에서든 `W`로 주 작업트리와 링크된 워크트리(브랜치, HEAD 요약, 잠김·폴더 없음)를
      보이고, 고르면 그 워크트리의 같은 화면으로 간다(작업트리 blame·HEAD 히스토리는 같은 경로, 라인 히스토리는 줄 범위를
      그 워크트리로 옮기고, 리비전에 고정된 blame·히스토리는 같은 리비전, 커밋 상세는 그 워크트리 그래프의 같은 커밋,
      나머지는 그래프). 지금 작업트리는 git 디렉터리로 가린다(폴더를 옮겨도 맞다). 그래프 이름표는 다른 워크트리가
      체크아웃한 브랜치(여럿이면 모두)·HEAD에 그 워크트리 이름을 붙이고, 다른 워크트리의 detached HEAD도 시작점이다
      — Windows·macOS 실기 확인 필요
- [x] Quick Open: gix 인덱스의 추적 파일 + 퍼지 매칭. 어느 화면에서든 `f`·Ctrl+P(⌘P)로 열고 `Enter` blame,
      `Shift+Enter` 파일 히스토리, `파일:줄`이면 그 줄로. 파일 이름 일치를 앞세우고(VS Code처럼) 한글 초성·조합 중인
      음절도 맞춘다(`plx_core::fuzzy`, CLAUDE.md 결정 기록). 빈 검색어는 최근 연 파일부터, 작업트리에서 지운 파일은
      표시하고 HEAD 버전 blame으로 — Windows·macOS 실기 확인 필요
- [x] 커맨드 팔레트, 에디터로 열기 (VS Code·Zed, `file:line`): 어느 화면에서든 `:`·Ctrl+Shift+P(⌘⇧P)로 그 화면의
      할 일을 이름·초성으로 찾아 실행한다(키가 없던 그래프 범위·가속 설정 포함). `o`는 작업트리 파일의 그 줄을
      에디터로 연다(`PLX_EDITOR`, 리비전의 줄은 작업트리 줄로, 그 뒤 rename은 새 이름으로). blame의 커밋 카드는 `i`
      — Windows·macOS 실기 확인 필요

**완료 기준:** 외부 커밋 후 1초 안에 반영. 키보드만으로 M1~M3 기능 전부 사용 가능.
→ Linux 충족: 외부 커밋 반영 약 0.2초(CI는 2초 안에 확인), 마우스로만 되던 커밋 카드 고정·그래프 HEAD 범위·가속 설정을
키(`i`)와 명령 팔레트로 옮겼다. Windows·macOS 확인 대기.

## M5 — (선택) AI 출처 오버레이
- [x] `refs/notes/ai` 읽기 (git-ai 노트 포맷 스펙 확인 후 구현): Git AI Standard v3.0.0(`authorship/3.0.0`)의 세션·사람·옛
      프롬프트 키, 따옴표 경로, fanout 노트 트리. 규칙은 git-ai의 표준 문서와 `git-ai blame` 소스에 맞췄다
      (`plx_core::ai_notes`, CLAUDE.md 결정 기록) — Windows·macOS 실기 확인 필요
- [x] AI 작성 줄 거터 표시, hover에 에이전트·세션·프롬프트 정보: 보라 줄과 `✦ 도구`, 카드에 모델·시킨 사람·(옛 형식이면)
      받아들인 줄 수, 머리줄에 AI 줄 수 — Windows·macOS 실기 확인 필요

**완료 기준:** `git-ai blame` 결과와 줄 단위로 일치.
→ 같은 규칙(원래 커밋·원래 경로·원래 줄, 뒤 항목 우선, 없는 키 건너뛰기)을 표준 예제와 손으로 만든 노트로 확인했다.
실제 `git-ai blame`과의 대조는 git-ai를 쓰는 저장소에서 `docs/MANUAL_CHECKS.md` 13번으로 한다.

## 하지 않는 것
편집 · LSP · 내장 터미널 · 스테이징/커밋/푸시 · PR 등 원격 호스팅 연동.
다시 꺼내려면 CLAUDE.md 결정 기록에 이유부터 남긴다.

## 리스크와 대응
| 리스크 | 대응 |
|---|---|
| gix-blame 결과가 git과 다름 (diff slider 처리 차이) | 골든 테스트 통과 전 기본값 금지 |
| 윈도우 프로세스 생성 비용 | gix 인프로세스 우선, 스폰 횟수 측정 |
| WebView 콜드 스타트 | 상주로 회피, 콜드 스타트는 1s 예산만 지킴 |
| 한글 경로·내용 | `core.quotepath=false`, `-z`, 바이트 파싱, fixtures 회귀 테스트 |
| 대형 파일 blame 지연 | incremental 스트리밍 + 커밋 고정 캐시 |
| 다른 프로젝트 코드의 라이선스(공개 저장소, MIT OR Apache-2.0) | GPL 코드(git 등)는 옮기지 않고 출력과의 골든 테스트로 동작만 맞춘다. 허용 라이선스 코드를 옮기면 그 파일 맨 위에 고지 |
| 바이너리 배포 | 의존성의 저작권 고지는 릴리스 빌드가 모아 앱에 넣는다(`npm run notices`, 앱의 "오픈 소스 라이선스"). 배포 전에 Tauri 기본 아이콘을 우리 것으로 바꾼다 |
