// 화면 하나하나(blame·커밋 상세·파일 히스토리·그래프)와 화면 사이의 탐색. 화면을 탭·패널에 늘어놓는 것은
// workspace/state.ts가 한다.
import { shortSha } from "./blame/format";
import type { SearchState } from "./blame/editor";
import type { BlameTarget } from "./blame/session";
import type { HistoryTarget, LineRange } from "./history/types";

/** 화면이 다른 화면으로 가자는 요청. */
export type Navigation = (
  /** blame → blame: `,` 부모 리비전, `<`·`>` 이전·다음 리비전 */
  | { kind: "parent" | "older" | "newer"; target: BlameTarget; line: number }
  /** blame·히스토리 → 커밋 상세(`Enter`). `path`는 처음 고를 파일, `only`면 그 파일만 보인다(히스토리에서 열 때) */
  | { kind: "commit"; repo: string; rev: string; path: string | null; only?: boolean }
  /**
   * 커밋 상세·히스토리 → 그 리비전의 파일 blame, Quick Open → 작업트리 파일 blame. 패널에서 연 파일은 미리보기 탭이고
   * `pinned`(Quick Open)면 보통 탭이다. `side`면 옆(오른쪽) 에디터 그룹에 연다(Quick Open의 Ctrl+Enter).
   */
  | { kind: "file"; target: BlameTarget; line: number | null; pinned?: boolean; side?: boolean }
  /** blame → 파일 히스토리(`H`), `lines`가 있으면 라인 히스토리(`L`) */
  | { kind: "history"; target: HistoryTarget; lines?: LineRange }
  /** 어느 화면에서든 → 커밋 그래프(`G`). `rev`가 있으면 그 커밋을 골라 둔다. */
  | { kind: "graph"; repo: string; rev: string | null }
) & {
  /** 다른 워크트리로 옮긴다(`W`): 작업 공간을 그 워크트리의 것으로 새로 연다 */
  worktree?: boolean;
};

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
  /** 호출한 plx 프로세스가 main에 들어온 시각. 모르면 0 */
  mainMs: number;
  recvMs: number;
  cold: boolean;
  /** 요청 대상 저장소의 작업트리. 앱이 뜨기 전에 받은 요청(콜드 스타트)은 null이다 */
  repo: string | null;
  /** 파일을 준 요청(blame·log)의 저장소 기준 경로. 모르면 null */
  path: string | null;
};

/** 화면에 띄운 요청과 프론트엔드가 그 요청을 받은 시각. */
export type Shown = { request: PlxRequest; receivedMs: number };

/** 탐색 스택의 화면 하나. 아래 화면은 숨긴 채 살려 둬 뒤로 오면 커서·스크롤이 그대로다. */
export type StackEntry = BlameEntry | CommitEntry | HistoryEntry | GraphEntry;

export type BlameEntry = {
  key: number;
  view: "blame";
  target: BlameTarget;
  line: number | null;
  /** 이 화면으로 온 방법 */
  via: "request" | "parent" | "older" | "newer" | "file";
  /** plx 요청으로 연 화면이면 그 요청 id(첫 화면까지의 시간을 잰다) */
  request?: number;
  /** 다시 열기(reload) 전 화면의 검색 */
  search?: SearchState;
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
  request?: number;
};

export type HistoryEntry = {
  key: number;
  view: "history";
  target: HistoryTarget;
  /** 있으면 그 줄 범위의 라인 히스토리 */
  lines: LineRange | null;
  via: "request" | "history";
  request?: number;
  /** 다시 열기(reload) 전 화면의 검색어(`n`/`N`이 이어서 찾는다) */
  query?: string;
};

export type GraphEntry = {
  key: number;
  view: "graph";
  /** 저장소 안의 아무 경로(작업트리 또는 plx 호출의 cwd) */
  repo: string;
  /** 처음 고를 커밋 */
  focus: string | null;
  via: "request" | "graph";
  request?: number;
};

/** 탐색 스택(탭 하나, 패널)이 이보다 깊어지면 가장 아래 화면부터 버린다. */
export const MAX_STACK = 30;

/** 탐색으로 온 화면에서 이 방향 키는 바로 아래 화면으로 돌아가는 것이다. */
export const backDirectionOf = (via: StackEntry["via"]) =>
  via === "older" ? "newer" : via === "newer" ? "older" : undefined;

/**
 * 스택의 `index`번 화면에서 돌아가는 방향 키. 맨 아래 화면에는 돌아갈 곳이 없다: 스택이 MAX_STACK을 넘어 아래를
 * 버리면 `<`로 온 화면이 맨 아래가 되는데, 그때 `>`는 새 화면(다음 리비전)을 연다.
 */
export const backDirectionAt = (stack: readonly StackEntry[], index: number) =>
  index > 0 ? backDirectionOf(stack[index].via) : undefined;

/** 탐색으로 연 화면의 창 제목. 경로는 저장소 기준이다. */
export function titleOf(entry: StackEntry): string {
  if (entry.view === "commit") return `커밋 ${shortSha(entry.rev)}`;
  if (entry.view === "graph") return `그래프 ${lastName(entry.repo)}`;
  const { target } = entry;
  const at = target.rev ? ` @${shortSha(target.rev)}` : "";
  if (entry.view === "blame") return `blame ${target.path}${at}`;
  return entry.lines
    ? `라인 히스토리 ${target.path}:${entry.lines.start}-${entry.lines.end}${at}`
    : `히스토리 ${target.path}${at}`;
}

/** 경로의 마지막 이름(Windows 경로도) */
export function lastName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.slice(Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\")) + 1) || trimmed || path;
}
