//! 취소할 수 있는 요청(blame, 히스토리)의 등록부와, 요청 하나의 패닉이 앱을 멈추지 않게 하는 도구.
//!
//! 화면은 요청을 열 때 id를 받고, 다른 파일·리비전으로 넘어가면 그 id로 취소한다. 요청 하나가 git을
//! 여러 개 띄울 수 있으므로(히스토리의 rename 구간) 취소 표시는 프로세스와 따로 남기고, 취소하면 붙어 있는
//! git을 모두 끝낸다.

use std::any::Any;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::git::Canceller;

#[derive(Default)]
pub struct Requests {
    next_id: AtomicU64,
    slots: Mutex<HashMap<u64, Slot>>,
}

#[derive(Default)]
struct Slot {
    cancelled: bool,
    /// 이 요청이 띄운 git. 이미 끝난 것도 남지만 끝낸 프로세스를 다시 끝내는 것은 아무 일도 하지 않는다.
    running: Vec<Canceller>,
}

impl Requests {
    pub fn new() -> Self {
        Self::default()
    }

    /// 새 요청 id. 화면은 이 id를 받은 뒤에만 취소할 수 있으므로 "시작 전 취소"도 등록된 요청에만 일어난다.
    pub fn register(&self) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.slots.lock().unwrap().insert(id, Slot::default());
        id
    }

    /// 요청을 멈춘다. 이미 끝났거나 모르는 id면 `false`.
    pub fn cancel(&self, id: u64) -> bool {
        let mut slots = self.slots.lock().unwrap();
        let Some(slot) = slots.get_mut(&id) else {
            return false;
        };
        slot.cancelled = true;
        for canceller in &slot.running {
            canceller.cancel();
        }
        true
    }

    /// 취소됐거나 이미 끝난(모르는) 요청이면 `true`.
    pub fn is_cancelled(&self, id: u64) -> bool {
        self.slots
            .lock()
            .unwrap()
            .get(&id)
            .is_none_or(|s| s.cancelled)
    }

    /// 띄운 git을 요청에 붙인다. 이미 취소된 요청이면 그 git을 바로 끝내고 `false`.
    pub fn attach(&self, id: u64, canceller: Canceller) -> bool {
        let mut slots = self.slots.lock().unwrap();
        match slots.get_mut(&id) {
            Some(slot) if !slot.cancelled => {
                slot.running.push(canceller);
                true
            }
            _ => {
                canceller.cancel();
                false
            }
        }
    }

    /// 요청이 끝났다. 이후 그 id로 취소하면 `false`다.
    pub fn finish(&self, id: u64) {
        self.slots.lock().unwrap().remove(&id);
    }

    /// 끝나지 않은 요청 수.
    pub fn active(&self) -> usize {
        self.slots.lock().unwrap().len()
    }
}

/// 요청 하나를 돌린다. 안에서 패닉하면 그 요청만 `error`(패닉 문장)로 끝낸다.
///
/// 상주 앱이라 패닉이 프로세스를 죽이지 않게 했지만(`panic = "abort"`를 뺐다), 흘려보내는 요청은 끝 이벤트를
/// 받지 못한 화면이 "읽는 중"에 멈춘다. 패닉 위치는 기본 패닉 훅이 표준 오류에 남긴다.
pub fn catch<T>(run: impl FnOnce() -> T, error: impl FnOnce(String) -> T) -> T {
    match panic::catch_unwind(AssertUnwindSafe(run)) {
        Ok(value) => value,
        Err(payload) => error(format!(
            "plx 내부 오류(버그): {}",
            panic_text(payload.as_ref())
        )),
    }
}

/// 패닉 값의 문장(`panic!`·`unwrap`이 남긴 문자열). 문자열이 아니면 그렇다고만 적는다.
fn panic_text(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("알 수 없는 패닉")
}

/// 앱이 끝날 때까지 두는 캐시의 잠금. 앞 요청이 이 잠금을 쥔 채 패닉했으면 내용이 반쯤 바뀌었을 수 있으니
/// 비우고 쓴다(`unwrap`하면 앱을 다시 띄울 때까지 이 캐시를 쓰는 요청이 모두 패닉한다).
pub fn lock_or_reset<T: Default>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        let mut guard = poisoned.into_inner();
        *guard = T::default();
        mutex.clear_poison();
        guard
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_turns_a_panic_into_the_error_value() {
        assert_eq!(catch(|| 1, |_| 2), 1);
        let message = catch(|| -> String { panic!("부서졌다 {}", 7) }, |m| m);
        assert_eq!(message, "plx 내부 오류(버그): 부서졌다 7");
        let message = catch(|| -> String { std::panic::panic_any(3_u8) }, |m| m);
        assert_eq!(message, "plx 내부 오류(버그): 알 수 없는 패닉");
    }

    #[test]
    fn a_poisoned_cache_starts_over() {
        let cache = std::sync::Arc::new(Mutex::new(vec![1, 2]));
        let held = cache.clone();
        let _ = std::thread::spawn(move || {
            let mut guard = held.lock().unwrap();
            guard.push(3);
            panic!("잠금을 쥔 채 패닉");
        })
        .join();
        assert!(cache.is_poisoned());
        assert!(lock_or_reset(&cache).is_empty(), "반쯤 바뀐 내용을 버린다");
        assert!(!cache.is_poisoned());
        lock_or_reset(&cache).push(4);
        assert_eq!(*cache.lock().unwrap(), vec![4]);
    }
}
