#!/usr/bin/env bash
# 참조 저장소를 준비한다: 전체 이력 클론 → 고정 태그 체크아웃 → commit-graph(+ changed-path Bloom 필터) 작성.
#
# 사용법: bench/setup.sh
#   저장 위치는 PLX_BENCH_DIR (기본값 bench/repos, .gitignore 대상). 디스크 약 2.5GB.
#   이미 받은 저장소는 다시 받지 않고 태그·설정·commit-graph만 맞춘다.
#
# 바꾸는 것은 이 벤치용 클론뿐이다(설정, commit-graph). 사용자 저장소는 건드리지 않는다.
# macOS 기본 bash 3.2와 Git Bash에서 돌아가게 쓴다.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
dir="${PLX_BENCH_DIR:-$here/repos}"
mkdir -p "$dir"

# 이름 URL 태그. 태그를 바꾸면 results.md의 수치는 모두 다시 잰다.
repos="git https://github.com/git/git.git v2.56.0
vscode https://github.com/microsoft/vscode.git 1.142.0"

echo "$repos" | while read -r name url tag; do
  repo="$dir/$name"
  if [ ! -d "$repo/.git" ]; then
    echo "== $name: 클론 $url"
    git clone --no-checkout "$url" "$repo"
  fi
  if ! git -C "$repo" rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    git -C "$repo" fetch origin "refs/tags/$tag:refs/tags/$tag"
  fi

  # 측정 조건을 플랫폼과 전역 설정에서 떼어 놓는다.
  # - autocrlf: Windows에서도 작업트리 바이트를 다른 플랫폼과 같게 둔다.
  # - gc·maintenance: 측정 중 자동 gc가 commit-graph를 다시 쓰지 않게 한다.
  git -C "$repo" config core.autocrlf false
  git -C "$repo" config core.quotepath false
  git -C "$repo" config gc.auto 0
  git -C "$repo" config maintenance.auto false

  echo "== $name: $tag 체크아웃"
  git -C "$repo" -c advice.detachedHead=false checkout -q --force --detach "$tag"

  echo "== $name: commit-graph + changed-path Bloom 필터 작성"
  git -C "$repo" commit-graph write --reachable --changed-paths

  echo "== $name: $(git -C "$repo" rev-parse --short HEAD), 커밋 $(git -C "$repo" rev-list --count HEAD)개"
done
