#!/usr/bin/env bash
# 골든 테스트용 git 저장소를 만든다. 어느 플랫폼에서 만들어도 커밋 sha가 같아야 한다.
#
# 사용법: fixtures/make-repos.sh <출력 디렉터리>
#   <출력>/history   rename, 디렉터리 이동, 파일 안·파일 간 줄 이동, merge(양쪽 수정, merge에서만 생긴 줄,
#                    충돌 해결), 삭제 후 재추가, 바이너리, 끝 개행 없음, 빈 파일, 브랜치·태그, 미커밋 변경
#   <출력>/encoding  CRLF, LF→CRLF 전환, 섞인 줄 끝, 한글 경로·내용·커밋 메시지·작성자,
#                    한글 경로 rename, 공백·이모지 경로, EUC-KR 바이트(비 UTF-8)
#   이미 있으면 지우고 다시 만든다.
#
# 결정성을 지키는 규칙
# - 시스템·전역 git 설정을 읽지 않는다(GIT_CONFIG_NOSYSTEM, GIT_CONFIG_GLOBAL). 저장소 설정만 쓴다.
# - 작성자·커미터·시각을 커밋마다 고정한다. merge 메시지도 직접 준다(git 버전마다 기본 문구가 다르다).
# - 실행 권한·심볼릭 링크·대소문자만 다른 rename은 쓰지 않는다(Windows·macOS 파일 시스템 차이).
# - 한글이 들어간 경로·메시지·이름은 명령 인자 대신 파일과 `git add -A`로 넘긴다(Windows 인자 변환 회피).
# - 경로 이름은 NFC로만 쓴다(macOS core.precomposeunicode와 결과가 같도록).
#
# macOS 기본 bash 3.2와 Git Bash에서 돌아가게 쓴다. 바꾸면 crates/plx-core/tests/fixtures.rs의 기대 sha를 갱신한다.
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "사용법: $0 <출력 디렉터리>" >&2
  exit 2
fi
mkdir -p "$1"
out="$(cd "$1" && pwd)"

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
  GIT_CONFIG GIT_CONFIG_PARAMETERS GIT_CONFIG_COUNT GIT_NAMESPACE GIT_CEILING_DIRECTORIES \
  GIT_TEMPLATE_DIR GIT_EDITOR GIT_SEQUENCE_EDITOR
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_TERMINAL_PROMPT=0
export GIT_AUTHOR_NAME="Fixture Author" GIT_AUTHOR_EMAIL="author@example.com"
export GIT_COMMITTER_NAME="Fixture Committer" GIT_COMMITTER_EMAIL="committer@example.com"

# 커밋 시각: 2023-11-14T22:13:20Z부터 커밋마다 1시간씩.
tick=0
next_date() {
  tick=$((tick + 1))
  date="$((1700000000 + tick * 3600)) +0900"
}

commit() { # commit <메시지>
  next_date
  git add -A
  GIT_AUTHOR_DATE="$date" GIT_COMMITTER_DATE="$date" git commit -q --no-verify -m "$1"
}

commit_msgfile() { # commit_msgfile <메시지> — 한글 메시지는 파일로 넘긴다
  printf '%s\n' "$1" >"$out/.msg"
  next_date
  git add -A
  GIT_AUTHOR_DATE="$date" GIT_COMMITTER_DATE="$date" git commit -q --no-verify -F "$out/.msg"
}

# 작성자를 환경 변수 대신 저장소 설정(user.name)에서 읽게 해 커밋한다. 한글 이름을 인자로 넘기지 않으려는 것.
commit_msgfile_as_config_user() { # commit_msgfile_as_config_user <메시지>
  printf '%s\n' "$1" >"$out/.msg"
  next_date
  git add -A
  (
    unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL
    GIT_AUTHOR_DATE="$date" GIT_COMMITTER_DATE="$date" git commit -q --no-verify -F "$out/.msg"
  )
}

