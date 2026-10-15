// 화면 상태: 받은 plx 요청과 blame 탐색 스택. 순수 함수라 단위 테스트한다(nav.test.ts).
import type { Navigation } from "./blame/BlameView";
import { shortSha } from "./blame/format";
import type { BlameTarget } from "./blame/session";

/** 열 뷰. plx-core/src/cli.rs의 Request와 같은 모양. */
export type ViewRequest =
  | { kind: "graph" }
  | { kind: "blame"; path: string; line: number | null }
  | { kind: "log"; path: string }
  | { kind: "lineLog"; path: string; start: number; end: number }
  | { kind: "show"; rev: string };

/** `plx` 호출 하나. src-tauri/src/instance.rs의 PlxRequest와 같은 모양. */
export type PlxRequest = {
  id: number;
  request: ViewRequest;
  label: string;
  args: string[];
  cwd: string;
  t0Ms: number;
  recvMs: number;
  cold: boolean;
};

/** 화면에 띄운 요청과 프론트엔드가 그 요청을 받은 시각. */
export type Shown = { request: PlxRequest; receivedMs: number };

/** blame 탐색 스택의 화면 하나. 아래 화면은 숨긴 채 살려 둬 뒤로 오면 커서·스크롤이 그대로다. */
export type StackEntry = {
  key: number;
  target: BlameTarget;
  line: number | null;
  /** 이 화면으로 온 방법 */
  via: "request" | Navigation["kind"];
};

export type ViewState = { shown: Shown | null; stack: StackEntry[] };

export type ViewAction =
  | { type: "request"; request: PlxRequest; receivedMs: number }
  | { type: "navigate"; navigation: Navigation }
  | { type: "back" };

/** 탐색 스택이 이보다 깊어지면 가장 아래 화면부터 버린다. */
export const MAX_STACK = 30;
let nextEntryKey = 1;

export function viewReducer(state: ViewState, action: ViewAction): ViewState {
  switch (action.type) {
    case "request": {
      // 이벤트와 current_request가 같은 요청을 두 번 줄 수 있다. 더 최근 것만 받는다.
      if (state.shown && state.shown.request.id >= action.request.id) return state;
      const view = action.request.request;
      return {
        shown: { request: action.request, receivedMs: action.receivedMs },
        stack:
          view.kind === "blame"
            ? [{ key: nextEntryKey++, target: { repo: null, path: view.path, rev: null }, line: view.line, via: "request" }]
            : [],
      };
    }
    case "navigate": {
      const { navigation } = action;
      const entry: StackEntry = {
        key: nextEntryKey++,
        target: navigation.target,
        line: navigation.line,
        via: navigation.kind,
      };
      return { ...state, stack: [...state.stack, entry].slice(-MAX_STACK) };
    }
    case "back":
      return state.stack.length > 1 ? { ...state, stack: state.stack.slice(0, -1) } : state;
  }
}

/** 탐색으로 온 화면에서 이 방향 키는 바로 아래 화면으로 돌아가는 것이다. */
export const backDirectionOf = (via: StackEntry["via"]) =>
  via === "older" ? "newer" : via === "newer" ? "older" : undefined;

/** 탐색으로 연 화면의 창 제목. 경로는 저장소 기준이다. */
export function titleOf({ target }: StackEntry): string {
  return `blame ${target.path}${target.rev ? ` @${shortSha(target.rev)}` : ""}`;
}
