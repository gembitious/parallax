// 작업 공간 상태: 에디터 그룹(좌우로 나눈 칸, 그룹마다 탭), 탭(탭마다 화면 스택), 아래 패널(화면 스택)과 키를 받는 칸
// (포커스). 순수 함수라 단위 테스트한다(state.test.ts). 화면 하나하나는 nav.ts의 StackEntry다. 어느 화면이든 탭에도
// 패널에도 둘 수 있다: blame은 탭에, 커밋 상세·히스토리·그래프는 패널에 열고 사용자가 옮긴다(탭 ↔ 패널).
import type { SearchState } from "../blame/editor";
import type { BlameTarget } from "../blame/session";
import type { LineRange } from "../history/types";
import {
  MAX_STACK,
  lastName,
  titleOf,
  type BlameEntry,
  type Navigation,
  type PlxRequest,
  type Shown,
  type StackEntry,
} from "../nav";

/** 화면을 늘어놓는 칸: 가운데 에디터(그룹·탭)와 아래 패널 */
export type Pane = "editor" | "panel";
/** 키를 받는 칸 */
export type Focus = Pane | "tree";

/** 에디터를 이보다 많이 나누지 않는다(좁은 창에서 쓸 수 없는 너비가 된다) */
export const MAX_GROUPS = 4;

export type Tab = {
  key: number;
  /** 탭 안의 탐색 스택(`,` `<` `>`가 쌓고 `q`가 뺀다). 맨 위 화면이 보인다. 아래 화면은 숨긴 채 살려 둔다. */
  stack: StackEntry[];
  /** 미리보기 탭: 다른 파일을 미리보기로 열면 이 자리에 연다. 탭 안에서 옮겨 가거나 고정하면 보통 탭이 된다. */
  preview: boolean;
  /** 이 탭을 연 칸. 패널에서 연 탭은 맨 아래 화면에서 `q`면 탭을 닫고 패널로 돌아간다. */
  from: Pane;
  /** 이 탭을 연 보기 탭(히스토리·커밋 상세·그래프가 맨 위인 탭). 맨 아래 화면에서 `q`면 탭을 닫고 그 탭으로 돌아간다. */
  opener?: number;
};

/** 에디터 그룹: 나란히 놓인 에디터 칸 하나와 그 탭들 */
export type Group = {
  key: number;
  tabs: Tab[];
  /** 보이는 탭 */
  active: number | null;
  /** 탭을 본 순서(앞이 최근). 보이는 탭을 닫으면 그 앞에 본 탭으로 간다. */
  recent: number[];
  /** 에디터 칸 너비에서 차지하는 몫(그룹들의 합에 대한 비율) */
  weight: number;
};

export type PanelEntry = StackEntry & {
  /** 이 화면을 연 칸. `q`로 빼면 그 칸으로 돌아간다. */
  from: Pane;
};

export type Workspace = {
  /** 작업트리 경로. 앱이 뜨기 전에 받은 요청(콜드 스타트)은 화면이 물어 채운다(그때까지 null). */
  repo: string | null;
  /** 마지막으로 받은 plx 요청(첫 화면까지의 시간을 잰다) */
  shown: Shown | null;
  /** 왼쪽부터. 하나 이상이고, 탭이 없는 그룹은 그것 하나뿐일 때만 있다. */
  groups: Group[];
  /** 키를 받는(마지막으로 쓴) 그룹 */
  activeGroup: number;
  panel: PanelEntry[];
  panelOpen: boolean;
  /** 패널이 에디터 자리까지 차지한다 */
  panelMax: boolean;
  focus: Focus;
};

/** 화면이 있는 자리 */
export type Place = { pane: "editor"; tab: number } | { pane: "panel" };