# 커밋하지 않고 merge만 한다. 충돌이 나면 1을 돌려준다.
# 메시지는 merge_commit에서 직접 준다(git 버전마다 기본 문구와 충돌 주석이 다르다).
merge_start() { # merge_start <브랜치>
  git merge -q --no-ff --no-commit "$1" >/dev/null 2>&1
}

merge_commit() { # merge_commit <메시지>
  next_date
  git add -A
  GIT_AUTHOR_DATE="$date" GIT_COMMITTER_DATE="$date" git commit -q --no-verify -m "$1"
}

init_repo() { # init_repo <이름>
  rm -rf "${out:?}/$1"
  mkdir -p "$out/$1"
  cd "$out/$1"
  git init -q -b main .
  # 이후 테스트에서 사용자 전역 설정(autocrlf 등)이 작업트리를 바꾸지 않게 저장소에 고정한다.
  git config core.autocrlf false
  git config core.safecrlf false
  git config core.quotepath false
  git config commit.gpgsign false
  git config tag.gpgsign false
  git config gc.auto 0
}

# 블록 이동 감지(blame -M/-C)는 줄당 영숫자 20자 이상이어야 하므로 줄을 길게 쓴다.
lines() { # lines <접두어> <시작> <끝>
  local i
  for i in $(seq "$2" "$3"); do
    printf '%s line %02d: the quick brown fox jumps over the lazy dog\n' "$1" "$i"
  done
}

# ---------------------------------------------------------------- history
init_repo history

mkdir -p src lib docs assets
lines lib 1 10 >src/lib.rs
printf 'fn old_name() {\n    println!("renamed later");\n}\n' >src/old_name.rs
lines util 1 6 >lib/util.rs
{
  lines alpha 1 5
  lines beta 1 5
  lines gamma 1 5
} >src/moves.rs
printf '# notes\n\nfirst version\n' >docs/notes.md
: >empty.txt
commit "initial import"
git tag v0.1

# src/lib.rs에 이력을 쌓는다.
for i in 1 2 3 4 5; do
  sed "s/^lib line 0$i: .*/lib line 0$i: edited in commit $i, jumps over the lazy dog/" src/lib.rs >src/lib.rs.tmp
  mv src/lib.rs.tmp src/lib.rs
  commit "edit lib line $i"
done

# rename + 내용 수정 (유사도 50% 이상)
git mv src/old_name.rs src/new_name.rs
printf 'fn new_name() {\n    println!("renamed later");\n}\n' >src/new_name.rs
commit "rename old_name.rs to new_name.rs"

# 디렉터리 이동
mkdir -p core
git mv lib/util.rs core/util.rs
commit "move lib/ to core/"

# 파일 안에서 블록 이동: gamma 블록을 맨 앞으로
{
  lines gamma 1 5
  lines alpha 1 5
  lines beta 1 5
} >src/moves.rs
commit "move gamma block to top"

# 파일 사이 블록 이동: beta 블록을 src/target.rs로
{
  lines gamma 1 5
  lines alpha 1 5
} >src/moves.rs
{
  printf '// target\n'
  lines beta 1 5
} >src/target.rs
commit "move beta block to target.rs"

# 삭제 후 다른 내용으로 다시 추가
git rm -q docs/notes.md
commit "remove notes"
mkdir -p docs # git rm이 빈 디렉터리도 지운다
printf '# notes\n\nsecond life\n' >docs/notes.md
commit "re-add notes"

# 바이너리(NUL 포함), 끝 개행 없음
printf '\211PNG\r\n\032\n\000\000\000\rIHDR\000\001\002\003' >assets/logo.bin
printf 'no newline at end' >src/no_eol.txt
commit "add binary and no-eol files"

