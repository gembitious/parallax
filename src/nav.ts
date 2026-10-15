// 화면 상태: 받은 plx 요청과 탐색 스택(blame·커밋 상세·파일 히스토리). 순수 함수라 단위 테스트한다(nav.test.ts).
import { shortSha } from "./blame/format";
import type { BlameTarget } from "./blame/session";
import type { HistoryTarget, LineRange } from "./history/types";

/** 화면이 다른 화면으로 가자는 요청. */
export type Navigation =
  /** blame → blame: `,` 부모 리비전, `<`·`>` 이전·다음 리비전 */
  | { kind: "parent" | "older" | "newer"; target: BlameTarget; line: number }
  /** blame·히스토리 → 커밋 상세(`Enter`). `path`는 처음 고를 파일, `only`면 그 파일만 보인다(히스토리에서 열 때) */
  | { kind: "commit"; repo: string; rev: string; path: string | null; only?: boolean }
  /** 커밋 상세·히스토리 → 그 리비전의 파일 blame */
  | { kind: "file"; target: BlameTarget; line: number | null }
  /** blame → 파일 히스토리(`H`), `lines`가 있으면 라인 히스토리(`L`) */
  | { kind: "history"; target: HistoryTarget; lines?: LineRange };

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

/** 탐색 스택의 화면 하나. 아래 화면은 숨긴 채 살려 둬 뒤로 오면 커서·스크롤이 그대로다. */
export type StackEntry = BlameEntry | CommitEntry | HistoryEntry;

export type BlameEntry = {
  key: number;
  view: "blame";
  target: BlameTarget;
  line: number | null;
  /** 이 화면으로 온 방법 */
  via: "request" | "parent" | "older" | "newer" | "file";
};

export type CommitEntry = {
  key: number;
  view: "commit";
  /** 저장소 안의 아무 경로(작업트리 또는 plx 호출의 cwd) */
  repo: string;
  rev: string;
  /** 처음 고를 파일(저장소 기준) */
  path: string | null;
  /** `path` 파일의 diff만 보인다(파일 히스토리에서 연 커밋). 화면에서 전체로 넓힐 수 있다. */
  only: boolean;
  via: "request" | "commit";
};

export type HistoryEntry = {
  key: number;
  view: "history";
  target: HistoryTarget;
  /** 있으면 그 줄 범위의 라인 히스토리 */
  lines: LineRange | null;
  via: "request" | "history";
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
      const { request } = action;
      const view = request.request;
      const key = nextEntryKey++;
      const root: StackEntry | null =
        view.kind === "blame"
          ? { key, view: "blame", target: { repo: null, path: view.path, rev: null }, line: view.line, via: "request" }
          : view.kind === "show"
            ? { key, view: "commit", repo: request.cwd, rev: view.rev, path: null, only: false, via: "request" }
            : view.kind === "log" || view.kind === "lineLog"
              ? {
                  key,
                  view: "history",
                  target: { repo: null, path: view.path, rev: null },
                  lines: view.kind === "lineLog" ? { start: view.start, end: view.end } : null,
                  via: "request",
                }
              : null;
      return { shown: { request, receivedMs: action.receivedMs }, stack: root ? [root] : [] };
    }
    case "navigate": {
      const { navigation } = action;
      const key = nextEntryKey++;
      const entry: StackEntry =
        navigation.kind === "commit"
          ? {
              key,
              view: "commit",
              repo: navigation.repo,
              rev: navigation.rev,
              path: navigation.path,
              only: navigation.only ?? false,
              via: "commit",
            }
          : navigation.kind === "history"
            ? { key, view: "history", target: navigation.target, lines: navigation.lines ?? null, via: "history" }
            : { key, view: "blame", target: navigation.target, line: navigation.line, via: navigation.kind };
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
export function titleOf(entry: StackEntry): string {
  if (entry.view === "commit") return `커밋 ${shortSha(entry.rev)}`;
  const { target } = entry;
  const at = target.rev ? ` @${shortSha(target.rev)}` : "";
  if (entry.view === "blame") return `blame ${target.path}${at}`;
  return entry.lines
    ? `라인 히스토리 ${target.path}:${entry.lines.start}-${entry.lines.end}${at}`
    : `히스토리 ${target.path}${at}`;
}
