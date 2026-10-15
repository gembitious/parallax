#!/bin/sh
# macOS: PATH에 plx shim을 설치한다.
#
# shim은 앱 번들 안의 실행 파일을 직접 exec한다. `open -a`로는 실행 중인 앱에
# argv와 cwd를 넘길 수 없기 때문이다.
#
# 사용법: scripts/install-macos-shim.sh [앱 경로] [설치 디렉터리]
#   앱 경로       기본값 /Applications/parallax.app, 없으면 ~/Applications/parallax.app
#                 개발 중에는 target/release/bundle/macos/parallax.app
#   설치 디렉터리 기본값 /usr/local/bin (쓸 수 있을 때), 아니면 ~/.local/bin
set -eu

app="${1:-}"
if [ -z "$app" ]; then
  for candidate in /Applications/parallax.app "$HOME/Applications/parallax.app"; do
    if [ -d "$candidate" ]; then
      app="$candidate"
      break
    fi
  done
fi
if [ -z "$app" ]; then
  echo "parallax.app을 찾지 못했다. 앱 경로를 첫 인자로 준다." >&2
  exit 1
fi
if [ ! -d "$app" ]; then
  echo "앱 경로가 없다: $app" >&2
  exit 1
fi
app="$(cd "$app" && pwd -P)"
bin="$app/Contents/MacOS/plx"
if [ ! -x "$bin" ]; then
  echo "실행 파일이 없다: $bin" >&2
  exit 1
fi
case "$bin" in
  *"'"*)
    echo "앱 경로에 작은따옴표가 있으면 shim을 만들 수 없다: $bin" >&2
    exit 1
    ;;
esac

dir="${2:-}"
if [ -z "$dir" ]; then
  if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
    dir=/usr/local/bin
  else
    dir="$HOME/.local/bin"
  fi
fi
mkdir -p "$dir"

shim="$dir/plx"
tmp="$shim.tmp.$$"
cat >"$tmp" <<EOF
#!/bin/sh
# parallax plx shim (scripts/install-macos-shim.sh가 만들었다)
exec '$bin' "\$@"
EOF
chmod 755 "$tmp"
mv -f "$tmp" "$shim"
echo "설치했다: $shim -> $bin"

case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "주의: $dir가 PATH에 없다. 셸 설정에 추가한다: export PATH=\"$dir:\$PATH\"" ;;
esac
