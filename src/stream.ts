// 백엔드가 Channel로 결과를 흘려보내는 요청(blame, 히스토리). 시작 명령은 요청 id를 바로 돌려주고,
// 이벤트는 끝 이벤트(done·error·cancelled) 하나로 끝난다. 취소는 id로 한다.
import { Channel, invoke, type InvokeArgs } from "@tauri-apps/api/core";

export type StreamHandle = {
  /** 멈춘다. 이후 이벤트는 넘기지 않는다. 이미 끝났으면 아무것도 하지 않는다. */
  cancel(): void;
};

type EndEvent = { kind: "done" } | { kind: "error"; message: string } | { kind: "cancelled" };

const isEnd = (event: { kind: string }) =>
  event.kind === "done" || event.kind === "error" || event.kind === "cancelled";

/** `open` 명령으로 요청을 시작한다. 실패하면 `error` 이벤트 하나로 끝난다. */
export function openStream<E extends { kind: string }>(
  open: string,
  cancel: string,
  args: InvokeArgs & Record<string, unknown>,
  onEvent: (event: E | Extract<EndEvent, { kind: "error" }>) => void,
): StreamHandle {
  let ended = false;
  let cancelRequested = false;
  let id: number | null = null;

  const channel = new Channel<E>();
  channel.onmessage = (event) => {
    if (ended) return;
    if (isEnd(event)) ended = true;
    onEvent(event);
  };
  invoke<number>(open, { ...args, onEvent: channel }).then(
    (value) => {
      id = value;
      // id를 받기 전에 취소했다.
      if (cancelRequested) invoke(cancel, { id: value }).catch(() => {});
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
      if (id !== null) invoke(cancel, { id }).catch(() => {});
    },
  };
}
