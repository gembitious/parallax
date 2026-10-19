// 작업 공간 상태: 파일 탭(탭마다 blame 화면 스택), 아래 패널(히스토리·라인 히스토리·그래프·커밋 상세 스택)과
// 키를 받는 칸(포커스). 순수 함수라 단위 테스트한다(state.test.ts). 화면 하나하나는 nav.ts의 StackEntry다.
import type { SearchState } from "../blame/editor";
import type { BlameTarget } from "../blame/session";
import type { LineRange } from "../history/types";
import {
  MAX_STACK,
  lastName,
  titleOf,
  type BlameEntry,
  type CommitEntry,
  type GraphEntry,
  type HistoryEntry,
  type Navigation,
  type PlxRequest,
  type Shown,
  type StackEntry,
} from "../nav";

/** 화면을 늘어놓는 칸: 가운데 에디터(탭)와 아래 패널 */
export type Pane = "editor" | "panel";
/** 키를 받는 칸 */
export type Focus = Pane | "tree";

export type Tab = {
  key: number;
  /** 탭 안의 탐색 스택(`,` `<` `>`가 쌓고 `q`가 뺀다). 맨 위 화면이 보인다. 아래 화면은 숨긴 채 살려 둔다. */
  stack: BlameEntry[];
  /** 미리보기 탭: 다른 파일을 미리보기로 열면 이 자리에 연다. 탭 안에서 옮겨 가거나 고정하면 보통 탭이 된다. */
  preview: boolean;
  /** 이 탭을 연 칸. 패널에서 연 탭은 맨 아래 화면에서 `q`면 탭을 닫고 패널로 돌아간다. */
  from: Pane;
};

export type PanelEntry = (CommitEntry | HistoryEntry | GraphEntry) & {
  /** 이 화면을 연 칸. `q`로 빼면 그 칸으로 돌아간다. */
  from: Pane;
};

export type Workspace = {
  /** 작업트리 경로. 앱이 뜨기 전에 받은 요청(콜드 스타트)은 화면이 물어 채운다(그때까지 null). */
  repo: string | null;
  /** 마지막으로 받은 plx 요청(첫 화면까지의 시간을 잰다) */
  shown: Shown | null;
  tabs: Tab[];
  /** 보이는 탭 */
  active: number | null;
  /** 탭을 본 순서(앞이 최근). 보이는 탭을 닫으면 그 앞에 본 탭으로 간다. */
  recent: number[];
  panel: PanelEntry[];
  panelOpen: boolean;
  /** 패널이 에디터 자리까지 차지한다 */
  panelMax: boolean;
  focus: Focus;
};

/** 화면이 있는 자리 */
export type Place = { pane: "editor"; tab: number } | { pane: "panel" };

export type WorkspaceAction =
  | { type: "request"; request: PlxRequest; receivedMs: number }
  | { type: "navigate"; navigation: Navigation; from: Place }
  | { type: "back"; from: Place }
  /**
   * 저장소가 바뀌어(HEAD 이동) `key` 화면을 새로 연다. blame은 `line` 줄로, 라인 히스토리는 `lines`(새 작업트리의 줄로
   * 옮긴 범위)로. 자리는 그대로다. 화면의 검색(blame `search`, 히스토리 `query`)은 새 화면으로 넘긴다.
   */
  | { type: "reload"; key: number; line?: number | null; lines?: LineRange; search?: SearchState; query?: string }
  /** 절대 경로로 연 화면(콜드 스타트의 plx 요청)이 저장소와 저장소 기준 경로를 알렸다 */
  | { type: "located"; key: number; repo: string; path: string }
  /** 작업 공간의 저장소를 알았다(콜드 스타트) */
  | { type: "repo"; repo: string }
  /** 파일 트리·빈 화면에서 연 파일. 작업트리에서 지운 파일은 HEAD 버전을 연다. */
  | { type: "open"; repo: string; path: string; missing: boolean; preview: boolean }
  | { type: "activate"; tab: number }
  | { type: "cycle"; delta: 1 | -1 }
  | { type: "pin"; tab: number }
  | { type: "close"; tab: number }
  /** 탭 메뉴: `tab`만 남기고 닫는다(그 탭을 보인다) */
  | { type: "closeOthers"; tab: number }
  /** 탭 메뉴: 모든 탭을 닫는다 */
  | { type: "closeAll" }
  | { type: "focus"; focus: Focus }
  /** 패널 열기·닫기(Mod+J). 빈 패널을 열면 그래프를 연다. */
  | { type: "togglePanel" }
  | { type: "maximizePanel" }
  /** 패널 스택을 `key` 화면까지 뺀다(머리줄의 앞 화면을 눌렀다) */
  | { type: "panelTo"; key: number };

