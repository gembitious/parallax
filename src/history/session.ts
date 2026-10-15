// 파일 히스토리 요청 하나. 백엔드(src-tauri/src/git.rs history_open)가 Channel로 이벤트를 보낸다.
import { openStream, type StreamHandle } from "../stream";
import type { HistoryEvent, HistoryTarget } from "./types";

/** 히스토리를 연다. 이벤트는 meta → entries* → done | error | cancelled 순서로 온다(끝 이벤트는 하나). */
export function openHistory(target: HistoryTarget, onEvent: (event: HistoryEvent) => void): StreamHandle {
  return openStream<HistoryEvent>("history_open", "history_cancel", { ...target }, onEvent);
}
