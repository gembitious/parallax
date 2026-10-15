//! 측정용 시각.
//!
//! 프로세스 사이(두 번째 `plx` → 상주 인스턴스)로 넘기는 값이라 단조 시계가 아니라 벽시계를 쓴다.

use std::time::{SystemTime, UNIX_EPOCH};

/// 현재 시각. Unix epoch 기준 ms이고 소수점 아래로 µs까지 담는다.
pub fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_ms_is_after_2020() {
        // 2020-01-01T00:00:00Z
        assert!(epoch_ms() > 1_577_836_800_000.0);
    }
}