export const emptyWorkspace = (repo: string | null = null): Workspace => ({
  repo,
  shown: null,
  tabs: [],
  active: null,
  recent: [],
  panel: [],
  panelOpen: false,
  panelMax: false,
  focus: "editor",
});

let nextKey = 1;
const newKey = () => nextKey++;

/** plx 요청의 첫 화면. 백엔드가 저장소와 저장소 기준 경로를 찾았으면 그것으로 연다(같은 파일의 탭을 찾을 수 있다). */
function requestEntry(request: PlxRequest): BlameEntry | PanelEntry {
  const view = request.request;
  const key = newKey();
  const id = request.id;
  const target = (path: string): BlameTarget =>
    request.repo !== null && request.path !== null
      ? { repo: request.repo, path: request.path, rev: null }
      : { repo: null, path, rev: null };
  switch (view.kind) {
    case "blame":
      return { key, view: "blame", target: target(view.path), line: view.line, via: "request", request: id };
    case "log":
    case "lineLog":
      return {
        key,
        view: "history",
        target: target(view.path),
        lines: view.kind === "lineLog" ? { start: view.start, end: view.end } : null,
        via: "request",
        request: id,
        from: "panel",
      };
    case "show":
      return {
        key,
        view: "commit",
        repo: request.repo ?? request.cwd,
        rev: view.rev,
        path: null,
        only: false,
        via: "request",
        request: id,
        from: "panel",
      };
    case "graph":
      return { key, view: "graph", repo: request.repo ?? request.cwd, focus: null, via: "request", request: id, from: "panel" };
  }
}

function blameEntry(navigation: Extract<Navigation, { kind: "parent" | "older" | "newer" | "file" }>): BlameEntry {
  return { key: newKey(), view: "blame", target: navigation.target, line: navigation.line, via: navigation.kind };
}

function panelEntry(navigation: Extract<Navigation, { kind: "commit" | "history" | "graph" }>, from: Pane): PanelEntry {
  const key = newKey();
  switch (navigation.kind) {
    case "commit":
      return {
        key,
        view: "commit",
        repo: navigation.repo,
        rev: navigation.rev,
        path: navigation.path,
        only: navigation.only ?? false,
        via: "commit",
        from,
      };
    case "history":
      return { key, view: "history", target: navigation.target, lines: navigation.lines ?? null, via: "history", from };
    case "graph":
      return { key, view: "graph", repo: navigation.repo, focus: navigation.rev, via: "graph", from };
  }
}

export const topOf = (tab: Tab): BlameEntry => tab.stack[tab.stack.length - 1];

/** 작업트리 파일의 탭이면 그 파일(저장소 기준). plx 요청이 같은 파일의 탭을 찾는다. */
export function tabFile(tab: Tab): string | null {
  const { target } = tab.stack[0];
  return target.rev === null && target.repo !== null ? target.path : null;
}

const sameTarget = (a: BlameTarget, b: BlameTarget) =>
  a.repo !== null && a.repo === b.repo && a.path === b.path && a.rev === b.rev;

function activate(state: Workspace, tab: number): Workspace {
  return {
    ...state,
    active: tab,
    recent: [tab, ...state.recent.filter((key) => key !== tab)],
    focus: "editor",
    panelMax: false,
  };
}

/**
 * `entry`를 탭에 연다. 같은 화면(저장소·경로·리비전)이 맨 위인 탭이 있으면 그 탭으로 가고, 줄을 주면 그 줄로 다시
 * 연다(캐시라 빠르다). 미리보기는 있던 미리보기 탭 자리에, 아니면 보이는 탭 바로 뒤에 연다.
 */
