#!/usr/bin/env bash
# 빌드한 앱을 자동으로 확인하고 지연을 잰다. CI(Windows·macOS·Linux)와 개발 머신에서 같이 쓴다.
#
# 사용법: scripts/check-app.sh [plx 실행 파일]
#   실행 파일 기본값 target/release/plx(.exe). 먼저 `npm run tauri build -- --no-bundle`로 빌드한다.
#   실행 중인 parallax가 있으면 먼저 종료한다(트레이 → 종료).
#   PLX_CHECK_RECORD=<이름>을 주면 지연 표를 bench/results.md의 "앱 지연"에 그 이름으로 넣는다(같은 이름은 덮어씀).
#
# 자동으로 확인하는 것
#   1. plx --version, 잘못된 인자(종료 코드 2, 상주 인스턴스를 띄우지 않음)
#   2. 콜드 스타트 → 첫 화면, 두 번째 호출이 상주 인스턴스로 전달되어 그려짐(한글·공백 경로 포함),
#      blame이 끝까지 와서 거터가 그려짐(한글 경로의 저장소 파일 포함), 커밋 상세,
#      파일·라인 히스토리(이름을 바꾼 한글 경로 파일의 옛 이름 구간까지), 커밋 그래프(인자 없는 plx),
#      다른 터미널의 커밋이 열린 그래프에 반영됨(.git 감시), 고친 파일이 열린 blame에 반영됨
#   3. 동시에 여러 번 실행해도 상주 인스턴스는 하나
#   4. 강제 종료 뒤 다시 시작(남은 소켓·락 정리)
#   5. 터미널에서 처음 실행하면 스스로 분리하고 셸을 바로 돌려줌(릴리스 빌드)
#   6. 앱 지연(bench/app-latency.sh)
# 화면을 봐야 하는 것(창이 앞으로 오는지, 닫으면 숨는지, 트레이 메뉴)은 확인하지 못한다.
#
# macOS 기본 bash 3.2와 Git Bash에서 돌아가게 쓴다.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) os=windows ;;
  Darwin) os=macos ;;
  *) os=linux ;;
esac

bin="${1:-}"
if [ -z "$bin" ]; then
  if [ -f "$root/target/release/plx.exe" ]; then bin="$root/target/release/plx.exe"; else bin="$root/target/release/plx"; fi
fi
if [ ! -x "$bin" ]; then
  echo "실행 파일이 없다: $bin (먼저 npm run tauri build -- --no-bundle)" >&2
  exit 1
fi
bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"

now_ms() { perl -MTime::HiRes=time -e 'printf("%.0f\n", time * 1000)'; }

plx_count() {
  if [ "$os" = windows ]; then
    tasklist //FI "IMAGENAME eq plx.exe" //NH 2>/dev/null | grep -ci '^plx\.exe' || true
  else
    pgrep -x plx | wc -l | tr -d ' '
  fi
}

# 이 스크립트가 띄운 인스턴스를 모두 끝낸다(시작할 때 다른 인스턴스가 없음을 확인했다).
stop_all() {
  if [ "$os" = windows ]; then
    taskkill //F //IM plx.exe >/dev/null 2>&1 || true
  else
    pkill "${1:--TERM}" -x plx 2>/dev/null || true
  fi
  local i=0
  while [ "$(plx_count)" -gt 0 ]; do
    i=$((i + 1))
    if [ "$i" -gt 100 ]; then
      echo "plx 프로세스가 끝나지 않는다" >&2
      return 1
    fi
    sleep 0.05
  done
}

if [ "$(plx_count)" -gt 0 ]; then
  echo "실행 중인 parallax(plx)가 있다. 트레이에서 종료한 뒤 다시 실행한다." >&2
  exit 1
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/plx-check.XXXXXX")"
log="$work/perf.jsonl"
cleanup() {
  stop_all || true
  rm -rf "$work"
}
trap cleanup EXIT
# 사용자의 blame 캐시와 섞이지 않게 캐시도 임시 디렉터리에 둔다.
export PLX_PERF=1 PLX_PERF_LOG="$log" PLX_CACHE_DIR="$work/cache"

# 기록에 문자열이 나타날 때까지 기다린다(최대 약 15초).
wait_for() {
  local i=0
  while ! LC_ALL=C grep -qF "$1" "$log" 2>/dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 300 ]; then return 1; fi
    sleep 0.05
  done
}

