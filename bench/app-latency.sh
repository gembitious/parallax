#!/usr/bin/env bash
# 앱 지연 측정: 콜드 스타트 1회 + 상주 인스턴스에 plx 호출 N회, 그 호출들의 blame 첫 줄 표시,
# 커밋 상세(plx show), 파일 히스토리(plx log) 첫 50개 표시, 커밋 그래프(인자 없는 plx) 첫 화면 표시.
# blame은 저장소의 파일 N/2개를, 히스토리는 그중 5개를 두 번씩 연다. 처음은 git, 두 번째는 캐시 히트다(캐시는 임시 디렉터리).
#
# 릴리스 빌드로 잰다. 개발 빌드 수치는 성능 예산 판단에 쓰지 않는다.
#   npm run tauri build -- --no-bundle
#
# 사용법: bench/app-latency.sh [plx 실행 파일] [반복 횟수]
#   실행 파일  기본값 target/release/plx(.exe)
#   반복 횟수  기본값 20
#
# 시간은 앱이 직접 잰다(PLX_PERF=1, PLX_PERF_LOG). 기준점은 OS가 plx 프로세스를 만든 시각이고
# 끝점은 대상 뷰가 화면에 그려진 시각이다. 셸이 plx를 띄우는 비용은 빠지므로,
# 셸에서 본 두 번째 plx 프로세스 시간(터미널이 막히는 시간)을 따로 함께 잰다.
# macOS 기본 bash 3.2와 Git Bash에서 돌아가게 쓴다.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
bin="${1:-}"
if [ -z "$bin" ]; then
  if [ -f "$root/target/release/plx.exe" ]; then
    bin="$root/target/release/plx.exe"
  else
    bin="$root/target/release/plx"
  fi
fi
runs="${2:-20}"
if [ ! -x "$bin" ]; then
  echo "실행 파일이 없다: $bin (먼저 npm run tauri build -- --no-bundle)" >&2
  exit 1
fi
# 아래에서 작업 디렉터리를 옮기므로 절대 경로로 바꿔 둔다.
bin="$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")"

now_ms() { perl -MTime::HiRes=time -e 'printf("%.3f\n", time * 1000)'; }

log="$(mktemp "${TMPDIR:-/tmp}/plx-perf.XXXXXX")"
primary=""
cleanup() {
  if [ -n "$primary" ]; then
    # Git Bash의 kill은 네이티브 Windows 프로세스를 끝내지 못할 수 있어 Windows PID로 끝낸다.
    if [ -r "/proc/$primary/winpid" ]; then
      taskkill //F //PID "$(cat "/proc/$primary/winpid")" >/dev/null 2>&1 || true
    else
      kill "$primary" 2>/dev/null || true
    fi
    # 다음 측정이 이 인스턴스에 넘어가지 않게 끝날 때까지 기다린다.
    wait "$primary" 2>/dev/null || true
  fi
  rm -rf "$log" "$log.files" "$log.logs" "$log.cache"
}
trap cleanup EXIT

# 기록에 문자열이 나타날 때까지 기다린다(최대 약 15초).
wait_for() {
  local i=0
  while ! LC_ALL=C grep -qF "$1" "$log" 2>/dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 1500 ]; then return 1; fi
    sleep 0.01
  done
}

total_of() { sed -n 's/.*"totalMs":\([0-9.]*\).*/\1/p'; }

# 기록에 문자열이 든 줄이 n개가 될 때까지 기다린다(최대 약 15초).
wait_lines() { # wait_lines <문자열> <개수>
  local i=0
  while [ "$(LC_ALL=C grep -cF "$1" "$log" 2>/dev/null || true)" -lt "$2" ]; do
    i=$((i + 1))
    if [ "$i" -gt 1500 ]; then return 1; fi
    sleep 0.01
  done
}

# 숫자 목록(한 줄에 하나) → "중앙값 p90 최대"
summary() {
  sort -n | awk '{ v[NR] = $1 } END {
    if (NR == 0) { print "- - -"; exit }
    m = v[int((NR + 1) / 2)]; p = v[int(NR * 0.9 + 0.999)]; if (p == "") p = v[NR]
    printf "%.1f %.1f %.1f\n", m, p, v[NR] }'
}