function openTab(state: Workspace, entry: BlameEntry, how: { preview: boolean; from: Pane }): Workspace {
  const existing = state.tabs.find((tab) => sameTarget(topOf(tab).target, entry.target));
  if (existing) {
    const top = topOf(existing);
    const stack =
      entry.line === null ? existing.stack : [...existing.stack.slice(0, -1), { ...top, key: entry.key, line: entry.line, search: undefined }];
    const tabs = state.tabs.map((tab) => (tab === existing ? { ...tab, stack, preview: tab.preview && how.preview } : tab));
    return activate({ ...state, tabs }, existing.key);
  }
  const tab: Tab = { key: newKey(), stack: [entry], preview: how.preview, from: how.from };
  const previewAt = how.preview ? state.tabs.findIndex((t) => t.preview) : -1;
  if (previewAt >= 0) {
    const replaced = state.tabs[previewAt].key;
    const tabs = state.tabs.map((t, i) => (i === previewAt ? tab : t));
    return activate({ ...state, tabs, recent: state.recent.filter((key) => key !== replaced) }, tab.key);
  }
  const at = state.tabs.findIndex((t) => t.key === state.active);
  const tabs = at < 0 ? [...state.tabs, tab] : [...state.tabs.slice(0, at + 1), tab, ...state.tabs.slice(at + 1)];
  return activate({ ...state, tabs }, tab.key);
}

function closeTab(state: Workspace, key: number): Workspace {
  const tabs = state.tabs.filter((tab) => tab.key !== key);
  if (tabs.length === state.tabs.length) return state;
  const recent = state.recent.filter((k) => k !== key);
  if (state.active !== key) return { ...state, tabs, recent };
  const active = recent[0] ?? null;
  const focus = active !== null || state.focus !== "editor" ? state.focus : state.panelOpen ? "panel" : "editor";
  return { ...state, tabs, recent, active, focus };
}

function pushPanel(state: Workspace, entry: PanelEntry, keep: boolean): Workspace {
  const panel = keep ? [...state.panel, entry].slice(-MAX_STACK) : [entry];
  return { ...state, panel, panelOpen: true, focus: "panel" };
}

/** 이 자리에서 `q`로 돌아갈 곳이 있는가. 없으면 창을 숨긴다. */
export function canGoBack(state: Workspace, place: Place): boolean {
  if (place.pane === "panel") return state.panel.length > 1 || state.tabs.length > 0;
  const tab = state.tabs.find((t) => t.key === place.tab);
  if (!tab) return false;
  return tab.stack.length > 1 || (tab.from === "panel" && state.panelOpen && state.panel.length > 0);
}

function reloaded<T extends StackEntry>(entry: T, action: Extract<WorkspaceAction, { type: "reload" }>): T {
  const key = newKey();
  if (entry.view === "blame") {
    return { ...entry, key, line: action.line !== undefined ? action.line : entry.line, search: action.search };
  }
  if (entry.view === "history") {
    return { ...entry, key, lines: entry.lines && action.lines ? action.lines : entry.lines, query: action.query };
  }
  return { ...entry, key };
}

/** 스택들의 화면 가운데 `key` 화면만 `update`로 바꾼다. 없으면 상태를 그대로 돌려준다. */
function updateEntry(state: Workspace, key: number, update: <T extends StackEntry>(entry: T) => T): Workspace {
  for (let i = 0; i < state.tabs.length; i++) {
    const tab = state.tabs[i];
    const at = tab.stack.findIndex((entry) => entry.key === key);
    if (at < 0) continue;
    const stack = tab.stack.slice();
    stack[at] = update(stack[at]);
    const tabs = state.tabs.slice();
    tabs[i] = { ...tab, stack };
    return { ...state, tabs };
  }
  const at = state.panel.findIndex((entry) => entry.key === key);
  if (at < 0) return state;
  const panel = state.panel.slice();
  panel[at] = update(panel[at]);
  return { ...state, panel };
}

