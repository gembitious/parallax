// 파일·라인 히스토리 요청 하나. 백엔드(src-tauri/src/git.rs history_open, line_history_open)가 Channel로 이벤트를 보낸다.
import { openStream, type StreamHandle } from "../stream";
import type { HistoryEvent, HistoryTarget, LineRange } from "./types";

/**
 * 히스토리를 연다. `lines`가 있으면 그 줄 범위의 라인 히스토리(`git log -L`)다.
 * 이벤트는 meta → entries* → done | error | cancelled 순서로 온다(끝 이벤트는 하나).
 */
export function openHistory(
  target: HistoryTarget,
  lines: LineRange | null,
  onEvent: (event: HistoryEvent) => void,
): StreamHandle {
  return lines
    ? openStream<HistoryEvent>("line_history_open", "history_cancel", { ...target, ...lines }, onEvent)
    : openStream<HistoryEvent>("history_open", "history_cancel", { ...target }, onEvent);
}