cd "$root"
target="bench/app-latency.sh" # 콜드 스타트 대상: 이 스크립트 자신(항상 있는 파일)
# 사용자 캐시와 섞이지 않게, 그리고 처음 여는 파일은 반드시 git blame을 돌게 빈 캐시로 시작한다.
cache_dir="$log.cache"
half=$(((runs + 1) / 2))
files="$log.files"
git -c core.quotepath=false ls-files -- '*.rs' '*.ts' '*.tsx' '*.sh' '*.md' '*.toml' |
  LC_ALL=C grep -vxF "$target" | head -n "$half" >"$files"
if [ "$(wc -l <"$files" | tr -d ' ')" -lt "$half" ]; then
  echo "blame할 파일이 ${half}개보다 적다($root)" >&2
  exit 1
fi

# 콜드 스타트. 분리 실행을 끄고 셸의 자식으로 띄워 끝날 때 정리한다.
PLX_PERF=1 PLX_PERF_LOG="$log" PLX_CACHE_DIR="$cache_dir" PLX_DETACH=0 "$bin" "$target" >/dev/null 2>&1 &
primary=$!
if ! wait_for '"kind":"cold"'; then
  if kill -0 "$primary" 2>/dev/null; then
    echo "콜드 스타트 기록이 15초 안에 나오지 않았다." >&2
  else
    status=0
    wait "$primary" || status=$?
    primary=""
    if [ "$status" -eq 0 ]; then
      echo "이미 실행 중인 parallax가 있다(호출이 그쪽으로 넘어갔다). 트레이에서 종료한 뒤 다시 실행한다." >&2
    else
      echo "plx가 바로 끝났다(종료 코드 $status): $bin" >&2
    fi
  fi
  exit 1
fi
cold="$(grep '"kind":"cold"' "$log" | total_of | awk '{ printf "%.1f", $1 }')"

# 상주 인스턴스에 호출. 앞 호출이 그려지고 blame이 끝난 뒤 다음 호출을 보낸다
# (먼저 보내면 앞 blame이 취소되어 기록이 없다). 두 번째 바퀴는 줄 번호를 바꿔 기록 이름을 구분한다.
shell_ms=""
calls=0
for pass in 1 2; do
  while IFS= read -r file; do
    calls=$((calls + 1))
    start="$(now_ms)"
    "$bin" "$file:$pass" >/dev/null
    end="$(now_ms)"
    shell_ms="$shell_ms$(awk -v s="$start" -v e="$end" 'BEGIN { printf "%.1f", e - s }')
"
    if ! wait_for "\"label\":\"blame $file:$pass\""; then
      echo "$file:$pass 호출 기록이 나오지 않았다." >&2
      exit 1
    fi
    if ! wait_lines '"kind":"blame"' $((calls + 1)); then
      echo "$file:$pass 호출의 blame 기록이 나오지 않았다." >&2
      exit 1
    fi
  done <"$files"
done
read -r warm_med warm_p90 warm_max <<<"$(grep '"kind":"plx"' "$log" | total_of | summary)"
read -r shell_med shell_p90 shell_max <<<"$(printf '%s' "$shell_ms" | summary)"
# 커밋 상세: HEAD를 몇 번 연다(CI는 얕은 클론이라 커밋이 하나뿐이다). 매번 새 화면이라 gix 비교부터 다시 한다.
shows=5
for i in $(seq 1 "$shows"); do
  "$bin" show HEAD >/dev/null
  if ! wait_lines '"kind":"commit"' "$i"; then
    echo "${i}번째 plx show의 커밋 상세 기록이 나오지 않았다." >&2
    exit 1
  fi
done
read -r commit_med commit_p90 commit_max <<<"$(LC_ALL=C grep -F '"kind":"commit"' "$log" | total_of | summary)"

# 파일 히스토리: 파일 5개를 두 번씩 연다(두 번째는 캐시). CI는 얕은 클론이라 커밋이 하나씩이다.
head -n 5 "$files" >"$log.logs"
logs=0
for pass in 1 2; do
  while IFS= read -r file; do
    logs=$((logs + 1))
    "$bin" log "$file" >/dev/null
    if ! wait_lines '"kind":"history"' "$logs"; then
      echo "plx log $file 의 히스토리 기록이 나오지 않았다." >&2
      exit 1
    fi
  done <"$log.logs"