export function workspaceReducer(state: Workspace, action: WorkspaceAction): Workspace {
  switch (action.type) {
    case "request": {
      // 이벤트와 current_request가 같은 요청을 두 번 줄 수 있다. 더 최근 것만 받는다.
      if (state.shown && state.shown.request.id >= action.request.id) return state;
      const { request } = action;
      const shown = { request, receivedMs: action.receivedMs };
      // 다른 저장소(또는 아직 모르는 저장소)면 작업 공간을 새로 연다.
      const base: Workspace =
        state.repo !== null && request.repo === state.repo ? { ...state, shown } : { ...emptyWorkspace(request.repo), shown };
      const entry = requestEntry(request);
      if (entry.view !== "blame") return pushPanel({ ...base, panelMax: false }, entry, false);
      // 같은 파일의 탭이 있으면 그 탭을 이 요청의 화면 하나로 바꾼다(다시 연다: 그 줄로, 캐시라 빠르다).
      const file = entry.target.repo !== null ? entry.target.path : null;
      const existing = file === null ? undefined : base.tabs.find((tab) => tabFile(tab) === file);
      if (!existing) return openTab(base, entry, { preview: false, from: "editor" });
      const tabs = base.tabs.map((tab) => (tab === existing ? { ...tab, stack: [entry], preview: false } : tab));
      return activate({ ...base, tabs }, existing.key);
    }
    case "navigate": {
      const { navigation, from } = action;
      if (navigation.worktree) {
        // 다른 워크트리: 그 워크트리의 작업 공간에서 같은 화면을 연다.
        const repo = navigation.kind === "commit" || navigation.kind === "graph" ? navigation.repo : navigation.target.repo;
        const base = { ...emptyWorkspace(repo), shown: state.shown };
        if (navigation.kind === "commit" || navigation.kind === "history" || navigation.kind === "graph") {
          return pushPanel(base, panelEntry(navigation, "editor"), false);
        }
        return openTab(base, blameEntry(navigation), { preview: false, from: "editor" });
      }
      switch (navigation.kind) {
        case "parent":
        case "older":
        case "newer": {
          // 탭 안에서 옮겨 간다(미리보기 탭은 고정된다).
          const tab = from.pane === "editor" ? state.tabs.find((t) => t.key === from.tab) : undefined;
          if (!tab) return state;
          const stack = [...tab.stack, blameEntry(navigation)].slice(-MAX_STACK);
          const tabs = state.tabs.map((t) => (t === tab ? { ...t, stack, preview: false } : t));
          return activate({ ...state, tabs }, tab.key);
        }
        case "file":
          // 패널(히스토리·커밋 상세)에서 연 파일은 미리보기 탭으로(Quick Open은 보통 탭), 에디터에서 연 파일은 보통 탭으로.
          return openTab(
            state,
            blameEntry(navigation),
            from.pane === "panel" ? { preview: !navigation.pinned, from: "panel" } : { preview: false, from: "editor" },
          );
        default:
          // 커밋 상세·히스토리·그래프는 패널에. 패널에서 갔거나 패널이 열려 있으면 패널 스택에 쌓는다.
          return pushPanel(
            state,
            panelEntry(navigation, from.pane),
            from.pane === "panel" || (state.panelOpen && state.panel.length > 0),
          );
      }
    }
    case "back": {
      const { from } = action;
      if (!canGoBack(state, from)) return state;
      if (from.pane === "editor") {
        const tab = state.tabs.find((t) => t.key === from.tab)!;
        if (tab.stack.length > 1) {
          const tabs = state.tabs.map((t) => (t === tab ? { ...t, stack: t.stack.slice(0, -1) } : t));
          return activate({ ...state, tabs }, tab.key);
        }
        // 패널에서 연 탭의 맨 아래: 탭을 닫고 패널로 돌아간다.
        return { ...closeTab(state, tab.key), focus: "panel" };
      }
      if (state.panel.length > 1) {
        const popped = state.panel[state.panel.length - 1];
        const focus = popped.from === "editor" && state.active !== null ? "editor" : "panel";
        return { ...state, panel: state.panel.slice(0, -1), focus, panelMax: focus === "panel" && state.panelMax };
      }
      // 패널 맨 아래: 패널을 닫고 에디터로.
      return { ...state, panel: [], panelOpen: false, panelMax: false, focus: "editor" };
    }
    case "reload":
      return updateEntry(state, action.key, (entry) => reloaded(entry, action));
    case "located": {
      const { key, repo, path } = action;
      const next = updateEntry(state, key, (entry) =>
        (entry.view === "blame" || entry.view === "history") && entry.target.repo === null
          ? { ...entry, target: { ...entry.target, repo, path } }
          : entry,
      );
      return next.repo === null ? { ...next, repo } : next;
    }
    case "repo":
      return state.repo === null ? { ...state, repo: action.repo } : state;
    case "open": {
      const target = { repo: action.repo, path: action.path, rev: action.missing ? "HEAD" : null };
      const entry: BlameEntry = { key: newKey(), view: "blame", target, line: null, via: "file" };
      return openTab(state, entry, { preview: action.preview, from: "editor" });
    }
    case "activate":
      return state.tabs.some((tab) => tab.key === action.tab) ? activate(state, action.tab) : state;
    case "cycle": {
      if (state.tabs.length === 0) return state;
      const at = Math.max(0, state.tabs.findIndex((tab) => tab.key === state.active));
      const next = state.tabs[(at + action.delta + state.tabs.length) % state.tabs.length];
      return activate(state, next.key);
    }
    case "pin":
      return {
        ...state,
        tabs: state.tabs.map((tab) => (tab.key === action.tab && tab.preview ? { ...tab, preview: false } : tab)),
      };
    case "close":
      return closeTab(state, action.tab);
    case "closeOthers": {
      const tab = state.tabs.find((t) => t.key === action.tab);
      return tab ? activate({ ...state, tabs: [tab], recent: [tab.key] }, tab.key) : state;
    }
    case "closeAll": {
      if (state.tabs.length === 0) return state;
      const focus = state.focus === "editor" && state.panelOpen && state.panel.length > 0 ? "panel" : state.focus;
      return { ...state, tabs: [], recent: [], active: null, focus };
    }
    case "focus": {
      const { focus } = action;
      if (focus === state.focus) return state;
      if (focus === "panel" && !state.panelOpen) return state;
      return { ...state, focus, panelMax: focus === "editor" && state.tabs.length > 0 ? false : state.panelMax };
    }
    case "togglePanel": {
      if (state.panelOpen) {
        return { ...state, panelOpen: false, panelMax: false, focus: state.focus === "panel" ? "editor" : state.focus };
      }
      if (state.panel.length > 0) return { ...state, panelOpen: true, focus: "panel" };
      if (state.repo === null) return state;
      const graph: PanelEntry = { key: newKey(), view: "graph", repo: state.repo, focus: null, via: "graph", from: "editor" };
      return pushPanel(state, graph, false);
    }
    case "maximizePanel":
      return state.panelOpen ? { ...state, panelMax: !state.panelMax } : state;
    case "panelTo": {
      const at = state.panel.findIndex((entry) => entry.key === action.key);
      if (at < 0) return state;
      return { ...state, panel: state.panel.slice(0, at + 1), focus: "panel" };
    }
  }
}

