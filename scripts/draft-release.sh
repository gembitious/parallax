#!/usr/bin/env bash
# 태그 빌드의 설치 파일로 GitHub 릴리스 초안을 만든다(CI의 release 작업이 부른다. gh와 GH_TOKEN이 필요하다).
# 초안은 사람이 보고 게시한다. 같은 태그로 다시 돌리면 초안의 파일만 바꾸고(설명은 사람이 고쳤을 수 있어 그대로 둔다),
# 이미 게시한 릴리스는 건드리지 않는다.
#
# 사용법: scripts/draft-release.sh <태그> <내려받은 실행 결과 디렉터리>
#   실행 결과 디렉터리: actions/download-artifact가 실행 결과마다 만든 하위 디렉터리(이름 = 파일 이름) 또는 파일
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "사용법: $0 <태그> <내려받은 실행 결과 디렉터리>" >&2
  exit 2
fi
tag="$1"
downloaded="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(node "$root/scripts/check-version.mjs" "$tag")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
assets="$work/assets"
mkdir "$assets"

# 압축하지 않고 올린 실행 결과는 이름(= 파일 이름)의 디렉터리에 내려온다. 안의 파일 이름은 믿지 않고 디렉터리 이름을 쓴다.
for entry in "$downloaded"/*; do
  if [ -d "$entry" ]; then
    inner=("$entry"/*)
    if [ "${#inner[@]}" -ne 1 ] || [ ! -f "${inner[0]}" ]; then
      echo "실행 결과 하나에 파일이 하나여야 한다: $entry" >&2
      ls -la "$entry" >&2
      exit 1
    fi
    cp "${inner[0]}" "$assets/$(basename "$entry")"
  elif [ -f "$entry" ]; then
    cp "$entry" "$assets/"
  fi
done
cp "$root/scripts/install-macos-shim.sh" "$assets/"

missing=0
for name in \
  "parallax_${version}_x64-setup.exe" \
  "parallax_${version}_x64-portable.zip" \
  "parallax_${version}_aarch64.dmg" \
  "parallax_${version}_x64.dmg" \
  "parallax_${version}_amd64.deb"; do
  if [ ! -s "$assets/$name" ]; then
    echo "릴리스에 넣을 파일이 없다: $name" >&2
    missing=1
  fi
done
[ "$missing" -eq 0 ] || exit 1
(cd "$assets" && sha256sum -- * >SHA256SUMS.txt && cat SHA256SUMS.txt)

# 설명: CHANGELOG.md의 이 버전 절(머리줄 뺌) + 설치 안내. 절이 없으면 초안을 만들지 않는다(적고 다시 낸다).
notes="$work/notes.md"
awk -v head="## $version" '
  $0 == head { inside = 1; found = 1; next }
  inside && /^## / { exit }
  inside { print }
  END { if (!found) exit 1 }
' "$root/CHANGELOG.md" >"$notes" || {
  echo "CHANGELOG.md에 '## $version' 절이 없다" >&2
  exit 1
}
sed "s/{{VERSION}}/$version/g" "$root/.github/release-notes.md" >>"$notes"

if draft="$(gh release view "$tag" --json isDraft --jq .isDraft 2>/dev/null)"; then
  if [ "$draft" != "true" ]; then
    echo "$tag 릴리스는 이미 게시했다. 파일을 바꾸지 않는다(새 버전으로 다시 낸다)." >&2
    exit 1
  fi
  gh release upload "$tag" "$assets"/* --clobber
  echo "릴리스 초안의 파일을 바꿨다: $tag"
else
  gh release create "$tag" "$assets"/* --draft --verify-tag --title "parallax $version" \
    --notes-file "$notes" --generate-notes
  echo "릴리스 초안을 만들었다: $tag"
fi
