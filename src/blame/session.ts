// blame 요청 하나. 백엔드(src-tauri/src/git.rs blame_open)가 Channel로 이벤트를 보낸다.
import { Channel, invoke } from "@tauri-apps/api/core";
import type { SessionEvent } from "./types";

export type BlameHandle = {
  /** 멈춘다. 이후 이벤트는 넘기지 않는다. 이미 끝났으면 아무것도 하지 않는다. */
  cancel(): void;
};

const isEnd = (event: SessionEvent) =>
  event.kind === "done" || event.kind === "error" || event.kind === "cancelled";

/**
 * blame을 연다. 이벤트는 meta → chunk* → done | error | cancelled 순서로 `onEvent`에 온다(끝 이벤트는 하나).
 * `rev`가 null이면 작업트리 파일이다.
 */
export function openBlame(path: string, rev: string | null, onEvent: (event: SessionEvent) => void): BlameHandle {
  let ended = false;
  let cancelRequested = false;
  let id: number | null = null;

  const channel = new Channel<SessionEvent>();
  channel.onmessage = (event) => {
    if (ended) return;
    if (isEnd(event)) ended = true;
    onEvent(event);
  };
  invoke<number>("blame_open", { path, rev, onEvent: channel }).then(
    (value) => {
      id = value;
      // id를 받기 전에 취소했다.
      if (cancelRequested) invoke("blame_cancel", { id: value }).catch(() => {});
    },
    (e: unknown) => {
      if (ended) return;
      ended = true;
      onEvent({ kind: "error", message: String(e) });
    },
  );

  return {
    cancel() {
      if (ended) return;
      ended = true;
      cancelRequested = true;
      if (id !== null) invoke("blame_cancel", { id }).catch(() => {});
    },
  };
}
