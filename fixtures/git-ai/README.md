# git-ai 골든 데이터

`crates/plx-core/tests/git_ai_golden.rs`가 쓴다. git-ai가 실제로 노트를 쓴 저장소에서, 줄마다 plx가 찾은 에이전트가
git-ai 1.7.6의 `git-ai blame --json`과 같은지 본다(M5 완료 기준). CI는 git-ai 없이 이 파일들만으로 대조한다.

시나리오마다 두 파일이 있다.
- `<이름>.bundle`: 그 저장소의 모든 참조(`refs/notes/ai` 포함). `git bundle create <이름>.bundle --all`
- `<이름>.json`: HEAD와, 파일마다 `git-ai blame --json`의 `lines`(지금 파일의 줄 범위 → 키)와
  `prompts`(키 → `agent_id`의 도구·id·모델, `human_author`)

같은 시나리오 10개를 두 형식으로 둔다.
- `<이름>`: git-ai 1.7.6이 노트를 썼다(새 형식: `s_…::t_…` 세션 키, `metadata.sessions`).
- `v1_<이름>`: git-ai 1.3.4가 노트를 썼다(옛 형식: 16자 프롬프트 키, `metadata.prompts`). 기대값은 지금 쓰는 1.7.6의 blame이다
  (`notes_git_ai_version`이 1.3.4). 옛 저장소를 지금 git-ai로 보는 경우와 같다.

## 만든 도구
git-ai(git-ai-project/git-ai, Apache-2.0)의 통합 테스트 도구(`TestRepo`, `mock_ai`·`mock_known_human` 체크포인트, 데몬
동기화)로 만들었다. 1.7.6은 `0670e7ef2759`(2026-09-09), 1.3.4는 태그 `v1.3.4`(`9604ea4`)다. 시나리오는
`plx_golden.rs`(1.7.6)와 `plx_golden_v1.rs`(1.3.4, 번들만 쓴다)이고, 1.3.4 번들의 기대값은 `v1_expected.py`가 1.7.6으로
만든다. 모두 우리가 쓴 코드다(git-ai 코드를 옮기지 않았다). 시나리오는 git-ai의 테스트 도구 안에서만 컴파일된다.

## 다시 만들기
세션 id와 커밋 sha는 실행마다 달라지므로 번들과 JSON은 늘 함께 바꾼다.

```
# 새 형식(1.7.6)
git clone https://github.com/git-ai-project/git-ai && cd git-ai
cp <parallax>/fixtures/git-ai/plx_golden.rs tests/integration/
# tests/integration/main.rs에 `mod plx_golden;`을 더한다
PLX_GOLDEN_OUT=<parallax>/fixtures/git-ai cargo test --test integration plx_golden

# 옛 형식(1.3.4): 번들을 쓴 뒤 1.7.6의 blame으로 기대값을 쓴다
git clone --branch v1.3.4 https://github.com/git-ai-project/git-ai git-ai-1.3.4 && cd git-ai-1.3.4
cp <parallax>/fixtures/git-ai/plx_golden_v1.rs tests/integration/
# tests/integration/main.rs에 `mod plx_golden_v1;`을 더한다
PLX_GOLDEN_OUT=<parallax>/fixtures/git-ai cargo test --test integration plx_golden_v1
python3 -I <parallax>/fixtures/git-ai/v1_expected.py <1.7.6의 target/debug/git-ai> <parallax>/fixtures/git-ai

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
| `cherry_pick` | 곁가지의 AI 커밋을 앞서 간 본줄기로 cherry-pick한다(git-ai가 새 커밋에 노트를 옮긴다) |
| `amend` | AI 줄을 더한 커밋을 `--amend`로 고친다 |
| `merge` | 곁가지의 AI 줄을 merge 커밋으로 합친다 |
| `korean_path` | 한글·공백 경로(`자료/설명 문서.md`)와 한글 내용 |
| `two_files` | 한 커밋에서 두 파일(하나는 하위 디렉터리)에 AI 줄 |

- `cherry_pick`은 본줄기를 한 커밋 앞으로 보낸 뒤 cherry-pick한다. 테스트 도구는 커밋 시각이 초 단위로 같아, 같은 부모
  위에 cherry-pick하면 원래 커밋과 sha가 같아진다(그러면 cherry-pick이 아니다).
- 두 형식 모두 git-ai는 cherry-pick한 커밋의 노트에 프롬프트·세션 기록을 함께 옮긴다. 그래서 키만 있고 기록은 다른 커밋의
  노트에 있는 경우(plx가 옛 형식 키를 다른 노트에서도 찾는 길)는 이 데이터에 없다. 그 길은 `tests/ai_notes.rs`의 손으로
  만든 노트로 본다.
- 테스트 도구는 파일 끝에 줄바꿈을 두지 않는다. 그 끝 줄 뒤에 AI가 줄을 붙이면 git-ai는 끝 줄도 AI 줄로 노트에 적고,
  git blame도 그 줄을 그 커밋으로 본다(`mixed_one_commit`의 6번째 줄, `cherry_pick`의 1번째 줄). 사람이 붙이면 반대로
  끝 줄이 사람 줄이 된다(`rename`의 5번째 줄). 어느 쪽이든 plx와 git-ai가 같게 보인다.
