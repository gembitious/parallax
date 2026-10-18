//! `plx`가 상주 인스턴스에 넘기기 전에 저장소를 열어 보는 확인(`Repo::discover_for_check`)은 저장소 밖의 시스템
//! 설정을 읽지 않는다(Windows에서는 그 자리를 찾으려고 gix가 git을 띄운다). 앱이 쓰는 `Repo::discover`는 읽는다.
//! 시스템 설정 자리를 환경 변수로 바꾸므로 이 파일에는 테스트를 하나만 둔다(테스트 바이너리 하나가 프로세스 하나다).

mod common;

use common::{commit_all, scratch_repo};
use plx_core::repo::Repo;

#[test]
fn the_check_skips_the_system_config() {
    let dir = scratch_repo("check-config");
    std::fs::write(dir.join("a.txt"), "x\n").unwrap();
    commit_all(&dir, "first");
    let system = dir.join("system-gitconfig");
    std::fs::write(&system, "[plx]\n\tmarker = system\n").unwrap();
    // SAFETY: 이 테스트 바이너리의 유일한 테스트이고, 저장소를 만든 git은 끝났다(다른 스레드가 환경을 읽지 않는다).
    unsafe { std::env::set_var("GIT_CONFIG_SYSTEM", &system) };

    let marker = |repo: &Repo| {
        repo.local()
            .config_snapshot()
            .string("plx.marker")
            .map(|value| value.to_string())
    };
    let app = Repo::discover(&dir).unwrap();
    assert_eq!(
        marker(&app).as_deref(),
        Some("system"),
        "앱은 시스템 설정을 읽는다"
    );
    let check = Repo::discover_for_check(&dir.join("a.txt")).unwrap();
    assert_eq!(
        marker(&check),
        None,
        "넘기기 전 확인은 시스템 설정을 읽지 않는다"
    );
    assert_eq!(check.workdir(), app.workdir());
}
