# git-ai 골든 데이터

`crates/plx-core/tests/git_ai_golden.rs`가 쓴다. git-ai가 실제로 노트를 쓴 저장소에서, 줄마다 plx가 찾은 에이전트가
`git-ai blame --json`과 같은지 본다(M5 완료 기준). CI는 git-ai 없이 이 파일들만으로 대조한다.

시나리오마다 두 파일이 있다.
- `<이름>.bundle`: 그 저장소의 모든 참조(`refs/notes/ai` 포함). `git bundle create <이름>.bundle --all`
- `<이름>.json`: HEAD와, 파일마다 `git-ai blame --json`의 `lines`(지금 파일의 줄 범위 → 키)와
  `prompts`(키 → `agent_id`의 도구·id·모델, `human_author`)

## 만든 도구
git-ai 1.7.6(git-ai-project/git-ai `0670e7ef2759`, 2026-09-09, Apache-2.0)의 통합 테스트 도구(`TestRepo`, `mock_ai`·
`mock_known_human` 체크포인트, 데몬 동기화)로 만들었다. 시나리오는 `plx_golden.rs`이고 우리가 쓴 코드다(git-ai 코드를
옮기지 않았다). git-ai의 테스트 도구 안에서만 컴파일된다.

## 다시 만들기
세션 id와 커밋 sha는 실행마다 달라지므로 번들과 JSON은 늘 함께 바꾼다.

```
git clone https://github.com/git-ai-project/git-ai && cd git-ai
cp <parallax>/fixtures/git-ai/plx_golden.rs tests/integration/
# tests/integration/main.rs에 `mod plx_golden;`을 더한다
PLX_GOLDEN_OUT=<parallax>/fixtures/git-ai cargo test --test integration plx_golden
cd <parallax> && cargo test -p plx-core --test git_ai_golden
```

## 시나리오
| 이름 | 내용 |
|---|---|
| `basic` | 사람이 쓴 파일에 AI가 줄을 더하고, 다음 커밋에서 사람이 AI 줄 하나를 고친다 |
| `mixed_one_commit` | 한 커밋 안에서 AI가 줄을 더하고 사람이 그중 하나를 고치고 지운다. AI 세션 둘 |
| `rename` | AI가 쓴 파일의 이름을 바꾼 뒤 사람이 줄을 더한다(blame의 원래 경로가 옛 이름) |
| `ai_over_ai` | AI가 쓴 줄을 다음 커밋에서 AI가 다시 고친다 |
| `shifted` | 사람이 위에 줄을 더하고 AI 줄 하나를 지운다(원래 줄 번호가 지금과 다르다) |
| `cherry_pick` | 곁가지의 AI 커밋을 본줄기로 cherry-pick한다(git-ai가 새 커밋에 노트를 옮긴다) |
| `amend` | AI 줄을 더한 커밋을 `--amend`로 고친다 |
| `merge` | 곁가지의 AI 줄을 merge 커밋으로 합친다 |
| `korean_path` | 한글·공백 경로(`자료/설명 문서.md`)와 한글 내용 |
| `two_files` | 한 커밋에서 두 파일(하나는 하위 디렉터리)에 AI 줄 |

`mixed_one_commit`의 6번째 줄(`three`)은 처음 커밋에서 파일 끝 줄바꿈 없이 끝났다. AI가 그 뒤에 줄을 붙이자 git-ai는
이 줄도 AI 세션의 줄로 적었고, git blame도 이 줄을 그 커밋으로 본다. 그래서 plx와 git-ai 모두 AI 줄로 보인다.
