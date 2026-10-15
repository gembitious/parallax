#!/usr/bin/env bash
# 앱 지연 측정: 콜드 스타트 1회 + 상주 인스턴스에 plx 호출 N회.
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
    kill "$primary" 2>/dev/null || true
    # 다음 측정이 이 인스턴스에 넘어가지 않게 끝날 때까지 기다린다.
    wait "$primary" 2>/dev/null || true
  fi
  rm -f "$log"
}
trap cleanup EXIT

# 기록에 패턴이 나타날 때까지 기다린다(최대 10초).
wait_for() {
  local i=0
  while ! grep -q "$1" "$log" 2>/dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 1000 ]; then return 1; fi
    sleep 0.01
  done
}

total_of() { sed -n 's/.*"totalMs":\([0-9.]*\).*/\1/p'; }

# 숫자 목록(한 줄에 하나) → "중앙값 p90 최대"
summary() {
  sort -n | awk '{ v[NR] = $1 } END {
    if (NR == 0) { print "- - -"; exit }
    m = v[int((NR + 1) / 2)]; p = v[int(NR * 0.9 + 0.999)]; if (p == "") p = v[NR]
    printf "%.1f %.1f %.1f\n", m, p, v[NR] }'
}

cd "$here"
target="app-latency.sh" # blame 대상: 이 스크립트 자신(항상 있는 파일)

# 콜드 스타트. 분리 실행을 끄고 셸의 자식으로 띄워 끝날 때 정리한다.
PLX_PERF=1 PLX_PERF_LOG="$log" PLX_DETACH=0 "$bin" "$target" >/dev/null 2>&1 &
primary=$!
if ! wait_for '"kind":"cold"'; then
  if kill -0 "$primary" 2>/dev/null; then
    echo "콜드 스타트 기록이 10초 안에 나오지 않았다." >&2
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

# 상주 인스턴스에 호출. 앞 호출이 그려진 뒤 다음 호출을 보낸다.
shell_ms=""
for i in $(seq 1 "$runs"); do
  start="$(now_ms)"
  "$bin" "$target:$i" >/dev/null
  end="$(now_ms)"
  shell_ms="$shell_ms$(awk -v s="$start" -v e="$end" 'BEGIN { printf "%.1f", e - s }')
"
  if ! wait_for "\"label\":\"blame $target:$i\""; then
    echo "$i번째 호출 기록이 나오지 않았다." >&2
    exit 1
  fi
done
read -r warm_med warm_p90 warm_max <<<"$(grep '"kind":"plx"' "$log" | total_of | summary)"
read -r shell_med shell_p90 shell_max <<<"$(printf '%s' "$shell_ms" | summary)"

echo "### 앱 지연: $(uname -srm), $(date '+%Y-%m-%d')"
echo
echo "| 항목 | 값 (ms) | 예산 |"
echo "|---|---|---|"
echo "| 콜드 스타트 → 첫 페인트 | $cold | ≤ 1000 |"
echo "| 상주 인스턴스에 plx 호출 → 대상 뷰 첫 페인트 (중앙값 / p90 / 최대, n=$runs) | $warm_med / $warm_p90 / $warm_max | ≤ 100 |"
echo "| 두 번째 plx 프로세스, 셸에서 본 시간 (중앙값 / p90 / 최대) | $shell_med / $shell_p90 / $shell_max | — |"
