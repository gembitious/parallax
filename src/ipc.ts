import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import { nowMs, record } from "./perf";

/** 백엔드 요청은 모두 이 함수로 보낸다. 성능 HUD에 왕복 시간이 남는다. */
export async function call<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  const start = nowMs();
  let ok = false;
  try {
    const result = await invoke<T>(cmd, args);
    ok = true;
    return result;
  } finally {
    const totalMs = nowMs() - start;
    record({ kind: "ipc", label: cmd, totalMs, phases: [{ label: "왕복", ms: totalMs }], ok });
  }
}