/** 탭을 놓을 자리: 그룹 안(`index` 앞, 없으면 보이는 탭 뒤), 그룹의 왼쪽·오른쪽(새 그룹) */
export type TabDrop = { kind: "group"; group: number; index?: number } | { kind: "split"; group: number; side: "left" | "right" };

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
  /** 파일 트리·빈 화면에서 연 파일. 작업트리에서 지운 파일은 HEAD 버전을 연다. `side`면 옆 그룹에 연다. */
  | { type: "open"; repo: string; path: string; missing: boolean; preview: boolean; side?: boolean }
  | { type: "activate"; tab: number }
  /** 그룹을 눌렀다: 그 그룹이 키를 받는다 */
  | { type: "focusGroup"; group: number }
  /** Mod+1~4: 왼쪽부터 `index`번 그룹으로 */
  | { type: "focusGroupAt"; index: number }
  /** 키를 받는 그룹 안에서 탭을 옮긴다 */
  | { type: "cycle"; delta: 1 | -1 }
  | { type: "pin"; tab: number }
  | { type: "close"; tab: number }
  /** 탭 메뉴: 그 그룹에서 `tab`만 남기고 닫는다(그 탭을 보인다) */
  | { type: "closeOthers"; tab: number }
  /** 탭 메뉴: 그 그룹(없으면 모든 그룹)의 탭을 닫는다 */
  | { type: "closeAll"; group?: number }
  /** 에디터 나누기: 탭의 화면을 옆 새 그룹에 하나 더 연다. blame은 `line` 줄로(커서 줄). */
  | { type: "split"; tab: number; side: "left" | "right"; line?: number | null }
  /** 탭을 옮긴다(끌어 놓기, 탭 메뉴). 옮긴 탭은 고정된다. */
  | { type: "moveTab"; tab: number; to: TabDrop }
  /** 탭의 화면 스택을 패널로 옮긴다(패널 화면 위에 쌓는다) */
  | { type: "tabToPanel"; tab: number }
  /** 패널에 보이는 화면을 탭으로 옮긴다(`to`가 없으면 키를 받는 그룹). 패널에는 그 아래 화면이 남는다. */
  | { type: "panelToEditor"; to?: TabDrop }
  /** 그룹 사이의 막대를 끌었다 */
  | { type: "resizeGroups"; weights: number[] }
  | { type: "focus"; focus: Focus }
  /** 패널 열기·닫기(Mod+J). 빈 패널을 열면 그래프를 연다. */
  | { type: "togglePanel" }
  | { type: "maximizePanel" }
  /** 패널 스택을 `key` 화면까지 뺀다(머리줄의 앞 화면을 눌렀다) */
  | { type: "panelTo"; key: number };

let nextKey = 1;
const newKey = () => nextKey++;

const newGroup = (weight = 1): Group => ({ key: newKey(), tabs: [], active: null, recent: [], weight });

export const emptyWorkspace = (repo: string | null = null): Workspace => {
  const group = newGroup();
  return {
    repo,
    shown: null,
    groups: [group],
    activeGroup: group.key,
    panel: [],
    panelOpen: false,
    panelMax: false,
    focus: "editor",
  };
};

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

function panelEntry(navigation: Extract<Navigation, { kind: "commit" | "history" | "graph" | "search" }>, from: Pane): PanelEntry {
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
        ...(navigation.compare ? { compare: navigation.compare } : {}),
        via: "commit",
        from,
      };
    case "history":
      return { key, view: "history", target: navigation.target, lines: navigation.lines ?? null, via: "history", from };
    case "graph":
      return { key, view: "graph", repo: navigation.repo, focus: navigation.rev, via: "graph", from };
    case "search":
      return { key, view: "search", repo: navigation.repo, rev: navigation.rev, query: navigation.query ?? "", via: "search", from };
  }
}

/** 나누기로 하나 더 여는 화면: 같은 대상을 새 화면으로(plx 요청·검색은 넘기지 않는다). blame은 `line` 줄로. */
function copyEntry(entry: StackEntry, line: number | null | undefined): StackEntry {
  const key = newKey();
  switch (entry.view) {
    case "blame":
      return { key, view: "blame", target: entry.target, line: line !== undefined ? line : entry.line, via: "file" };
    case "history":
      return { key, view: "history", target: entry.target, lines: entry.lines, via: "history" };
    case "commit":
      return {
        key,
        view: "commit",
        repo: entry.repo,
        rev: entry.rev,
        path: entry.path,
        only: entry.only,
        ...(entry.compare ? { compare: entry.compare } : {}),
        via: "commit",
      };
    case "graph":
      return { key, view: "graph", repo: entry.repo, focus: entry.focus, via: "graph" };
    case "search":
      return { key, view: "search", repo: entry.repo, rev: entry.rev, query: entry.query, via: "search" };
  }
}