done
history_all="$(LC_ALL=C grep -F '"kind":"history"' "$log" || true)"
git_history="$(printf '%s\n' "$history_all" | LC_ALL=C grep -vF '(캐시)' || true)"
cached_history="$(printf '%s\n' "$history_all" | LC_ALL=C grep -F '(캐시)' || true)"
read -r hist_med hist_p90 hist_max <<<"$(printf '%s\n' "$git_history" | total_of | summary)"
read -r hcache_med hcache_p90 hcache_max <<<"$(printf '%s\n' "$cached_history" | total_of | summary)"
git_page="$(printf '%s\n' "$git_history" | sed -n 's/.*git 50개째 \([0-9.]*\).*/\1/p' | summary | awk '{ print $1 }')"

# 커밋 그래프: 인자 없는 plx(이 저장소의 그래프)를 몇 번 연다. 매번 새 화면이라 DAG부터 다시 읽는다.
graphs=5
for i in $(seq 1 "$graphs"); do
  "$bin" >/dev/null
  if ! wait_lines '"kind":"graph"' "$i"; then
    echo "${i}번째 plx(그래프)의 기록이 나오지 않았다." >&2
    exit 1
  fi
done
read -r graph_med graph_p90 graph_max <<<"$(LC_ALL=C grep -F '"kind":"graph"' "$log" | total_of | summary)"
graph_rows="$(LC_ALL=C grep -F '"kind":"graph"' "$log" | head -n 1 | sed -n 's/.*"detail":"커밋 \([0-9]*\).*/\1/p')"

warm_blame="$(LC_ALL=C grep -F '"kind":"blame"' "$log" | LC_ALL=C grep -vF '(콜드 스타트)' || true)"
git_blame="$(printf '%s\n' "$warm_blame" | LC_ALL=C grep -vF '(캐시)' || true)"
cached_blame="$(printf '%s\n' "$warm_blame" | LC_ALL=C grep -F '(캐시)' || true)"
count_of() { printf '%s' "$1" | LC_ALL=C grep -c . || true; }
read -r blame_med blame_p90 blame_max <<<"$(printf '%s\n' "$git_blame" | total_of | summary)"
read -r cache_med cache_p90 cache_max <<<"$(printf '%s\n' "$cached_blame" | total_of | summary)"
git_first="$(printf '%s\n' "$git_blame" | sed -n 's/.*git 첫 출력 \([0-9.]*\).*/\1/p' | summary | awk '{ print $1 }')"

unpainted="$(LC_ALL=C grep -c '"ok":false' "$log" || true)"
# 콜드 스타트 구간: "프로세스 39 · Tauri·WebView 275 · …"(구간 이름 뒤에 바로 "ms"가 오는 항목만 고른다)
phases_of() {
  grep -o '"label":"[^"]*","ms":[0-9.]*' | sed 's/"label":"\([^"]*\)","ms":\([0-9.]*\)/\1|\2/' |
    awk -F'|' 'NF == 2 { printf "%s%s %.1f", sep, $1, $2; sep = " · " } END { print "" }'
}
# 여러 기록의 구간별 중앙값(구간은 처음 나온 순서로). macOS awk에는 정렬이 없어 구간마다 삽입 정렬한다.
phase_medians() {
  grep -o '"label":"[^"]*","ms":[0-9.]*' | sed 's/"label":"\([^"]*\)","ms":\([0-9.]*\)/\1|\2/' |
    awk -F'|' 'NF == 2 {
        if (!($1 in count)) names[++n] = $1
        v[$1, ++count[$1]] = $2 + 0
      }
      END {
        for (i = 1; i <= n; i++) {
          l = names[i]; k = count[l]
          for (a = 2; a <= k; a++) {
            x = v[l, a]
            for (b = a - 1; b >= 1 && v[l, b] > x; b--) v[l, b + 1] = v[l, b]
            v[l, b + 1] = x
          }
          printf "%s%s %.1f", sep, l, v[l, int((k + 1) / 2)]; sep = " · "
        }
        print ""
      }'
}
cold_phases="$(grep '"kind":"cold"' "$log" | head -n 1 | phases_of)"
cold_blame="$(LC_ALL=C grep -F '(콜드 스타트)' "$log" | LC_ALL=C grep -F '"kind":"blame"' | head -n 1 | phases_of)"
warm_phases="$(LC_ALL=C grep -F '"kind":"plx"' "$log" | phase_medians)"
# 기록 가운데 가장 느린 것: "합계 · 구간 — 이름 (설명)". 가끔 한 번만 크게 느린 것(p90 = 최대)이 어느 구간인지 본다.
# 기록의 열쇠는 이름 순이다(kind 다음이 label).
slowest() {
  local line record detail
  line="$(awk '{ if (match($0, /"totalMs":[0-9.]*/)) print substr($0, RSTART + 10, RLENGTH - 10) "\t" $0 }' |
    LC_ALL=C sort -n | tail -n 1)"
  if [ -z "$line" ]; then
    echo "-"
    return
  fi
  record="$(printf '%s\n' "$line" | cut -f 2-)"
  detail="$(printf '%s\n' "$record" | sed -n 's/.*"detail":"\([^"]*\)".*/\1/p')"
  printf '%s · %s — %s%s\n' "$(printf '%s\n' "$line" | cut -f 1 | awk '{ printf "%.1f", $1 }')" \
    "$(printf '%s\n' "$record" | phases_of)" \
    "$(printf '%s\n' "$record" | sed -n 's/.*"kind":"[^"]*","label":"\([^"]*\)".*/\1/p')" \
    "${detail:+ ($detail)}"
}
echo "- 환경: $(uname -srm), $(date '+%Y-%m-%d'), $("$bin" --version 2>/dev/null || echo plx)"
if [ "$unpainted" -gt 0 ]; then
  echo "- 주의: 페인트를 측정하지 못한 기록 ${unpainted}건(창이 화면에 그려지지 않는 환경). 그 기록은 렌더까지만 잰 값이다."
