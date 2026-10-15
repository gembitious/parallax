# 수동 확인 안내

CI(`.github/workflows/ci.yml`)가 Windows·macOS·Linux에서 빌드, 테스트, 앱 자동 확인(`scripts/check-app.sh`)을
돌린다. 여기에는 **사람이 화면을 봐야만 알 수 있는 것**만 적는다. 한 번에 다 하지 않아도 된다.
확인한 항목은 PR이나 이슈에 "Windows ✓ 3번" 식으로 남기면 `docs/MILESTONES.md`의 "실기 확인 필요"를 지운다.

## CI가 이미 확인하는 것 (다시 볼 필요 없음)
- 단일 인스턴스: 두 번째 `plx`가 상주 인스턴스로 넘어가고 바로 끝난다, 동시 실행해도 인스턴스는 하나, 강제 종료 뒤 재시작
- 잘못된 인자는 터미널에 오류를 쓰고 종료 코드 2
- 한글·공백 경로 전달, 한글 경로 저장소 파일 blame(커밋된 줄 + 미커밋 줄)
- blame이 끝까지 와서 거터를 그린다, 같은 파일을 다시 열면 캐시에서 온다
- 터미널에서 처음 실행하면 스스로 분리하고 셸을 바로 돌려준다(릴리스 빌드)
- 지연 측정: 콜드 스타트, plx 호출 → 첫 페인트, blame 첫 줄 표시(처음·캐시). 결과는 Actions 실행 화면의 Summary에 있다

## 0. 준비 (한 번)

필요한 것: git, [Volta](https://volta.sh)(Node·npm 버전을 `package.json`에 맞춰 자동으로 쓴다), [rustup](https://rustup.rs)
(`rust-toolchain.toml`의 Rust를 자동으로 받는다). Windows는 Git for Windows와 Visual Studio Build Tools(C++)도 필요하다.

```sh
git clone https://github.com/gembitious/parallax && cd parallax
npm install
npm run tauri build -- --no-bundle     # Windows·Linux: target/release/plx(.exe)
npm run tauri build -- --bundles app   # macOS: target/release/bundle/macos/parallax.app
```

빌드가 끝나면 `plx`를 PATH에서 부를 수 있게 한다.

- **Windows (Git Bash):** `export PATH="$PWD/target/release:$PATH"` (또는 `target/release`를 시스템 PATH에 추가)
- **macOS:** `scripts/install-macos-shim.sh target/release/bundle/macos/parallax.app` → `/usr/local/bin/plx`
  (쓸 수 없으면 `~/.local/bin/plx`에 설치하고 PATH 안내를 출력한다)

확인: `plx --version` → `plx 0.1.0`

이후 단계는 **아무 git 저장소 안에서** 한다(이 저장소도 된다). 개발 빌드 수치를 보려면 `PLX_PERF=1`을 붙여
실행한다(`` ` `` 키로 성능 HUD를 켜고 끈다).

## 1. 창과 상주 (M0)

| # | 할 일 | 기대 |
|---|---|---|
| 1 | `plx README.md` (처음 실행) | 셸이 바로 돌아오고, 잠시 뒤 창이 앞에 뜬다. 빈 화면 깜빡임 없이 파일 내용이 보인다 |
| 2 | 창을 다른 앱 뒤로 보낸 뒤 `plx CLAUDE.md` | 창이 **앞으로 오고** 내용이 바뀐다(0.1초 안) |
| 3 | 창의 닫기 버튼 | 창이 숨는다(앱은 살아 있다). macOS는 포커스가 터미널로 돌아간다 |
| 4 | 다시 `plx README.md` | 창이 바로 뜬다 |
| 5 | 트레이(메뉴 막대) 아이콘 → 열기 / 종료 | 열기는 창을 보인다, 종료는 앱을 끝낸다(다음 `plx`는 다시 콜드 스타트) |
| 6 | macOS: 창을 닫은 뒤 Dock 아이콘 클릭 | 창이 다시 보인다 |

## 2. blame 화면 (M1)

| # | 할 일 | 기대 |
|---|---|---|
| 1 | 이력이 긴 파일: `plx <파일>:<줄>` | 내용이 먼저 뜨고 왼쪽 거터가 곧 채워진다. 그 줄이 가운데에 선택돼 있다 |
| 2 | 거터 보기 | 같은 커밋이 이어지는 묶음의 첫 줄에 요약·작성자·"3일 전" 같은 시간, 모든 줄 왼쪽에 색 막대(최근일수록 주황, 오래될수록 파랑) |
| 3 | 파일을 고치고(저장만, 커밋 안 함) 다시 `plx <파일>` | 바꾼 줄이 "커밋 안 됨"(초록 기울임, 점선 막대) |
| 4 | 거터에 마우스를 올린다 | 카드: 커밋 요약, 전체 메시지, 작성자·이메일, 날짜(커밋한 곳의 시간대), sha, 바뀐 파일 수 |
| 5 | `j`/`k` | 커서가 줄 단위로 움직이고, 커서 줄 거터에 그 줄의 커밋 요약이 보인다. 같은 커밋 줄의 거터가 옅게 칠해진다 |
| 6 | `y` | "복사했다: abcd1234" 알림, 다른 곳에 붙여 넣으면 sha 전체 |
| 7 | `Enter` | 커서 줄의 카드가 고정(파란 테두리), 다시 `Enter`나 `Esc`로 닫힘 |
| 8 | `/` 후 단어 입력, `Enter`·`n`·`N` | 검색 결과로 이동 |
| 9 | `?` | 단축키 도움말 |
| 10 | OS를 다크 모드로 바꾼다 | 배경·글자·거터 색이 읽힌다 |
| 11 | 한글 경로·한글 내용 파일, EUC-KR 파일 | 글자가 깨지지 않는다. EUC-KR이면 머리줄에 `EUC-KR` 표시 |

## 3. 이력 거슬러 오르기 (M1)

| # | 할 일 | 기대 |
|---|---|---|
| 1 | 여러 번 고친 줄에서 `,` | 그 줄을 바꾸기 직전 리비전의 blame으로 간다(머리줄에 sha, `뒤로 q · 1`). 커서가 그 줄 근처 |
| 2 | `,`를 몇 번 더 | 계속 거슬러 오른다. 처음 생긴 커밋에서는 "처음 생긴 커밋" 알림 |
| 3 | `q`를 여러 번 | 한 단계씩 돌아오고, 돌아온 화면은 커서·스크롤이 그대로다. 맨 앞에서 `q`는 창을 숨긴다 |
| 4 | `<` / `>` | 파일을 바꾼 이전·다음 커밋 버전으로 간다. `<` 다음 `>`는 원래 화면으로 돌아온다 |
| 5 | 같은 파일을 다시 `plx` | 거터가 거의 즉시 찬다(캐시). HUD를 켰다면 `(캐시)` 기록 |

## 문제를 알릴 때
- 플랫폼(Windows/macOS 버전), 위 표의 번호, 실제로 본 것
- 가능하면 `PLX_PERF=1`로 실행한 성능 HUD 캡처
- 콘솔 출력이 필요하면 트레이에서 종료한 뒤 `PLX_DETACH=0 plx <파일>`로 실행한다(상주 인스턴스가 터미널에 붙어 로그가 남는다)