export const topOf = (tab: Tab): StackEntry => tab.stack[tab.stack.length - 1];

/** 모든 그룹의 탭(왼쪽 그룹부터) */
export const allTabs = (state: Workspace): Tab[] => state.groups.flatMap((group) => group.tabs);

/** 키를 받는 그룹 */
export const activeGroupOf = (state: Workspace): Group =>
  state.groups.find((group) => group.key === state.activeGroup) ?? state.groups[0];

/** 그룹에 보이는 탭 */
export const visibleTab = (group: Group): Tab | null => group.tabs.find((tab) => tab.key === group.active) ?? null;

/** 키를 받는 그룹에 보이는 탭 */
export const activeTabOf = (state: Workspace): Tab | null => visibleTab(activeGroupOf(state));

/** `tab` 탭이 있는 그룹 */
export const groupOfTab = (state: Workspace, tab: number): Group | undefined =>
  state.groups.find((group) => group.tabs.some((t) => t.key === tab));

const findTabByKey = (state: Workspace, key: number): Tab | undefined => allTabs(state).find((tab) => tab.key === key);

/** 작업트리 파일의 탭이면 그 파일(저장소 기준). plx 요청이 같은 파일의 탭을 찾는다. */
export function tabFile(tab: Tab): string | null {
  const root = tab.stack[0];
  return root.view === "blame" && root.target.rev === null && root.target.repo !== null ? root.target.path : null;
}

/** 보기 화면(히스토리·그래프·커밋 상세)이 맨 위인 탭: 파일을 열 때 가리지 않는다 */
const isViewTab = (tab: Tab | null | undefined): boolean => !!tab && topOf(tab).view !== "blame";

const sameTarget = (a: BlameTarget, b: BlameTarget) =>
  a.repo !== null && a.repo === b.repo && a.path === b.path && a.rev === b.rev;

function withGroup(state: Workspace, key: number, update: (group: Group) => Group): Workspace {
  return { ...state, groups: state.groups.map((group) => (group.key === key ? update(group) : group)) };
}

function activate(state: Workspace, tab: number): Workspace {
  const group = groupOfTab(state, tab);
  if (!group) return state;
  return {
    ...withGroup(state, group.key, (g) => ({ ...g, active: tab, recent: [tab, ...g.recent.filter((key) => key !== tab)] })),
    activeGroup: group.key,
    focus: "editor",
    panelMax: false,
  };
}

/** `group` 옆 그룹(오른쪽 먼저). 없으면 undefined */
function neighborOf(state: Workspace, group: number): Group | undefined {
  const at = state.groups.findIndex((g) => g.key === group);
  return state.groups[at + 1] ?? state.groups[at - 1];
}

/**
 * 파일을 열 그룹. 파일을 연 화면의 그룹(탭에서 열었으면 그 탭의 그룹, 아니면 키를 받는 그룹)에 열되, 그 그룹에 보기
 * 화면(히스토리·그래프·커밋 상세)이 보이고 옆 그룹이 있으면 옆 그룹에 연다(나란히 놓은 보기 화면을 가리지 않는다).
 * `side`면 오른쪽 그룹에 열고, 없으면 오른쪽에 새 그룹을 만든다(이미 MAX_GROUPS면 옆 그룹).
 */
function fileGroup(state: Workspace, from: Place | null, side: boolean): { state: Workspace; group: number } {
  const source = (from?.pane === "editor" && groupOfTab(state, from.tab)) || activeGroupOf(state);
  // 빈 그룹(탭이 하나도 없다) 옆에 나누지 않는다: 빈 그룹은 그것 하나뿐일 때만 둔다.
  if (side && source.tabs.length > 0) {
    const at = state.groups.indexOf(source);
    const right = state.groups[at + 1];
    if (right) return { state, group: right.key };
    if (state.groups.length >= MAX_GROUPS) return { state, group: (state.groups[at - 1] ?? source).key };
    const half = source.weight / 2;
    const group = newGroup(half);
    const groups = [...state.groups.slice(0, at), { ...source, weight: half }, group, ...state.groups.slice(at + 1)];
    return { state: { ...state, groups }, group: group.key };
  }
  const neighbor = isViewTab(visibleTab(source)) ? neighborOf(state, source.key) : undefined;
  return { state, group: (neighbor ?? source).key };
}