# 기록 한 줄에 문자열들이 모두 있을 때까지 기다린다(최대 약 15초).
# 기능 확인은 기록이 있는지와 그 내용(블록·커밋·구간 수)으로 본다. 기록의 "ok"는 페인트를 쟀는지라 보지 않는다:
# 창이 가려지면 WebView가 그리기를 멈춰(Windows CI에서 본 일) 화면은 바뀌었어도 못 잴 수 있다. 그런 기록은 끝에 알린다.
wait_for_all() { # wait_for_all <문자열...>
  local i=0
  while :; do
    local lines
    lines="$(LC_ALL=C grep -F "$1" "$log" 2>/dev/null || true)"
    for needle in "${@:2}"; do
      lines="$(printf '%s\n' "$lines" | LC_ALL=C grep -F "$needle" || true)"
    done
    if [ -n "$lines" ]; then return 0; fi
    i=$((i + 1))
    if [ "$i" -gt 300 ]; then return 1; fi
    sleep 0.05
  done
}

# 기록에 문자열이 든 줄이 그만큼 쌓일 때까지 기다린다(최대 약 15초).
wait_records() { # wait_records <문자열> <개수>
  local i=0 n
  while :; do
    n="$(LC_ALL=C grep -cF "$1" "$log" 2>/dev/null || true)"
    if [ "${n:-0}" -ge "$2" ]; then return 0; fi
    i=$((i + 1))
    if [ "$i" -gt 300 ]; then return 1; fi
    sleep 0.05
  done
}

wait_count() { # wait_count <기대 개수> — 프로세스 수가 그 값이 될 때까지(최대 약 10초)
  local i=0
  while [ "$(plx_count)" -ne "$1" ]; do
    i=$((i + 1))
    if [ "$i" -gt 200 ]; then return 1; fi
    sleep 0.05
  done
}

passed=0
failed=0
report=""
result() { # result <PASS|FAIL> <설명>
  echo "$1  $2"
  report="$report- $1 $2
"
  if [ "$1" = PASS ]; then passed=$((passed + 1)); else failed=$((failed + 1)); fi
}
check() { # check <설명> <명령...>
  local what="$1"
  shift
  if "$@"; then result PASS "$what"; else result FAIL "$what"; fi
}

cd "$root"
mkdir -p "$work/한글 폴더"
printf '첫 줄\n둘째 줄\n' >"$work/한글 폴더/파일 이름.txt"
# blame할 한글 경로 저장소(커밋 하나 + 커밋 안 한 줄 하나). 이 스크립트가 만든 임시 저장소라 바꿔도 된다.
repo="$work/한글 저장소"
mkdir -p "$repo/소스" "$repo/자료"
printf '하나\n둘\n' >"$repo/소스/파일 이름.rs"
printf '# 설명\n\n이름을 바꿀 문서의 첫 내용\n둘째 줄\n셋째 줄\n' >"$repo/자료/옛 이름.md"
(
  cd "$repo"
  git -c init.defaultBranch=main init -q
  git add .
  git -c user.name=검사 -c user.email=check@example.com -c commit.gpgsign=false commit -q --no-verify -m "첫 커밋"
)
printf '셋(커밋 안 함)\n' >>"$repo/소스/파일 이름.rs"

echo "== $bin ($os)"

# 1. 버전·잘못된 인자 -----------------------------------------------------
version="$("$bin" --version 2>&1 || true)"
check "plx --version → \`$version\`" test "${version#plx }" != "$version"

status=0
error="$("$bin" no-such-file.rs 2>&1 >/dev/null)" || status=$?
usage_error() { [ "$status" -eq 2 ] && [ "${error#*파일이 없다}" != "$error" ]; }
check "없는 파일 → 종료 코드 2, 터미널에 오류 (\`$status\`, \`$error\`)" usage_error
check "잘못된 인자는 상주 인스턴스를 띄우지 않는다" test "$(plx_count)" -eq 0

# 2. 콜드 스타트와 전달 ---------------------------------------------------
PLX_DETACH=0 "$bin" package.json >/dev/null 2>&1 &
check "콜드 스타트 → 첫 화면" wait_for '"kind":"cold"'

start="$(now_ms)"
status=0
"$bin" package.json:2 || status=$?
check "두 번째 호출 → 종료 코드 0, 셸 $(($(now_ms) - start))ms" test "$status" -eq 0
check "두 번째 호출 → 상주 인스턴스가 그림" wait_for '"label":"blame package.json:2"'
check "blame이 끝까지 와서 거터를 그림 (콜드 스타트, package.json)" \
  wait_for_all '"kind":"blame"' '"label":"blame package.json (콜드 스타트)"'
# 얕은 클론은 결과를 캐시하지 않는다(CLAUDE.md "불변 결과는 영구 캐시"). 그런 클론에서 돌리면 이 확인은 건너뛴다.
if [ "$(git rev-parse --is-shallow-repository)" = true ]; then
  result PASS "같은 파일을 다시 열면 캐시에서 (package.json) — 얕은 클론이라 캐시하지 않아 건너뜀"