fi
echo
echo "| 항목 | 값 (ms) | 예산 |"
echo "|---|---|---|"
echo "| 콜드 스타트 → 첫 페인트 | $cold | ≤ 1000 |"
echo "| 상주 인스턴스에 plx 호출 → 대상 뷰 첫 페인트 (중앙값 / p90 / 최대, n=$((calls + shows + logs + graphs))) | $warm_med / $warm_p90 / $warm_max | ≤ 100 |"
echo "| 두 번째 plx 프로세스, 셸에서 본 시간 (중앙값 / p90 / 최대) | $shell_med / $shell_p90 / $shell_max | — |"
echo "| blame 첫 줄 표시, 처음 여는 파일 (n=$(count_of "$git_blame")) | $blame_med / $blame_p90 / $blame_max | ≤ git 첫 출력 + 30 (git 첫 출력 중앙값 $git_first) |"
echo "| blame 첫 줄 표시, 다시 여는 파일·캐시 (n=$(count_of "$cached_blame")) | $cache_med / $cache_p90 / $cache_max | ≤ 16 |"
echo "| 커밋 상세 → 파일 목록 + 첫 diff 표시 (HEAD, n=$shows) | $commit_med / $commit_p90 / $commit_max | ≤ 50 |"
echo "| 파일 히스토리 첫 50개(적으면 전부) 표시, 처음 여는 파일 (n=$(count_of "$git_history")) | $hist_med / $hist_p90 / $hist_max | ≤ git 50개째 + 30 (git 중앙값 $git_page) |"
echo "| 파일 히스토리, 다시 여는 파일·캐시 (n=$(count_of "$cached_history")) | $hcache_med / $hcache_p90 / $hcache_max | ≤ 16 |"
echo "| 커밋 그래프 첫 화면 표시 (이 저장소 커밋 ${graph_rows:-?}개, n=$graphs) | $graph_med / $graph_p90 / $graph_max | ≤ 150 |"
echo
echo "- 콜드 스타트 구간(ms): $cold_phases"
echo "- 콜드 스타트의 blame(ms): $cold_blame"
echo "- 상주 인스턴스에 plx 호출, 구간별 중앙값(ms): $warm_phases"
echo "- 가장 느린 기록(ms, 합계 · 구간 (설명)):"
echo "  - plx 호출: $(LC_ALL=C grep -F '"kind":"plx"' "$log" | slowest)"
echo "  - blame, 처음 여는 파일: $(printf '%s\n' "$git_blame" | slowest)"
echo "  - blame, 캐시: $(printf '%s\n' "$cached_blame" | slowest)"
echo "  - 커밋 상세: $(LC_ALL=C grep -F '"kind":"commit"' "$log" | slowest)"
echo "  - 파일 히스토리, 처음 여는 파일: $(printf '%s\n' "$git_history" | slowest)"
echo "  - 커밋 그래프: $(LC_ALL=C grep -F '"kind":"graph"' "$log" | slowest)"
