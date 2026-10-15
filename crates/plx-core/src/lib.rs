//! parallax 코어 라이브러리.
//!
//! gix·git CLI 래퍼, git 출력 파서, 캐시, 그래프 레인 레이아웃, 인스턴스 간 전달을 둔다.
//! UI(Tauri)에 의존하지 않으며, 모든 모듈은 단위 테스트를 가진다.

pub mod instance;
pub mod time;

/// 코어 크레이트 버전. 앱 정보 표시와 캐시 키 버전에 쓴다.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}
