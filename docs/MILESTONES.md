# parallax 마일스톤

**진행 중: M2** (M0·M1은 Windows·macOS 화면 확인 대기) — 마일스톤을 끝낼 때마다 이 줄과 체크박스를 갱신한다.

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
| 그래프 스크롤 | 60fps (참조 저장소 전체 이력) |

## M0 — 뼈대와 측정
- [x] Tauri 2 + React + Vite 스캐폴딩, `crates/plx-core` 분리 — Windows·macOS 실기 확인 필요
- [x] 단일 인스턴스 + argv/cwd 전달, 창 닫기 = 숨기기, 트레이(종료) — Windows·macOS 실기 확인 필요
- [x] `plx` 인자 파싱 (CLAUDE.md의 CLI 인터페이스), macOS shim — Windows·macOS 실기 확인 필요
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
- [x] 캐시: 커밋 고정 blame은 디스크 영구 캐시, 작업트리 blame은 HEAD + 파일 지문 — Windows·macOS 실기 확인 필요

**완료 기준:** blame 성능 예산 충족. 부모 이동 줄 매핑이 fixtures(이동·rename·merge 포함) 골든 테스트를 통과한다.

## M2 — 히스토리
- [x] 파일 히스토리: `git log -- <path>`를 스트리밍하고, 파일이 rename으로 생긴 커밋에서 옛 이름의 `git log` 구간을 이어 붙인다
      (두 방식을 측정해 `--follow`는 쓰지 않기로 확정: CLAUDE.md 결정 기록, bench/results.md) — Windows·macOS 실기 확인 필요
- [ ] 라인/범위 히스토리 (`git log -L`), 코드 뷰 선택 영역에서 바로 진입
- [x] 커밋 상세: gix 트리 diff + `@codemirror/merge` (분할/통합 전환), 히스토리에서 열면 해당 파일 diff만
      — Windows·macOS 실기 확인 필요
- [ ] Bloom 필터 감지 → 없으면 "가속 설정" 버튼
      (사용자 확인 후 `git commit-graph write --reachable --changed-paths`, `commitGraph.changedPaths=true`)

**완료 기준:** 히스토리·커밋 상세 예산 충족. Bloom 필터 유무별 측정을 `bench/results.md`에 추가.

## M3 — 커밋 그래프
- [ ] gix 리비전 워크(topo 순) + 레인 레이아웃은 Rust에서 계산, 가시 영역 행만 전송
- [ ] 가상 스크롤, 브랜치·태그·HEAD 라벨
- [ ] 검색: 메시지·작성자·경로
- [ ] orrery의 SVG 렌더링 재사용 검토 (성능 미달이면 canvas)

**완료 기준:** 참조 저장소 전체 이력에서 첫 화면 ≤ 150ms, 스크롤 60fps.

## M4 — 라이브 갱신과 동선
- [ ] `.git` 감시 → 에이전트 커밋 자동 반영 + 캐시 무효화
- [ ] 워크트리 목록·전환
- [ ] Quick Open: gix 인덱스의 추적 파일 + 퍼지 매칭
- [ ] 커맨드 팔레트, 에디터로 열기 (VS Code·Zed, `file:line`)

**완료 기준:** 외부 커밋 후 1초 안에 반영. 키보드만으로 M1~M3 기능 전부 사용 가능.

## M5 — (선택) AI 출처 오버레이
- [ ] `refs/notes/ai` 읽기 (git-ai 노트 포맷 스펙 확인 후 구현)
- [ ] AI 작성 줄 거터 표시, hover에 에이전트·세션·프롬프트 정보

**완료 기준:** `git-ai blame` 결과와 줄 단위로 일치.

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