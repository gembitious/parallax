# parallax

로컬 git 이력을 생각하는 속도로 보는 **읽기 전용** 데스크탑 뷰어(이름은 가칭).
터미널에서 `plx <파일>:<줄>` 한 번으로 blame을 열고, 줄 → 커밋 → 부모 리비전 blame → 파일·라인 히스토리 →
커밋 그래프를 키보드로 오간다. 편집·스테이징·커밋처럼 저장소를 바꾸는 일은 하지 않는다.

Tauri 2 + Rust(`gix`, git CLI) + React. Windows와 macOS를 1급으로 지원한다(Linux도 CI에서 확인한다).

**상태:** 개발 중. M0~M5 기능은 구현했고 Windows·macOS 화면 확인이 남았다([docs/MILESTONES.md](docs/MILESTONES.md)).

## 빌드

Node(Volta로 고정), Rust(rustup이 `rust-toolchain.toml`로 맞춘다), git이 필요하다.

```sh
npm install
npm run tauri dev                    # 개발
npm run tauri build -- --no-bundle   # 릴리스 실행 파일: target/release/plx
cargo test --workspace && npm test   # 테스트
```

명령줄 사용법·키맵·설계 결정은 [CLAUDE.md](CLAUDE.md), 성능 측정은 [bench/results.md](bench/results.md)에 있다.

## 라이선스

[MIT](LICENSE-MIT) 또는 [Apache-2.0](LICENSE-APACHE) 가운데 하나를 골라 쓸 수 있다.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you,
as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
(따로 밝히지 않는 한 이 저장소에 보낸 기여도 위의 두 라이선스로 낸 것으로 본다.)

퍼지 매칭의 점수 방식(`crates/plx-core/src/fuzzy.rs`)은 [fzf](https://github.com/junegunn/fzf)(MIT)를 따랐고,
fzf의 저작권 고지는 그 파일 맨 위에 있다. 앱에 들어 있는 다른 프로젝트의 저작권 고지는 릴리스 빌드가 모아
(`npm run notices`) 앱의 명령 팔레트 → "오픈 소스 라이선스"에서 보인다.