else
  check "같은 파일을 다시 열면 캐시에서 (package.json)" \
    wait_for_all '"kind":"blame"' '"label":"blame package.json (캐시)"'
fi

status=0
(cd "$repo" && "$bin" "소스/파일 이름.rs:3") || status=$?
check "한글 경로 저장소 파일 blame → 종료 코드 0" test "$status" -eq 0
check "한글 경로 저장소 파일 blame → 거터를 그림 (블록 2: 커밋 1, 미커밋 1)" \
  wait_for_all '"kind":"blame"' '"label":"blame 소스/파일 이름.rs"' '블록 2'
# 작업트리 blame은 파일을 고치면(에이전트·편집기, 커밋 전) 다시 그린다.
sleep 0.5
printf '넷(커밋 안 함)\n' >>"$repo/소스/파일 이름.rs"
check "파일을 고치면 열린 작업트리 blame을 다시 그림" wait_records '"label":"blame 소스/파일 이름.rs"' 2

status=0
(cd "$repo" && "$bin" show HEAD) || status=$?
check "plx show → 종료 코드 0" test "$status" -eq 0
check "커밋 상세가 파일 목록과 diff를 그림 (한글 경로)" \
  wait_for_all '"kind":"commit"' '소스/파일 이름.rs'

# 파일 히스토리: 이름을 바꾼 파일은 옛 이름의 이력이 이어진다(커밋 2개, git log 구간 2개).
# (자료/는 경로 순으로 소스/ 뒤라 위 커밋 상세가 처음 고르는 파일은 그대로 소스/파일 이름.rs다.)
(
  cd "$repo"
  git mv "자료/옛 이름.md" "자료/새 이름.md"
  printf '# 설명\n\n이름을 바꿀 문서의 첫 내용\n둘째 줄을 고쳤다\n셋째 줄\n' >"자료/새 이름.md"
  git add 자료
  git -c user.name=검사 -c user.email=check@example.com -c commit.gpgsign=false commit -q --no-verify -m "이름 바꿈"
)
status=0
(cd "$repo" && "$bin" log "자료/새 이름.md") || status=$?
check "plx log → 종료 코드 0" test "$status" -eq 0
check "파일 히스토리가 rename 앞 구간까지 그림 (커밋 2, 구간 2)" \
  wait_for_all '"kind":"history"' '"label":"히스토리 자료/새 이름.md"' '커밋 2 · 구간 2'
(cd "$repo" && "$bin" log "자료/새 이름.md") || true
check "같은 히스토리를 다시 열면 캐시에서" \
  wait_for_all '"kind":"history"' '"label":"히스토리 자료/새 이름.md (캐시)"'

# 라인 히스토리: 3~4줄은 이름을 바꾼 커밋(4줄을 고침)과 처음 만든 커밋(옛 이름)이 바꿨다.
status=0
(cd "$repo" && "$bin" log -L "3,4:자료/새 이름.md") || status=$?
check "plx log -L → 종료 코드 0" test "$status" -eq 0
check "라인 히스토리가 rename 앞 커밋까지 그림 (커밋 2)" \
  wait_for_all '"kind":"history"' '"label":"라인 히스토리 자료/새 이름.md:3-4"' '커밋 2'

# 커밋 그래프: 인자 없는 plx는 cwd 저장소의 그래프다. 한글 이름 브랜치를 merge해 레인이 둘이다.
(
  cd "$repo"
  author=(-c user.name=검사 -c user.email=check@example.com -c commit.gpgsign=false)
  git checkout -q -b 곁가지
  git "${author[@]}" commit -q --no-verify --allow-empty -m "곁가지 커밋"
  git checkout -q main
  git "${author[@]}" merge -q --no-ff --no-verify -m "곁가지 합침" 곁가지
)
status=0
(cd "$repo/소스" && "$bin") || status=$?
check "plx (인자 없음) → 종료 코드 0" test "$status" -eq 0
check "커밋 그래프가 첫 화면을 그림 (커밋 4, 레인 2)" \
  wait_for_all '"kind":"graph"' '"label":"그래프 한글 저장소"' '커밋 4 · 레인 2'

# 다른 터미널(에이전트)의 커밋이 열린 그래프에 반영된다(.git 감시 → 다시 읽기). 예산은 1초, CI 러너는 느려 2초까지 본다.
sleep 0.5
start="$(now_ms)"
git -C "$repo" -c user.name=검사 -c user.email=check@example.com -c commit.gpgsign=false \
  commit -q --no-verify --allow-empty -m "외부 커밋"