# merge: 양쪽이 같은 파일의 다른 곳을 고치고, merge 커밋에서만 생긴 줄이 있다.
git checkout -q -b feature
sed 's/^lib line 02: .*/lib line 02: changed on feature branch, over the lazy dog/' src/lib.rs >src/lib.rs.tmp
mv src/lib.rs.tmp src/lib.rs
commit "feature: change line 2"
lines feature 1 3 >src/feature.rs
commit "feature: add feature.rs"
git checkout -q main
sed 's/^lib line 09: .*/lib line 09: changed on main branch, over the lazy dog/' src/lib.rs >src/lib.rs.tmp
mv src/lib.rs.tmp src/lib.rs
commit "main: change line 9"
merge_start feature
# merge 커밋에서만 생긴 줄(evil merge)
printf 'evil line added only in the merge commit, over the lazy dog\n' >>src/lib.rs
merge_commit "Merge branch feature"

# 충돌 merge: 같은 줄을 양쪽에서 고치고 손으로 푼다.
printf 'shared line: original value of the conflicting line\n' >src/conflict.rs
commit "add conflict.rs"
git checkout -q -b conflict
printf 'shared line: value chosen on the conflict branch\n' >src/conflict.rs
commit "conflict: change shared line"
git checkout -q main
printf 'shared line: value chosen on the main branch\n' >src/conflict.rs
commit "main: change shared line"
if merge_start conflict; then
  echo "충돌이 나야 하는데 나지 않았다" >&2
  exit 1
fi
printf 'shared line: resolved by combining main and conflict values\n' >src/conflict.rs
merge_commit "Merge branch conflict"

# annotated 태그의 tagger 시각도 고정한다.
next_date
GIT_COMMITTER_DATE="$date" git tag -a v0.2 -m "release 0.2"

# 합치지 않은 브랜치
git checkout -q -b topic
printf 'topic work, not merged into main yet, lazy dog\n' >src/topic.rs
commit "topic: start"
printf 'topic work continues on the unmerged branch, lazy dog\n' >>src/topic.rs
commit "topic: continue"
git checkout -q main

# 미커밋 변경: 수정 1줄 + 추가 1줄(blame의 zero sha), 추적하지 않는 파일
sed 's/^lib line 04: .*/lib line 04: uncommitted edit in the working tree, lazy dog/' src/lib.rs >src/lib.rs.tmp
mv src/lib.rs.tmp src/lib.rs
printf 'uncommitted new line at the end, over the lazy dog\n' >>src/lib.rs
printf 'untracked\n' >untracked.txt

# ---------------------------------------------------------------- encoding
init_repo encoding

printf 'first\r\nsecond\r\nthird\r\n' >crlf.txt
printf 'one\ntwo\nthree\n' >lf-to-crlf.txt
printf 'lf line\ncrlf line\r\nlf again\n' >mixed.txt
mkdir -p 한글
printf '안녕하세요\n한글 내용 둘째 줄\n' >"한글/파일 이름.txt"
printf '# 설명\n\n저장소 설명 문서\n' >한글/설명.md
printf 'emoji path\n' >"이모지-😀.txt"
# EUC-KR로 쓴 "안녕하세요" (UTF-8이 아닌 바이트)
printf '\276\310\263\347\307\317\274\274\277\344\n' >euc-kr.txt
commit_msgfile "초기 커밋: 한글 경로와 줄 끝 시나리오"

# LF → CRLF 전환 (모든 줄이 바뀐다)
printf 'one\r\ntwo\r\nthree\r\n' >lf-to-crlf.txt
commit "convert lf-to-crlf.txt to CRLF"

# 한글 내용 수정 + 한글 작성자(저장소 설정 파일에 바이트로 쓴다)
printf '[user]\n\tname = 김개발\n\temail = kim@example.com\n' >>.git/config
printf '안녕하세요\n한글 내용 둘째 줄을 고쳤다\n셋째 줄\n' >"한글/파일 이름.txt"
commit_msgfile_as_config_user "한글 파일 수정"

# 한글 경로 rename (디렉터리 이동)
mkdir -p 문서
mv 한글/설명.md 문서/설명.md
commit_msgfile "설명.md를 문서/로 옮김"

rm -f "$out/.msg"
echo "만들었다: $out/history, $out/encoding"
