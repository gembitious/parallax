// 다른 워크트리로 화면 옮기기(`W`): 그 워크트리의 같은 화면이 무엇인지 정한다. 순수 함수라 단위 테스트한다
// (switch.test.ts). 객체는 워크트리끼리 같으므로 리비전에 고정된 화면은 리비전 그대로 옮기고, 작업트리·HEAD 기준
// 화면만 그 워크트리의 것으로 바꾼다.
import type { Navigation } from "../nav";
import type { HistoryMeta, LineRange } from "../history/types";
import type { Worktree } from "./WorktreePicker";

/**
 * blame에서 고른 워크트리로: 리비전 blame은 같은 리비전(내용이 같다), 작업트리 blame은 그 워크트리의 같은 경로와
 * 같은 줄(파일이 다를 수 있어 가까운 자리). 아직 blame을 열지 못했으면(읽는 중, 오류) 그 워크트리의 그래프.
 * 지금 워크트리면 null.
 */
export function blameSwitch(
  worktree: Worktree,
  meta: { path: string; rev: string | null } | null,
  line: number | null,
): Navigation | null {
  if (worktree.current) return null;
  if (!meta) return { kind: "graph", repo: worktree.path, rev: null };
  return { kind: "file", target: { repo: worktree.path, path: meta.path, rev: meta.rev }, line };
}

/**
 * 히스토리에서 고른 워크트리로. 리비전에서 시작한 히스토리는 그대로 옮긴다. HEAD에서 시작한 히스토리는 그 워크트리의
 * HEAD에서 다시 시작하고, 라인 히스토리의 줄 범위(시작 커밋 버전)는 `mapLines`로 그 워크트리의 작업트리 줄로 옮긴다.
 * 옮기지 못하면(그 워크트리에 파일이 없다) 줄 범위 없이 파일 히스토리로 간다. 지금 워크트리면 null.
 */
export async function historySwitch(
  worktree: Worktree,
  meta: Pick<HistoryMeta, "path" | "rev" | "start" | "lines">,
  mapLines: (from: { rev: string; path: string }, lines: LineRange) => Promise<LineRange | null>,
): Promise<Navigation | null> {
  if (worktree.current) return null;
  if (meta.rev !== null) {
    return {
      kind: "history",
      target: { repo: worktree.path, path: meta.path, rev: meta.rev },
      lines: meta.lines ?? undefined,
    };
  }
  const target = { repo: worktree.path, path: meta.path, rev: null };
  if (!meta.lines || !meta.start) return { kind: "history", target };
  const lines = await mapLines({ rev: meta.start, path: meta.path }, meta.lines).catch(() => null);
  return lines ? { kind: "history", target, lines } : { kind: "history", target };
}

/** 커밋 상세에서 고른 워크트리로: 그 워크트리의 그래프에서 같은 커밋을 고른다. 지금 워크트리면 null. */
export function commitSwitch(worktree: Worktree, sha: string): Navigation | null {
  return worktree.current ? null : { kind: "graph", repo: worktree.path, rev: sha };
}
