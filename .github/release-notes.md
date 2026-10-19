서명하지 않은 빌드다(코드 서명 인증서가 아직 없다). 처음 실행할 때 OS가 막거나 경고하면 아래처럼 연다.

## macOS (Apple Silicon)

1. `parallax_{{VERSION}}_aarch64.dmg`를 열어 `parallax.app`을 응용 프로그램(Applications) 폴더로 끈다.
2. 터미널에서 내려받은 파일 표시를 지운다(지우지 않으면 "Apple이 확인할 수 없다"며 열지 않는다):
   `xattr -dr com.apple.quarantine /Applications/parallax.app`
3. `install-macos-shim.sh`를 내려받아 `sh install-macos-shim.sh`로 `plx` 명령을 설치한다
   (`/usr/local/bin/plx`, 쓸 수 없으면 `~/.local/bin/plx`).

Intel 맥용 빌드는 없다(소스에서 빌드한다: README의 "빌드").

## Windows (x64)

- **설치:** `parallax_{{VERSION}}_x64-setup.exe`. 관리자 권한 없이 `%LOCALAPPDATA%\parallax`에 설치하고 시작 메뉴에
  parallax를 둔다. `plx` 명령은 그 폴더를 PATH에 넣어 쓴다(Git Bash: `~/.bashrc`에
  `export PATH="$LOCALAPPDATA/parallax:$PATH"`).
- **설치 없이:** `parallax_{{VERSION}}_x64-portable.zip`을 풀어 `plx.exe`를 PATH에 있는 폴더에 둔다.
  풀기 전에 zip 파일의 속성에서 "차단 해제"를 고르면 처음 실행할 때 경고가 뜨지 않는다.

SmartScreen이 "Windows의 PC 보호" 창을 띄우면 "추가 정보" → "실행"을 누른다. WebView2 런타임이 필요하다
(Windows 11과 최신 Windows 10에는 들어 있고, 설치 프로그램은 없으면 받아 준다).

## Linux (x64, deb)

`sudo apt install ./parallax_{{VERSION}}_amd64.deb` → `/usr/bin/plx`

## 확인

`plx --version` → `plx {{VERSION}}`. 파일의 SHA-256은 `SHA256SUMS.txt`에 있다.