/**
 * `entry`를 `group` 그룹의 탭에 연다. 같은 화면(저장소·경로·리비전의 blame)이 맨 위인 탭이 있으면 그 탭으로 가고(그
 * 그룹에서 먼저, `reveal`이면 다른 그룹에서도 찾는다), 줄을 주면 그 줄로 다시 연다(캐시라 빠르다). 미리보기는 그 그룹의
 * 미리보기 탭 자리에, 아니면 보이는 탭 바로 뒤에 연다.
 */
function openTab(
  state: Workspace,
  entry: BlameEntry,
  how: { preview: boolean; from: Pane; group: number; reveal: boolean; opener?: number },
): Workspace {
  const same = (tab: Tab) => {
    const top = topOf(tab);
    return top.view === "blame" && sameTarget(top.target, entry.target);
  };
  const target = state.groups.find((g) => g.key === how.group) ?? activeGroupOf(state);
  const existing = target.tabs.find(same) ?? (how.reveal ? allTabs(state).find(same) : undefined);
  if (existing) {
    const top = topOf(existing) as BlameEntry;
    const stack =
      entry.line === null ? existing.stack : [...existing.stack.slice(0, -1), { ...top, key: entry.key, line: entry.line, search: undefined }];
    const owner = groupOfTab(state, existing.key)!;
    const next = withGroup(state, owner.key, (g) => ({
      ...g,
      tabs: g.tabs.map((tab) => (tab === existing ? { ...tab, stack, preview: tab.preview && how.preview } : tab)),
    }));
    return activate(next, existing.key);
  }
  const tab: Tab = { key: newKey(), stack: [entry], preview: how.preview, from: how.from };
  if (how.opener !== undefined) tab.opener = how.opener;
  const previewAt = how.preview ? target.tabs.findIndex((t) => t.preview) : -1;
  if (previewAt >= 0) {
    const replaced = target.tabs[previewAt].key;
    const next = withGroup(state, target.key, (g) => ({
      ...g,
      tabs: g.tabs.map((t, i) => (i === previewAt ? tab : t)),
      recent: g.recent.filter((key) => key !== replaced),
    }));
    return activate(next, tab.key);
  }
  return placeTab(state, tab, { kind: "group", group: target.key });
}

/**
 * `tab`을 놓는다: 그룹 안(`index` 앞, 없으면 보이는 탭 뒤), 그룹의 옆(새 그룹. 그룹이 MAX_GROUPS면 그쪽 옆 그룹, 그쪽이
 * 끝이면 반대쪽 옆 그룹에. 빈 그룹의 옆은 그 그룹 안). 놓은 탭이 보이고 그 그룹이 키를 받는다.
 */
function placeTab(state: Workspace, tab: Tab, drop: TabDrop): Workspace {
  const at = state.groups.findIndex((g) => g.key === drop.group);
  if (at < 0) return state;
  if (drop.kind === "split" && state.groups[at].tabs.length > 0) {
    if (state.groups.length >= MAX_GROUPS) {
      const [near, far] = drop.side === "right" ? [at + 1, at - 1] : [at - 1, at + 1];
      const beside = state.groups[near] ?? state.groups[far] ?? state.groups[at];
      return placeTab(state, tab, { kind: "group", group: beside.key });
    }
    const source = state.groups[at];
    const half = source.weight / 2;
    const group: Group = { ...newGroup(half), tabs: [tab] };
    const groups = state.groups.slice();
    groups[at] = { ...source, weight: half };
    groups.splice(drop.side === "right" ? at + 1 : at, 0, group);
    return activate({ ...state, groups }, tab.key);
  }
  // 그룹 안(빈 그룹의 옆도 그 그룹 안이다)
  const group = state.groups[at];
  const after = group.tabs.findIndex((t) => t.key === group.active) + 1;
  const wanted = drop.kind === "group" ? drop.index : undefined;
  const index = Math.max(0, Math.min(wanted ?? (after > 0 ? after : group.tabs.length), group.tabs.length));
  const next = withGroup(state, group.key, (g) => ({ ...g, tabs: [...g.tabs.slice(0, index), tab, ...g.tabs.slice(index)] }));
  return activate(next, tab.key);
}

