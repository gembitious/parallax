#!/usr/bin/env bash
# git baseline을 재서 bench/results.md에 넣는다. 먼저 bench/setup.sh로 참조 저장소를 준비한다.
#
# 사용법: bench/run.sh [이름] [반복 횟수]
#   이름       results.md에서 이 측정을 구분하는 이름. 기본값 "OS-호스트명". 같은 이름은 덮어쓴다.
#   반복 횟수  기본값 5 (예열 1회는 따로)
#
# 측정 항목은 bench/cases.txt, 측정 방법은 bench/src/main.rs와 results.md 머리말에 있다.
# macOS 기본 bash 3.2와 Git Bash에서 돌아가게 쓴다.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
dir="${PLX_BENCH_DIR:-$here/repos}"
label="${1:-$(uname -s)-$(hostname)}"
runs="${2:-5}"
results="$here/results.md"

if [ ! -d "$dir/git/.git" ] || [ ! -d "$dir/vscode/.git" ]; then
  echo "참조 저장소가 없다: $dir (먼저 bench/setup.sh)" >&2
  exit 1
fi

cargo build --release --quiet -p plx-bench --manifest-path "$root/Cargo.toml"
exe="$root/target/release/plx-bench"
if [ -f "$exe.exe" ]; then exe="$exe.exe"; fi

out="$(mktemp "${TMPDIR:-/tmp}/plx-bench.XXXXXX")"
section="$(mktemp "${TMPDIR:-/tmp}/plx-bench.XXXXXX")"
trap 'rm -f "$out" "$section" "$results.tmp"' EXIT

"$exe" --repos "$dir" --cases "$here/cases.txt" --runs "$runs" >"$out"

{
  echo "<!-- bench:$label -->"
  echo "### $label ($(date '+%Y-%m-%d'))"
  echo
  cat "$out"
  echo "<!-- /bench:$label -->"
} >"$section"

# 같은 이름의 구간이 있으면 바꾸고, 없으면 "git baseline" 절 끝 표시 앞에 붙인다.
if grep -qF "<!-- bench:$label -->" "$results"; then
  awk -v start="<!-- bench:$label -->" -v end="<!-- /bench:$label -->" -v file="$section" '
    $0 == start { while ((getline line < file) > 0) print line; skip = 1; next }
    $0 == end { skip = 0; next }
    !skip { print }' "$results" >"$results.tmp"
else
  awk -v anchor="<!-- bench:baseline-end -->" -v file="$section" '
    $0 == anchor { while ((getline line < file) > 0) print line; print "" }
    { print }' "$results" >"$results.tmp"
fi
mv "$results.tmp" "$results"
echo "갱신했다: $results ($label)"
