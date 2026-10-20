#!/usr/bin/env bash
# 릴리스 빌드(`npm run tauri build -- --bundles …`)가 만든 설치 파일을 배포 이름으로 한 디렉터리에 모은다.
# CI가 실행 결과(Artifacts)에 올리고, 태그 빌드에서는 그대로 릴리스 초안에 붙인다.
#   Windows  parallax_<버전>_x64-setup.exe (NSIS), parallax_<버전>_x64-portable.zip (plx.exe + 라이선스)
#   macOS    parallax_<버전>_aarch64.dmg (CI 러너의 아키텍처)
#   Linux    parallax_<버전>_amd64.deb
# GITHUB_OUTPUT이 있으면 모은 파일의 경로를 `installer`(설치 파일)와 `portable`(Windows 압축 파일)로 쓴다.
#
# 사용법: scripts/collect-dist.sh <출력 디렉터리>
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "사용법: $0 <출력 디렉터리>" >&2
  exit 2
fi
root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$1"
out="$(cd "$1" && pwd)"
cd "$root"

version="$(node -p 'require("./src-tauri/tauri.conf.json").version')"
bundle=target/release/bundle

# 이번 버전의 파일이 꼭 하나여야 한다(지난 빌드가 남긴 다른 버전은 보지 않는다).
one() {
  local matches=()
  local f
  for f in "$@"; do
    [ -e "$f" ] && matches+=("$f")
  done
  if [ "${#matches[@]}" -ne 1 ]; then
    echo "설치 파일을 하나로 정하지 못했다(${#matches[@]}개): $*" >&2
    exit 1
  fi
  printf '%s\n' "${matches[0]}"
}

installer=""
portable=""
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    installer="$(one "$bundle/nsis/parallax_${version}_"*-setup.exe)"

    # 설치 없이 쓰는 실행 파일. 압축은 Windows에 늘 있는 PowerShell로 한다(Git Bash에는 zip이 없다).
    portable="$out/parallax_${version}_x64-portable.zip"
    stage="$(mktemp -d)"
    cp target/release/plx.exe LICENSE-MIT LICENSE-APACHE "$stage/"
    rm -f "$portable"
    powershell.exe -NoProfile -NonInteractive -Command \
      "Compress-Archive -Path '$(cygpath -w "$stage")\\*' -DestinationPath '$(cygpath -w "$portable")'"
    rm -rf "$stage"
    ;;
  Darwin)
    installer="$(one "$bundle/dmg/parallax_${version}_"*.dmg)"
    ;;
  Linux)
    installer="$(one "$bundle/deb/parallax_${version}_"*.deb)"
    ;;
  *)
    echo "모르는 플랫폼: $(uname -s)" >&2
    exit 1
    ;;
esac

cp "$installer" "$out/"
installer="$out/$(basename "$installer")"
ls -l "$out"

# 다음 단계(upload-artifact, Node)가 읽을 경로. Git Bash의 /d/a/…는 Windows에서 D:/a/…로 바꾼다.
native() {
  if [ -n "$1" ] && command -v cygpath >/dev/null 2>&1; then
    cygpath -m "$1"
  else
    printf '%s\n' "$1"
  fi
}
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  {
    echo "installer=$(native "$installer")"
    echo "portable=$(native "$portable")"
  } >>"$GITHUB_OUTPUT"
fi
