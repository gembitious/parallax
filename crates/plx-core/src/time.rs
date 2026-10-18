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

/// 이 프로세스가 만들어진 시각(epoch ms). `main()`보다 앞선 실행 파일 로딩 시간까지 측정에 넣으려고 쓴다.
/// 얻지 못하면 `None`. Linux 값은 clock tick 해상도(보통 10ms)다.
pub fn process_start_ms() -> Option<f64> {
    sys::process_start_ms()
}

#[cfg(target_os = "macos")]
mod sys {
    pub fn process_start_ms() -> Option<f64> {
        // SAFETY: proc_bsdinfo는 POD이고, proc_pidinfo는 넘긴 크기만큼만 쓴다.
        unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            let written = libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast(),
                size,
            );
            (written == size).then(|| {
                info.pbi_start_tvsec as f64 * 1000.0 + info.pbi_start_tvusec as f64 / 1000.0
            })
        }
    }
}

#[cfg(target_os = "linux")]
mod sys {
    pub fn process_start_ms() -> Option<f64> {
        // /proc/self/stat의 22번째 필드가 부팅 후 시작 시각(clock tick)이다.
        // 2번째 필드(comm)에 공백이 있을 수 있어 마지막 ')' 뒤부터 센다. 그 뒤 첫 필드가 3번째다.
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let rest = stat.get(stat.rfind(')')? + 1..)?;
        let start_ticks: f64 = rest.split_whitespace().nth(22 - 3)?.parse().ok()?;
        // SAFETY: sysconf와 clock_gettime은 넘긴 구조체만 쓴다.
        let (ticks_per_sec, uptime_ms) = unsafe {
            let ticks = libc::sysconf(libc::_SC_CLK_TCK);
            let mut now = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            if ticks <= 0 || libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut now) != 0 {
                return None;
            }
            (
                ticks as f64,
                now.tv_sec as f64 * 1000.0 + now.tv_nsec as f64 / 1e6,
            )
        };
        let age_ms = uptime_ms - start_ticks / ticks_per_sec * 1000.0;
        Some(super::epoch_ms() - age_ms)
    }
}

#[cfg(windows)]
mod sys {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    /// 1601-01-01부터 1970-01-01까지의 100ns 단위 수.
    const UNIX_EPOCH_IN_FILETIME: u64 = 116_444_736_000_000_000;

    pub fn process_start_ms() -> Option<f64> {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: 현재 프로세스 의사 핸들과 출력용 FILETIME 네 개만 넘긴다.
        let ok = unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        if ok == 0 {
            return None;
        }
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        Some(ticks.checked_sub(UNIX_EPOCH_IN_FILETIME)? as f64 / 10_000.0)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod sys {
    pub fn process_start_ms() -> Option<f64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_ms_is_after_2020() {
        // 2020-01-01T00:00:00Z
        assert!(epoch_ms() > 1_577_836_800_000.0);
    }

    #[test]
    fn process_started_recently_and_in_the_past() {
        let start = process_start_ms().expect("프로세스 시작 시각");
        let now = epoch_ms();
        // Linux는 tick 해상도(10ms)라 약간의 여유를 둔다.
        assert!(start <= now + 20.0, "start={start} now={now}");
        assert!(
            now - start < 10.0 * 60.0 * 1000.0,
            "테스트 프로세스가 10분 전에 시작했을 리 없다"
        );
    }
}
