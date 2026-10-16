//! 앱이 저장소를 열어 둔 채로 사용자가 git 정리(repack·gc·commit-graph write)를 해도 git이 막히지 않아야 한다.
//! 상주 앱은 저장소를 열어 두고 gix는 pack·idx·commit-graph를 메모리 매핑한다. Windows는 다른 프로세스가 연 파일을
//! 지우지 못하는 경우가 있고(Git for Windows는 다시 시도하다 터미널에서 "Unlink of file … failed. Should I try
//! again? (y/n)"을 묻는다), 그러면 옛 pack이 남고 commit-graph를 바꾸지 못한다. windows-latest(Git for Windows 2.56)에서는
//! 지우고 바꾼다(2026-10-16 CI). 매핑하는 방식이나 열어 두는 것을 바꿀 때 이 테스트가 지킨다.

mod common;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use common::{commit_all, git, scratch_repo};
use plx_core::graph::{GraphOptions, GraphSessions};
use plx_core::repo::RepoCache;

fn pack_files(dir: &Path) -> Vec<PathBuf> {
    let mut packs: Vec<PathBuf> = std::fs::read_dir(dir.join(".git/objects/pack"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|x| x == "pack" || x == "idx"))
        .collect();
    packs.sort();
    packs
}

fn stamp(path: &Path) -> (u64, SystemTime) {
    let meta = std::fs::metadata(path).unwrap();
    (meta.len(), meta.modified().unwrap())
}

#[test]
fn an_open_repository_does_not_block_git_maintenance() {
    let dir = scratch_repo("maintenance");
    for i in 0..5 {
        std::fs::write(dir.join("f.txt"), format!("{i}\n")).unwrap();
        commit_all(&dir, &format!("c{i}"));
    }
    git(&dir, &["repack", "-a", "-d", "-q"]);
    git(&dir, &["commit-graph", "write", "--reachable"]);
    let old_packs = pack_files(&dir);
    assert_eq!(old_packs.len(), 2, "{old_packs:?}");

    // 앱처럼 저장소를 열어 pack의 객체를 읽고(blame 카드·커밋 상세), 그래프 화면을 연 채로 둔다.
    let repos = RepoCache::new();
    let repo = repos.for_path(&dir).unwrap();
    let head = repo.resolve_commit("HEAD").unwrap();
    repo.commit_info(head).unwrap();
    assert!(repo.blob_at(head, "f.txt").unwrap().is_some());
    let sessions = GraphSessions::new();
    let (tx, rx) = mpsc::channel();
    let id = sessions
        .open(&repos, &dir, GraphOptions::default(), move |event| {
            tx.send(event).is_ok()
        })
        .unwrap();
    while !rx.recv_timeout(Duration::from_secs(60)).unwrap().is_end() {}

    // 사용자가 커밋하고 정리한다: git은 옛 pack을 지우고 commit-graph를 새 파일로 바꾼다.
    std::fs::write(dir.join("f.txt"), "new\n").unwrap();
    commit_all(&dir, "after");
    let graph = dir.join(".git/objects/info/commit-graph");
    let old_graph = stamp(&graph);
    git(&dir, &["repack", "-a", "-d", "-q"]);
    let left: Vec<&PathBuf> = old_packs.iter().filter(|path| path.exists()).collect();
    assert!(left.is_empty(), "git이 옛 pack을 지우지 못했다: {left:?}");
    // 파일 시각 해상도가 거친 파일 시스템에서도 다른 시각이 되게
    std::thread::sleep(Duration::from_millis(50));
    git(&dir, &["commit-graph", "write", "--reachable"]);
    assert_ne!(
        stamp(&graph),
        old_graph,
        "git이 commit-graph를 바꾸지 못했다"
    );

    // 열어 둔 저장소와 그래프는 그대로 쓸 수 있다(정리 뒤의 객체도 찾는다).
    let new_head = repo.resolve_commit("HEAD").unwrap();
    assert_ne!(new_head, head);
    repo.commit_info(new_head).unwrap();
    assert!(sessions.rows(id, 0, 10).is_ok());
    assert!(sessions.close(id));
}
