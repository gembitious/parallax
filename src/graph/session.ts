// 커밋 그래프 하나. 백엔드(src-tauri/src/git.rs graph_open)가 Channel로 이벤트를 보내고, 첫 화면 뒤의 행은
// graph_rows로 읽는다. 백엔드가 행 레이아웃을 메모리에 두므로 화면을 떠나면 반드시 닫는다(끝난 뒤에도).
import { Channel, invoke } from "@tauri-apps/api/core";
import type { GraphEvent, GraphScope, GraphWindow } from "./types";

/** 새 검색이 앞 검색을 멈췄을 때의 오류(plx-core graph::CANCELLED). 화면은 무시한다. */
export const CANCELLED = "취소했다";

export type GraphHandle = {
  rows(start: number, end: number): Promise<GraphWindow>;
  /** 리비전이 놓인 행. 그래프에 없으면 null. */
  find(rev: string): Promise<number | null>;
  /** 메시지·작성자·sha로 `from` 다음(`forward`가 아니면 이전) 행부터 찾는다. 새 검색이 오면 CANCELLED로 실패한다. */
  search(query: string, from: number, forward: boolean): Promise<number | null>;
  /** 경로(파일·디렉터리)를 바꾼 커밋의 행들(오름차순) */
  searchPath(path: string): Promise<number[]>;
  /** 닫는다. 이후 이벤트는 넘기지 않는다. */
  close(): void;
};

export function openGraph(repo: string, scope: GraphScope, onEvent: (event: GraphEvent) => void): GraphHandle {
  let closed = false;
  const channel = new Channel<GraphEvent>();
  channel.onmessage = (event) => {
    if (!closed) onEvent(event);
  };
  const id = invoke<number>("graph_open", {
    repo,
    options: { scope },
    onEvent: channel,
  });
  id.catch((e: unknown) => {
    if (!closed) onEvent({ kind: "error", message: String(e) });
  });
  return {
    rows: (start, end) => id.then((value) => invoke<GraphWindow>("graph_rows", { id: value, start, end })),
    find: (rev) => id.then((value) => invoke<number | null>("graph_find", { id: value, rev })),
    search: (query, from, forward) =>
      id.then((value) => invoke<number | null>("graph_search", { id: value, query, from, forward })),
    searchPath: (path) => id.then((value) => invoke<number[]>("graph_search_path", { id: value, path })),
    close() {
      if (closed) return;
      closed = true;
      id.then((value) => invoke("graph_close", { id: value })).catch(() => {});
    },
  };
}
