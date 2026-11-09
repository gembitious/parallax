// 파일 트리·탭의 변경 표시(M13)를 위한 작업트리 상태 구독. 작업 공간이 저장소마다 하나 둔다.
// 작업트리 파일이 바뀌면(백엔드 감시, 300ms 조용해지면), HEAD·참조가 바뀌면, 창으로 돌아오면(`git add`처럼 인덱스만 바꾼 것)
// worktree_status를 다시 묻는다. 묻는 동안 또 바뀌면 끝난 뒤 한 번만 더 묻는다. 결과가 같으면 다시 그리지 않는다.
import { useEffect, useState } from "react";
import { call } from "../ipc";
import { watchRepo, watchTree } from "../watch";
import { EMPTY_STATUS, indexStatus, type StatusIndex, type WorktreeStatus } from "./status";

const sameStatus = (a: WorktreeStatus, b: WorktreeStatus) =>
  a.head === b.head &&
  a.untrackedOmitted === b.untrackedOmitted &&
  a.files.length === b.files.length &&
  a.files.every((f, i) => f.path === b.files[i].path && f.kind === b.files[i].kind);

/** `repo`(작업트리 경로)의 변경 상태. 저장소가 없거나 아직 모르면 빈 상태다. */
export function useWorktreeStatus(repo: string | null): StatusIndex {
  const [index, setIndex] = useState<StatusIndex>(EMPTY_STATUS);
  useEffect(() => {
    setIndex(EMPTY_STATUS);
    if (repo === null) return;
    let stopped = false;
    let running = false;
    let again = false;
    let last: WorktreeStatus | null = null;
    const refresh = () => {
      if (stopped) return;
      if (running) {
        again = true;
        return;
      }
      running = true;
      call<WorktreeStatus>("worktree_status", { repo })
        .then(
          (status) => {
            if (stopped || (last && sameStatus(last, status))) return;
            last = status;
            setIndex(indexStatus(status));
          },
          (e: unknown) => console.error("작업트리 상태를 읽지 못했다", e),
        )
        .finally(() => {
          running = false;
          if (again) {
            again = false;
            refresh();
          }
        });
    };
    refresh();
    const stopTree = watchTree(repo, refresh);
    const stopRepo = watchRepo(repo, refresh);
    window.addEventListener("focus", refresh);
    return () => {
      stopped = true;
      stopTree();
      stopRepo();
      window.removeEventListener("focus", refresh);
    };
  }, [repo]);
  return index;
}
