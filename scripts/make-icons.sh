#!/usr/bin/env bash
# 앱 아이콘을 src-tauri/icons/source의 SVG에서 다시 만든다(tauri icon이 resvg로 그린다).
#   app.svg           → Windows(icon.ico, Store 로고)·Linux(png)·macOS·Linux 창 아이콘(32x32.png)
#   app-macos.svg     → icon.icns (Apple 아이콘 격자: 여백과 그림자가 있는 판)
#   tray-template.svg → tray-template.png (macOS 메뉴 막대 템플릿 이미지, 36px)
# 결과 파일은 커밋한다(빌드는 다시 만들지 않는다). SVG를 고친 뒤에만 돌린다.
#
# 사용법: scripts/make-icons.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
icons="$root/src-tauri/icons"
src="$icons/source"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# tauri icon은 만든 파일마다 한 줄씩 쓴다. 실패했을 때만 보인다.
icon() {
  local out
  if ! out="$(npm run --silent tauri -- icon "$@" 2>&1)"; then
    printf '%s\n' "$out" >&2
    exit 1
  fi
}

cd "$root"
icon "$src/app.svg" -o "$tmp/app"
icon "$src/app-macos.svg" -o "$tmp/macos"
icon "$src/tray-template.svg" -o "$tmp/tray" -p 36

# 데스크탑 번들이 쓰는 것만 옮긴다(tauri icon이 함께 만드는 android/·ios/·64x64.png는 쓰지 않는다).
(cd "$tmp/app" && cp 32x32.png 128x128.png 128x128@2x.png icon.png icon.ico Square*Logo.png StoreLogo.png "$icons/")
cp "$tmp/macos/icon.icns" "$icons/icon.icns"
cp "$tmp/tray/36x36.png" "$icons/tray-template.png"
echo "아이콘을 다시 만들었다: $icons"
