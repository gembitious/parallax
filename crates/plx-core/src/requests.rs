//! 취소할 수 있는 요청(blame, 히스토리)의 등록부.
//!
//! 화면은 요청을 열 때 id를 받고, 다른 파일·리비전으로 넘어가면 그 id로 취소한다. 요청 하나가 git을
//! 여러 개 띄울 수 있으므로(히스토리의 rename 구간) 취소 표시는 프로세스와 따로 남기고, 취소하면 붙어 있는
//! git을 모두 끝낸다.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

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
