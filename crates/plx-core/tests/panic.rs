//! 요청 안에서 패닉해도 그 요청만 오류 이벤트 하나로 끝나고, 화면이 끝 이벤트를 기다리며 멈추지 않는다.
//! 패닉은 이벤트를 받는 쪽(`on_event`)에서 일으킨다: 요청을 처리하는 코드 한가운데서 나는 패닉과 같은 길로
//! 풀려 나온다(요청이 띄운 git도 그 길에서 끝난다).

mod common;

use std::sync::mpsc;
use std::time::Duration;

use common::history;
use plx_core::blame_session::{SessionEvent, Sessions};
use plx_core::graph::{GraphEvent, GraphOptions, GraphSessions};
use plx_core::history::{HistoryEvent, HistorySessions, LineRange};
use plx_core::repo::RepoCache;

const BUG: &str = "plx 내부 오류(버그): 화면이 사라졌다";

/// 첫 이벤트에서 패닉하고, 그 뒤로 받은 이벤트를 모은다.
fn panics_first<E>(events: &mut Vec<E>, event: E) -> bool {
    let first = events.is_empty();
    events.push(event);
    if first {
        panic!("화면이 사라졌다");
    }
    true
}

#[test]
fn blame_ends_with_an_error() {
    let sessions = Sessions::new();
    let id = sessions.register();
    let mut events = Vec::new();
    sessions.run(
        id,
        &RepoCache::new(),
        history().join("src/lib.rs"),
        None,
        |event| panics_first(&mut events, event),
    );
    assert!(matches!(events[0], SessionEvent::Meta(_)), "{events:?}");
    let [_, SessionEvent::Error { message }] = &events[..] else {
        panic!("meta 다음에 오류 하나로 끝나야 한다: {events:?}");
    };
    assert_eq!(message, BUG);
    assert_eq!(sessions.active(), 0, "끝난 요청은 지운다");
    assert!(!sessions.cancel(id));
}

#[test]
fn history_ends_with_an_error() {
    let sessions = HistorySessions::new();
    let repos = RepoCache::new();
    let id = sessions.register();
    let mut events = Vec::new();
    sessions.run(id, &repos, history().join("src/lib.rs"), None, |event| {
        panics_first(&mut events, event)
    });
    let [HistoryEvent::Meta(_), HistoryEvent::Error { message }] = &events[..] else {
        panic!("meta 다음에 오류 하나로 끝나야 한다: {events:?}");
    };
    assert_eq!(message, BUG);

    let id = sessions.register();
    let mut events = Vec::new();
    let lines = LineRange { start: 1, end: 2 };
    sessions.run_lines(
        id,
        &repos,
        history().join("src/lib.rs"),
        None,
        lines,
        |event| panics_first(&mut events, event),
    );
    let [HistoryEvent::Meta(_), HistoryEvent::Error { message }] = &events[..] else {
        panic!("라인 히스토리도 meta 다음에 오류로 끝나야 한다: {events:?}");
    };
    assert_eq!(message, BUG);
    assert_eq!(sessions.active(), 0, "끝난 요청은 지운다");
}

#[test]
fn graph_ends_with_an_error() {
    let sessions = GraphSessions::new();
    let (tx, rx) = mpsc::channel();
    let mut first = true;
    let id = sessions
        .open(
            &RepoCache::new(),
            &history(),
            GraphOptions::default(),
            move |event| {
                let _ = tx.send(event);
                if std::mem::take(&mut first) {
                    panic!("화면이 사라졌다");
                }
                true
            },
        )
        .unwrap();
    let mut events = Vec::new();
    while let Ok(event) = rx.recv_timeout(Duration::from_secs(30)) {
        let end = matches!(
            event,
            GraphEvent::Done { .. } | GraphEvent::Error { .. } | GraphEvent::Cancelled
        );
        events.push(event);
        if end {
            break;
        }
    }
    let [GraphEvent::Meta(_), GraphEvent::Error { message }] = &events[..] else {
        panic!("meta 다음에 오류 하나로 끝나야 한다: {events:?}");
    };
    assert_eq!(message, BUG);
    // 끝난 그래프의 행 읽기·닫기는 그대로 된다(잠금이 오염되지 않았다).
    assert!(sessions.rows(id, 0, 10).is_ok());
    assert!(sessions.close(id));
}
