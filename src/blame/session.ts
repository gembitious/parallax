// blame 요청 하나. 백엔드(src-tauri/src/git.rs blame_open)가 Channel로 이벤트를 보낸다.
import { openStream, type StreamHandle } from "../stream";
import type { SessionEvent } from "./types";

export type BlameHandle = StreamHandle;

/** 열 파일. `repo`가 있으면 `path`는 저장소 기준 경로, 없으면 절대 경로다. `rev`가 null이면 작업트리 파일이다. */
export type BlameTarget = { repo: string | null; path: string; rev: string | null };

/** blame을 연다. 이벤트는 meta → chunk* → done | error | cancelled 순서로 `onEvent`에 온다(끝 이벤트는 하나). */
export function openBlame(target: BlameTarget, onEvent: (event: SessionEvent) => void): BlameHandle {
  return openStream<SessionEvent>("blame_open", "blame_cancel", { ...target }, onEvent);
}