reloaded=0
wait_for_all '"label":"그래프 다시 읽음 한글 저장소"' '커밋 5' || reloaded=$?
elapsed=$(($(now_ms) - start))
reflected() { [ "$reloaded" -eq 0 ] && [ "$elapsed" -le 2000 ]; }
check "외부 커밋 → 열린 그래프가 다시 그림 (커밋 5, ${elapsed}ms, 2초 이내)" reflected

status=0
(cd "$work" && "$bin" "한글 폴더/파일 이름.txt:1") || status=$?
check "한글·공백 경로 전달 → 종료 코드 0" test "$status" -eq 0
check "한글·공백 경로 전달 → 상주 인스턴스가 그림" wait_for '파일 이름.txt:1"'
check "상주 인스턴스는 하나" test "$(plx_count)" -eq 1

# 3. 동시 실행 ------------------------------------------------------------
stop_all
for i in 1 2 3 4 5; do
  PLX_DETACH=0 "$bin" "package.json:$i" >/dev/null 2>&1 &
done
check "동시에 5번 실행 → 상주 인스턴스 하나" wait_count 1

# 4. 강제 종료 뒤 다시 시작 -----------------------------------------------
stop_all -KILL
PLX_DETACH=0 "$bin" package.json:7 >/dev/null 2>&1 &
check "강제 종료 뒤 다시 시작" wait_for '"label":"콜드 스타트 · blame package.json:7"'

# 5. 터미널에서 처음 실행하면 분리 실행 -------------------------------------
stop_all
case "$bin" in
  */debug/*) result PASS "분리 실행: 개발 빌드는 분리하지 않으므로 건너뜀" ;;
  *)
    start="$(now_ms)"
    status=0
    case "$os" in
      # 셸이 넘긴 표준 핸들이 있으면 분리한다.
      windows) "$bin" package.json:8 || status=$? ;;
      # 터미널이 붙어 있어야 분리하므로 script로 가상 터미널을 준다.
      macos) script -q /dev/null "$bin" package.json:8 >/dev/null || status=$? ;;
      linux) script -qec "'$bin' package.json:8" /dev/null >/dev/null || status=$? ;;
    esac
    elapsed=$(($(now_ms) - start))
    returned() { [ "$status" -eq 0 ] && [ "$elapsed" -lt 3000 ]; }
    check "터미널에서 처음 실행 → 셸 복귀 ${elapsed}ms (종료 코드 $status, 3초 이내)" returned
    check "분리된 프로세스가 상주 인스턴스가 됨" wait_for '"label":"콜드 스타트 · blame package.json:8"'
    check "분리 뒤 프로세스는 하나" wait_count 1
    ;;
esac

if LC_ALL=C grep -qF '"ok":false' "$log"; then
  echo "주의: 페인트를 측정하지 못한 기록이 있다(창이 화면에 그려지지 않는 환경일 수 있다)."
fi

# 6. 지연 ------------------------------------------------------------------
stop_all
# 지금 이 스크립트를 돌리는 bash로 실행한다(macOS CI는 기본 bash 3.2).
latency="$("$BASH" "$root/bench/app-latency.sh" "$bin" 20)" || latency=""
if [ -n "$latency" ]; then
  result PASS "앱 지연 측정"
  echo
  echo "$latency"
else
  result FAIL "앱 지연 측정"
fi

if [ -n "${PLX_CHECK_RECORD:-}" ] && [ -n "$latency" ]; then
  results="$root/bench/results.md"
  section="$work/section.md"
  {
    echo "<!-- app:$PLX_CHECK_RECORD -->"
    echo "### $PLX_CHECK_RECORD ($(date '+%Y-%m-%d'))"
    echo
    echo "$latency"
    echo "<!-- /app:$PLX_CHECK_RECORD -->"
  } >"$section"
  if grep -qF "<!-- app:$PLX_CHECK_RECORD -->" "$results"; then
    awk -v start="<!-- app:$PLX_CHECK_RECORD -->" -v end="<!-- /app:$PLX_CHECK_RECORD -->" -v file="$section" '
      $0 == start { while ((getline line < file) > 0) print line; skip = 1; next }
      $0 == end { skip = 0; next }
      !skip { print }' "$results" >"$work/results.md"
  else
    awk -v anchor="<!-- app:end -->" -v file="$section" '
      $0 == anchor { while ((getline line < file) > 0) print line; print "" }
      { print }' "$results" >"$work/results.md"
  fi
  cp "$work/results.md" "$results"
  echo "기록했다: bench/results.md ($PLX_CHECK_RECORD)"
fi

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### 앱 확인: $os"
    echo
    printf '%s' "$report"
    echo
    echo "$latency"
  } >>"$GITHUB_STEP_SUMMARY"
fi

echo
echo "통과 $passed, 실패 $failed"
[ "$failed" -eq 0 ]