/** 키를 받는 칸의 맨 위 화면(트리면 보이는 탭의 화면) */
export function focusedEntry(state: Workspace): StackEntry | null {
  if (state.focus === "panel" && state.panelOpen && state.panel.length > 0) return state.panel[state.panel.length - 1];
  const tab = state.tabs.find((t) => t.key === state.active);
  if (tab) return topOf(tab);
  return state.panelOpen && state.panel.length > 0 ? state.panel[state.panel.length - 1] : null;
}

/**
 * 도구 막대·메뉴 막대가 명령을 보내는 칸: 키를 받는 칸. 트리에 있으면(또는 키를 받는 칸에 화면이 없으면) 보이는 탭,
 * 탭이 없거나 패널이 가렸으면 열린 패널. 보이는 화면이 없으면 null.
 */
export function targetPane(state: Workspace): Pane | null {
  const panel = state.panelOpen && state.panel.length > 0;
  if (state.focus === "panel" && panel) return "panel";
  if (state.active !== null && !(state.panelOpen && state.panelMax)) return "editor";
  return panel ? "panel" : null;
}

/** 창 제목: 키를 받는 화면. plx 요청으로 연 화면은 그 호출의 요약이다. 화면이 없으면 저장소 이름. */
export function windowTitle(state: Workspace): string | null {
  const entry = focusedEntry(state);
  if (!entry) return state.repo === null ? null : lastName(state.repo);
  if (entry.request !== undefined && entry.request === state.shown?.request.id) return state.shown.request.label;
  return titleOf(entry);
}