/**
 * 빈 그룹을 뺀다. 그 자리(너비)와 키를 받던 그룹이면 키도 왼쪽 그룹(없으면 오른쪽)이 받는다. 그룹이 하나뿐이면 그대로
 * 둔다.
 */
function removeGroup(state: Workspace, key: number): Workspace {
  const at = state.groups.findIndex((g) => g.key === key);
  if (at < 0 || state.groups.length < 2) return state;
  const groups = state.groups.filter((g) => g.key !== key);
  const heir = Math.max(0, at - 1);
  groups[heir] = { ...groups[heir], weight: groups[heir].weight + state.groups[at].weight };
  if (state.activeGroup !== key) return { ...state, groups };
  return { ...state, groups, activeGroup: groups[heir].key };
}

/** 탭을 그룹에서 뗀다(닫기·옮기기). 그룹이 비면 그룹을 뺀다(하나뿐이면 빈 그룹으로 둔다). 포커스는 그대로다. */
function detachTab(state: Workspace, key: number): Workspace {
  const group = groupOfTab(state, key);
  if (!group) return state;
  const tabs = group.tabs.filter((tab) => tab.key !== key);
  if (tabs.length === 0 && state.groups.length > 1) return removeGroup(state, group.key);
  const recent = group.recent.filter((k) => k !== key);
  const active = group.active === key ? (recent[0] ?? tabs[0]?.key ?? null) : group.active;
  return withGroup(state, group.key, (g) => ({ ...g, tabs, recent, active }));
}

/** `match`인 탭들이 맨 아래에서 `q`로 돌아갈 곳을 바꾼다(그 탭을 연 화면이 탭과 패널 사이로 옮겨 갔다). */
function goBackTo(state: Workspace, match: (tab: Tab) => boolean, to: Pick<Tab, "from" | "opener">): Workspace {
  if (!allTabs(state).some(match)) return state;
  return { ...state, groups: state.groups.map((g) => ({ ...g, tabs: g.tabs.map((t) => (match(t) ? { ...t, ...to } : t)) })) };
}

function closeTab(state: Workspace, key: number): Workspace {
  const next = detachTab(state, key);
  if (next === state) return state;
  // 키를 받는 그룹에 탭이 남지 않았으면(마지막 탭을 닫았다) 패널이나 빈 화면
  if (state.focus !== "editor" || activeTabOf(next) !== null) return next;
  return { ...next, focus: state.panelOpen && state.panel.length > 0 ? "panel" : "editor" };
}

function pushPanel(state: Workspace, entry: PanelEntry, keep: boolean): Workspace {
  const panel = keep ? [...state.panel, entry].slice(-MAX_STACK) : [entry];
  return { ...state, panel, panelOpen: true, focus: "panel" };
}

/**
 * 탭 메뉴의 "왼쪽·오른쪽 그룹으로": 그쪽 옆 그룹, 없으면 그쪽에 나눈다. 나눌 수 없으면(그룹이 MAX_GROUPS, 또는 탭이 그룹에
 * 혼자라 옮겨도 그대로다) null.
 */
export function besideTarget(state: Workspace, tab: number, side: "left" | "right"): TabDrop | null {
  const group = groupOfTab(state, tab);
  if (!group) return null;
  const at = state.groups.indexOf(group);
  const beside = state.groups[side === "left" ? at - 1 : at + 1];
  if (beside) return { kind: "group", group: beside.key };
  if (state.groups.length >= MAX_GROUPS || group.tabs.length < 2) return null;
  return { kind: "split", group: group.key, side };
}

