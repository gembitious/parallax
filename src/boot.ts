// 시작할 때 필요한 백엔드 값(성능 HUD 설정, 띄울 plx 요청, 앱 정보)을 가장 먼저 묻는다. main.tsx가 맨 먼저 불러오므로
// React·CodeMirror 모듈을 실행하는 동안 IPC가 오가고, 첫 렌더는 답을 받은 뒤 요청한 화면으로 바로 한다
// (빈 화면을 먼저 그리고 effect에서 차례로 묻지 않는다). 콜드 스타트 구간은 App.tsx recordShown이 남긴다.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PlxRequest } from "./nav";
import { initPerf, nowMs } from "./perf";

/** 첫 JS를 실행한 시각(epoch ms): JS 묶음을 받아 파싱한 직후, 큰 모듈을 실행하기 전이다. */
export const scriptStartMs = nowMs();

export type AppInfo = { version: string; coreVersion: string };

let handler: ((request: PlxRequest) => void) | null = null;
let early: PlxRequest[] = [];
const deliver = (request: PlxRequest | null) => {
  if (!request) return;
  if (handler) handler(request);
  else early.push(request);
};

// 듣기 시작하는 것과 띄울 요청을 묻는 것을 함께 보낸다. 듣기 전에 온 요청은 이벤트로 오지 않으므로 듣기 시작한 뒤
// 한 번 더 묻는다(같은 요청을 두 번 받으면 viewReducer가 id로 거른다).
const first = invoke<PlxRequest | null>("current_request");
listen<PlxRequest>("plx-request", (e) => deliver(e.payload))
  .then(() => invoke<PlxRequest | null>("current_request"))
  .then(deliver, (e) => console.error("plx 요청을 듣지 못했다", e));

export const appInfo: Promise<AppInfo> = invoke<AppInfo>("app_info");

/** 첫 렌더에 띄울 요청(콜드 스타트의 요청, 없거나 묻지 못했으면 null). 성능 HUD 설정을 받은 뒤 끝난다. */
export const initialRequest: Promise<PlxRequest | null> = Promise.all([
  first.catch((e) => {
    console.error("띄울 요청을 묻지 못했다", e);
    return null;
  }),
  initPerf().catch((e) => console.error("성능 HUD 설정을 받지 못했다", e)),
]).then(([request]) => request);

/** 첫 렌더 뒤의 plx 요청을 받는다. 그 전에 온 요청도 차례로 넘긴다. 돌려준 함수로 그만 받는다. */
export function onRequest(next: (request: PlxRequest) => void): () => void {
  handler = next;
  const pending = early;
  early = [];
  for (const request of pending) next(request);
  return () => {
    if (handler === next) handler = null;
  };
}