/** 이 자리에서 `q`로 돌아갈 곳이 있는가. 없으면 창을 숨긴다. */
export function canGoBack(state: Workspace, place: Place): boolean {
  if (place.pane === "panel") return state.panel.length > 1 || allTabs(state).length > 0;
  const tab = findTabByKey(state, place.tab);
  if (!tab) return false;
  return (
    tab.stack.length > 1 ||
    (tab.from === "panel" && state.panelOpen && state.panel.length > 0) ||
    (tab.opener !== undefined && findTabByKey(state, tab.opener) !== undefined)
  );
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
  for (const group of state.groups) {
    for (const tab of group.tabs) {
      const at = tab.stack.findIndex((entry) => entry.key === key);
      if (at < 0) continue;
      const stack = tab.stack.slice();
      stack[at] = update(stack[at]);
      return withGroup(state, group.key, (g) => ({ ...g, tabs: g.tabs.map((t) => (t === tab ? { ...t, stack } : t)) }));
    }
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
      if (entry.view !== "blame") return pushPanel({ ...base, panelMax: false }, entry as PanelEntry, false);
      // 같은 파일의 탭이 있으면(키를 받는 그룹 먼저) 그 탭을 이 요청의 화면 하나로 바꾼다(다시 연다: 그 줄로, 캐시라 빠르다).
      const file = entry.target.repo !== null ? entry.target.path : null;
      const isFile = (tab: Tab) => tabFile(tab) === file;
      const existing =
        file === null ? undefined : (activeGroupOf(base).tabs.find(isFile) ?? allTabs(base).find(isFile));
      if (!existing) {
        const { state: next, group } = fileGroup(base, null, false);
        return openTab(next, entry, { preview: false, from: "editor", group, reveal: false });
      }
      const owner = groupOfTab(base, existing.key)!;
      const next = withGroup(base, owner.key, (g) => ({
        ...g,
        tabs: g.tabs.map((tab) => (tab === existing ? { ...tab, stack: [entry], preview: false } : tab)),
      }));
      return activate(next, existing.key);
    }
    case "navigate": {
      const { navigation, from } = action;
      if (navigation.worktree) {
        // 다른 워크트리: 그 워크트리의 작업 공간에서 같은 화면을 연다.
        const repo =
          navigation.kind === "commit" || navigation.kind === "graph" || navigation.kind === "search"
            ? navigation.repo
            : navigation.target.repo;
        const base = { ...emptyWorkspace(repo), shown: state.shown };
        if (
          navigation.kind === "commit" ||
          navigation.kind === "history" ||
          navigation.kind === "graph" ||
          navigation.kind === "search"
        ) {
          return pushPanel(base, panelEntry(navigation, "editor"), false);
        }
        return openTab(base, blameEntry(navigation), { preview: false, from: "editor", group: base.activeGroup, reveal: false });
      }
      switch (navigation.kind) {
        case "parent":
        case "older":
        case "newer": {
          // 그 화면이 있는 스택에 쌓는다. 탭이면 미리보기 탭이 고정된다.
          if (from.pane === "panel") return pushPanel(state, { ...blameEntry(navigation), from: "panel" }, true);
          const tab = findTabByKey(state, from.tab);
          if (!tab) return state;
          const stack = [...tab.stack, blameEntry(navigation)].slice(-MAX_STACK);
          const owner = groupOfTab(state, tab.key)!;
          const next = withGroup(state, owner.key, (g) => ({
            ...g,
            tabs: g.tabs.map((t) => (t === tab ? { ...t, stack, preview: false } : t)),
          }));
          return activate(next, tab.key);
        }
        case "file": {
          // 패널·보기 탭(히스토리·커밋 상세)에서 연 파일은 미리보기 탭으로(Quick Open은 보통 탭), blame에서 연 파일은 보통
          // 탭으로. 패널·보기 탭에서 연 탭은 맨 아래에서 `q`면 닫고 그 패널·탭으로 돌아간다.
          const source = from.pane === "editor" ? findTabByKey(state, from.tab) : undefined;
          const browsing = from.pane === "panel" || isViewTab(source);
          const { state: next, group } = fileGroup(state, from, navigation.side ?? false);
          return openTab(next, blameEntry(navigation), {
            preview: browsing && !navigation.pinned,
            from: from.pane === "panel" ? "panel" : "editor",
            group,
            reveal: !navigation.side,
            opener: source && isViewTab(source) ? source.key : undefined,
          });
        }
        default:
          // 커밋 상세·히스토리·그래프·검색은 패널에. 패널에서 갔거나 패널이 열려 있으면 패널 스택에 쌓는다.
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
        const tab = findTabByKey(state, from.tab)!;
        if (tab.stack.length > 1) {
          const owner = groupOfTab(state, tab.key)!;
          const next = withGroup(state, owner.key, (g) => ({
            ...g,
            tabs: g.tabs.map((t) => (t === tab ? { ...t, stack: t.stack.slice(0, -1) } : t)),
          }));
          return activate(next, tab.key);
        }
        // 보기 탭에서 연 탭의 맨 아래: 탭을 닫고 그 탭으로 돌아간다.
        if (tab.opener !== undefined && findTabByKey(state, tab.opener)) return activate(closeTab(state, tab.key), tab.opener);
        // 패널에서 연 탭의 맨 아래: 탭을 닫고 패널로 돌아간다.
        return { ...closeTab(state, tab.key), focus: "panel" };
      }
      if (state.panel.length > 1) {
        const popped = state.panel[state.panel.length - 1];
        const focus = popped.from === "editor" && activeTabOf(state) !== null ? "editor" : "panel";
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
      const side = action.side ?? false;
      const { state: next, group } = fileGroup(state, null, side);
      return openTab(next, entry, { preview: action.preview, from: "editor", group, reveal: !side });
    }
    case "activate":
      return findTabByKey(state, action.tab) ? activate(state, action.tab) : state;
    case "focusGroup": {
      const group = state.groups.find((g) => g.key === action.group);
      if (!group || (state.activeGroup === group.key && state.focus === "editor")) return state;
      return { ...state, activeGroup: group.key, focus: "editor", panelMax: group.tabs.length > 0 ? false : state.panelMax };
    }
    case "focusGroupAt": {
      const group = state.groups[action.index];
      return group ? workspaceReducer(state, { type: "focusGroup", group: group.key }) : state;
    }
    case "cycle": {
      const group = activeGroupOf(state);
      if (group.tabs.length === 0) return state;
      const at = Math.max(0, group.tabs.findIndex((tab) => tab.key === group.active));
      const next = group.tabs[(at + action.delta + group.tabs.length) % group.tabs.length];
      return activate(state, next.key);
    }
    case "pin": {
      const group = groupOfTab(state, action.tab);
      if (!group) return state;
      return withGroup(state, group.key, (g) => ({
        ...g,
        tabs: g.tabs.map((tab) => (tab.key === action.tab && tab.preview ? { ...tab, preview: false } : tab)),
      }));
    }
    case "close":
      return closeTab(state, action.tab);
    case "closeOthers": {
      const group = groupOfTab(state, action.tab);
      const tab = group?.tabs.find((t) => t.key === action.tab);
      if (!group || !tab || group.tabs.length < 2) return state;
      return activate(withGroup(state, group.key, (g) => ({ ...g, tabs: [tab], recent: [tab.key] })), tab.key);
    }
    case "closeAll": {
      const targets = action.group === undefined ? state.groups : state.groups.filter((g) => g.key === action.group);
      if (targets.every((g) => g.tabs.length === 0)) return state;
      let next = state;
      for (const group of targets) {
        next =
          next.groups.length > 1
            ? removeGroup(next, group.key)
            : withGroup(next, group.key, (g) => ({ ...g, tabs: [], recent: [], active: null }));
      }
      if (state.focus !== "editor" || activeTabOf(next) !== null) return next;
      return { ...next, focus: state.panelOpen && state.panel.length > 0 ? "panel" : "editor" };
    }
    case "split": {
      const tab = findTabByKey(state, action.tab);
      if (!tab) return state;
      const copy: Tab = { key: newKey(), stack: [copyEntry(topOf(tab), action.line)], preview: false, from: "editor" };
      return placeTab(state, copy, { kind: "split", group: groupOfTab(state, tab.key)!.key, side: action.side });
    }
    case "moveTab": {
      const source = groupOfTab(state, action.tab);
      const tab = source?.tabs.find((t) => t.key === action.tab);
      if (!source || !tab) return state;
      const { to } = action;
      const moved = { ...tab, preview: false };
      if (to.kind === "group" && to.group === source.key) {
        // 제 그룹의 화면 칸에 놓았으면(자리 없음) 그대로, 탭 막대에 놓았으면 자리만 바꾼다. `index`는 옮기기 전 목록의 자리다.
        if (to.index === undefined) return activate(state, tab.key);
        const from = source.tabs.indexOf(tab);
        const others = source.tabs.filter((t) => t !== tab);
        const at = Math.max(0, Math.min(to.index > from ? to.index - 1 : to.index, others.length));
        const next = withGroup(state, source.key, (g) => ({ ...g, tabs: [...others.slice(0, at), moved, ...others.slice(at)] }));
        return activate(next, tab.key);
      }
      // 혼자 있는 탭을 제 그룹 옆으로 옮겨도 그대로다(그룹이 비어 빠진다).
      if (to.kind === "split" && to.group === source.key && source.tabs.length === 1) return activate(state, tab.key);
      const next = detachTab(state, tab.key);
      if (!next.groups.some((g) => g.key === to.group)) return state;
      return placeTab(next, moved, to);
    }
    case "tabToPanel": {
      const tab = findTabByKey(state, action.tab);
      if (!tab) return state;
      const entries: PanelEntry[] = tab.stack.map((entry) => ({ ...entry, from: "editor" }));
      // 이 탭에서 연 탭들은 이제 패널로 돌아간다.
      const next = goBackTo(detachTab(state, tab.key), (t) => t.opener === tab.key, { from: "panel", opener: undefined });
      return { ...next, panel: [...state.panel, ...entries].slice(-MAX_STACK), panelOpen: true, panelMax: false, focus: "panel" };
    }
    case "panelToEditor": {
      const top = state.panel[state.panel.length - 1];
      const to: TabDrop = action.to ?? { kind: "group", group: activeGroupOf(state).key };
      // 끄는 사이 놓을 그룹이 없어졌으면(그 그룹의 탭을 닫았다) 옮기지 않는다.
      if (!top || !state.groups.some((g) => g.key === to.group)) return state;
      // 보이는 화면만 옮긴다(끌 때 이름표도 그 화면이다). 패널에 남은 화면이 있으면 그 탭의 맨 아래에서 q는 패널로 돌아간다.
      const panel = state.panel.slice(0, -1);
      const tab: Tab = { key: newKey(), stack: [top], preview: false, from: panel.length > 0 ? "panel" : "editor" };
      let base: Workspace = { ...state, panel, panelOpen: state.panelOpen && panel.length > 0, panelMax: false };
      // 패널이 비면 패널에서 연 탭들은 옮긴 화면의 탭으로 돌아간다.
      if (panel.length === 0) base = goBackTo(base, (t) => t.from === "panel", { from: "editor", opener: tab.key });
      return placeTab(base, tab, to);
    }
    case "resizeGroups": {
      const { weights } = action;
      if (weights.length !== state.groups.length || weights.some((w) => !(w > 0))) return state;
      return { ...state, groups: state.groups.map((group, i) => ({ ...group, weight: weights[i] })) };
    }
    case "focus": {
      const { focus } = action;
      if (focus === state.focus) return state;
      if (focus === "panel" && !state.panelOpen) return state;
      return { ...state, focus, panelMax: focus === "editor" && activeTabOf(state) !== null ? false : state.panelMax };
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

/** 키를 받는 칸의 맨 위 화면(트리면 키를 받는 그룹에 보이는 탭의 화면) */
export function focusedEntry(state: Workspace): StackEntry | null {
  if (state.focus === "panel" && state.panelOpen && state.panel.length > 0) return state.panel[state.panel.length - 1];
  const tab = activeTabOf(state);
  if (tab) return topOf(tab);
  return state.panelOpen && state.panel.length > 0 ? state.panel[state.panel.length - 1] : null;
}

/**
 * 도구 막대·메뉴 막대가 명령을 보내는 칸: 키를 받는 칸. 트리에 있으면(또는 키를 받는 칸에 화면이 없으면) 키를 받는
 * 그룹의 보이는 탭, 탭이 없거나 패널이 가렸으면 열린 패널. 보이는 화면이 없으면 null.
 */
export function targetPane(state: Workspace): Pane | null {
  const panel = state.panelOpen && state.panel.length > 0;
  if (state.focus === "panel" && panel) return "panel";
  if (activeTabOf(state) !== null && !(state.panelOpen && state.panelMax)) return "editor";
  return panel ? "panel" : null;
}

/** 창 제목: 키를 받는 화면. plx 요청으로 연 화면은 그 호출의 요약이다. 화면이 없으면 저장소 이름. */
export function windowTitle(state: Workspace): string | null {
  const entry = focusedEntry(state);
  if (!entry) return state.repo === null ? null : lastName(state.repo);
  if (entry.request !== undefined && entry.request === state.shown?.request.id) return state.shown.request.label;
  return titleOf(entry);
}
