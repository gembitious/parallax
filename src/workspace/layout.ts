// 작업 공간 복원(M7-5): 저장소마다 그룹·탭(화면 스택)·패널 배치를 이 컴퓨터에 남기고, 그 저장소를 다시 열면(앱을 다시
// 띄운 뒤에도) 되살린다. 여기는 배치 ↔ 저장 모양(순수 함수, layout.test.ts)과 저장소(localStorage)다. 되살린 배치를
// 작업 공간으로 만드는 것(화면 key)은 state.ts의 restoreWorkspace가 한다.
import type { Compare, StackEntry } from "../nav";
import type { Pane, Workspace } from "./state";

/** 저장하는 화면 하나: key·plx 요청·다시 열기 때 넘긴 검색은 뺀다. */
export type SavedEntry =
  | {
      view: "blame";
      target: { repo: string; path: string; rev: string | null };
      line: number | null;
      via: "parent" | "older" | "newer" | "file";
    }
  | { view: "history"; target: { repo: string; path: string; rev: string | null }; lines: { start: number; end: number } | null }
  | { view: "commit"; repo: string; rev: string; path: string | null; only: boolean; compare?: Compare }
  | { view: "graph"; repo: string; focus: string | null }
  | { view: "search"; repo: string; rev: string | null; query: string };

export type SavedTab = {
  stack: SavedEntry[];
  preview: boolean;
  from: Pane;
  /** 이 탭을 연 보기 탭(모든 그룹의 탭을 왼쪽 그룹부터 센 자리) */
  opener?: number;
};

export type SavedGroup = { tabs: SavedTab[]; active: number | null; weight: number };

export type Layout = {
  groups: SavedGroup[];
  activeGroup: number;
  panel: (SavedEntry & { from: Pane })[];
  panelOpen: boolean;
  panelMax: boolean;
};

/** 되살릴 때 쓰는 탭 수 한도(모든 그룹의 합). 보이지 않는 탭은 처음 보일 때 읽는다(Workspace.tsx). */
export const MAX_SAVED_TABS = 40;
const MAX_SAVED_STACK = 30;

/**
 * 작업 공간의 배치. `lineOf`는 blame 화면의 지금 커서 줄(없으면 연 줄)이다. 저장소를 모르거나 남길 화면이 없으면 null.
 */
export function toLayout(state: Workspace, lineOf: (key: number) => number | null | undefined = () => undefined): Layout | null {
  if (state.repo === null) return null;
  const tabIndex = new Map<number, number>();
  const flat: SavedTab[] = [];
  const kept: { key: number; group: SavedGroup }[] = [];
  for (const group of state.groups) {
    const tabs: SavedTab[] = [];
    let active: number | null = null;
    for (const tab of group.tabs) {
      if (flat.length >= MAX_SAVED_TABS) break;
      const stack = tab.stack.map((entry) => saveEntry(entry, lineOf(entry.key))).filter((e): e is SavedEntry => e !== null);
      if (stack.length === 0) continue;
      if (tab.key === group.active) active = tabs.length;
      const saved: SavedTab = { stack: stack.slice(-MAX_SAVED_STACK), preview: tab.preview, from: tab.from };
      tabIndex.set(tab.key, flat.length);
      flat.push(saved);
      tabs.push(saved);
    }
    if (tabs.length > 0) kept.push({ key: group.key, group: { tabs, active: active ?? 0, weight: group.weight } });
  }
  // opener는 탭 자리(모든 그룹의 탭을 왼쪽부터 센)로 바꾼다. 되살릴 때 새 key로 잇는다.
  for (const tab of state.groups.flatMap((g) => g.tabs)) {
    const at = tabIndex.get(tab.key);
    const opener = tab.opener !== undefined ? tabIndex.get(tab.opener) : undefined;
    if (at !== undefined && opener !== undefined) flat[at].opener = opener;
  }
  const panel = state.panel
    .map((entry) => {
      const saved = saveEntry(entry, lineOf(entry.key));
      return saved && { ...saved, from: entry.from };
    })
    .filter((e): e is SavedEntry & { from: Pane } => e !== null)
    .slice(-MAX_SAVED_STACK);
  if (flat.length === 0 && panel.length === 0) return null;
  const groups = kept.length > 0 ? normalizeWeights(kept.map((g) => g.group)) : [{ tabs: [], active: null, weight: 1 }];
  return {
    groups,
    activeGroup: Math.max(0, kept.findIndex((g) => g.key === state.activeGroup)),
    panel,
    panelOpen: state.panelOpen && panel.length > 0,
    panelMax: state.panelMax && state.panelOpen && panel.length > 0,
  };
}

function normalizeWeights(groups: SavedGroup[]): SavedGroup[] {
  const sum = groups.reduce((s, g) => s + g.weight, 0);
  return groups.map((g) => ({ ...g, weight: sum > 0 ? g.weight / sum : 1 / groups.length }));
}

function saveEntry(entry: StackEntry, line: number | null | undefined): SavedEntry | null {
  switch (entry.view) {
    case "blame": {
      const { repo, path, rev } = entry.target;
      if (repo === null) return null;
      return {
        view: "blame",
        target: { repo, path, rev },
        line: line !== undefined ? line : entry.line,
        via: entry.via === "request" ? "file" : entry.via,
      };
    }
    case "history": {
      const { repo, path, rev } = entry.target;
      if (repo === null) return null;
      return { view: "history", target: { repo, path, rev }, lines: entry.lines };
    }
    case "commit":
      return {
        view: "commit",
        repo: entry.repo,
        rev: entry.rev,
        path: entry.path,
        only: entry.only,
        ...(entry.compare ? { compare: entry.compare } : {}),
      };
    case "graph":
      return { view: "graph", repo: entry.repo, focus: entry.focus };
    case "search":
      return { view: "search", repo: entry.repo, rev: entry.rev, query: entry.query };
  }
}

// 읽기: 디스크(localStorage)에서 온 값이라 모양을 모두 확인한다. 화면 하나가 틀리면 그 화면만, 배치가 틀리면 배치를 버린다.

type Json = unknown;
const isObject = (v: Json): v is Record<string, Json> => typeof v === "object" && v !== null && !Array.isArray(v);
const isString = (v: Json): v is string => typeof v === "string";
const isStringOrNull = (v: Json): v is string | null => v === null || typeof v === "string";
const isLine = (v: Json): v is number => typeof v === "number" && Number.isInteger(v) && v >= 1 && v <= 1e9;
const isPane = (v: Json): v is Pane => v === "editor" || v === "panel";

function readTarget(v: Json): { repo: string; path: string; rev: string | null } | null {
  if (!isObject(v) || !isString(v.repo) || !isString(v.path) || !isStringOrNull(v.rev)) return null;
  return { repo: v.repo, path: v.path, rev: v.rev };
}

function readCompare(v: Json): Compare | null {
  if (!isObject(v) || !isString(v.base) || !isStringOrNull(v.target) || typeof v.mergeBase !== "boolean") return null;
  return { base: v.base, target: v.target, mergeBase: v.mergeBase };
}

export function readEntry(v: Json): SavedEntry | null {
  if (!isObject(v)) return null;
  switch (v.view) {
    case "blame": {
      const target = readTarget(v.target);
      const via = v.via === "parent" || v.via === "older" || v.via === "newer" || v.via === "file" ? v.via : null;
      if (!target || via === null || !(v.line === null || isLine(v.line))) return null;
      return { view: "blame", target, line: v.line, via };
    }
    case "history": {
      const target = readTarget(v.target);
      if (!target) return null;
      if (v.lines === null) return { view: "history", target, lines: null };
      if (!isObject(v.lines) || !isLine(v.lines.start) || !isLine(v.lines.end) || v.lines.start > v.lines.end) return null;
      return { view: "history", target, lines: { start: v.lines.start, end: v.lines.end } };
    }
    case "commit": {
      if (!isString(v.repo) || !isString(v.rev) || !isStringOrNull(v.path) || typeof v.only !== "boolean") return null;
      const entry: SavedEntry = { view: "commit", repo: v.repo, rev: v.rev, path: v.path, only: v.only };
      if (v.compare === undefined) return entry;
      const compare = readCompare(v.compare);
      return compare && { ...entry, compare };
    }
    case "graph":
      return isString(v.repo) && isStringOrNull(v.focus) ? { view: "graph", repo: v.repo, focus: v.focus } : null;
    case "search":
      return isString(v.repo) && isStringOrNull(v.rev) && isString(v.query)
        ? { view: "search", repo: v.repo, rev: v.rev, query: v.query }
        : null;
    default:
      return null;
  }
}

function readStack(v: Json): SavedEntry[] {
  return Array.isArray(v) ? v.slice(-MAX_SAVED_STACK).map(readEntry).filter((e): e is SavedEntry => e !== null) : [];
}

/** 저장한 배치를 읽는다. 모양이 틀리면 null(처음 여는 저장소처럼 연다). */
export function readLayout(v: Json): Layout | null {
  if (!isObject(v) || !Array.isArray(v.groups) || !Array.isArray(v.panel)) return null;
  const groups: SavedGroup[] = [];
  // 읽지 못한 탭·그룹은 빼므로 저장한 자리(opener, activeGroup)를 새 자리로 옮긴다.
  const tabAt = new Map<number, number>();
  const flat: SavedTab[] = [];
  let activeGroup = 0;
  let seen = 0;
  for (const [gi, g] of v.groups.slice(0, 4).entries()) {
    if (!isObject(g) || !Array.isArray(g.tabs)) return null;
    const tabs: SavedTab[] = [];
    let active: number | null = null;
    for (const [i, t] of g.tabs.entries()) {
      const at = seen++;
      if (flat.length >= MAX_SAVED_TABS || !isObject(t)) continue;
      const stack = readStack(t.stack);
      if (stack.length === 0) continue;
      if (i === g.active) active = tabs.length;
      const tab: SavedTab = { stack, preview: t.preview === true, from: isPane(t.from) ? t.from : "editor" };
      if (typeof t.opener === "number" && Number.isInteger(t.opener)) tab.opener = t.opener;
      tabAt.set(at, flat.length);
      flat.push(tab);
      tabs.push(tab);
    }
    if (tabs.length === 0) continue;
    if (gi === v.activeGroup) activeGroup = groups.length;
    const weight = typeof g.weight === "number" && Number.isFinite(g.weight) && g.weight > 0 ? g.weight : 1;
    groups.push({ tabs, active: active ?? 0, weight });
  }
  for (const tab of flat) {
    if (tab.opener === undefined) continue;
    const opener = tabAt.get(tab.opener);
    if (opener === undefined || flat[opener] === tab) delete tab.opener;
    else tab.opener = opener;
  }
  const panel = v.panel
    .slice(-MAX_SAVED_STACK)
    .map((e: Json) => {
      const entry = readEntry(e);
      return entry && { ...entry, from: isObject(e) && isPane(e.from) ? e.from : "panel" };
    })
    .filter((e): e is SavedEntry & { from: Pane } => e !== null);
  if (groups.length === 0 && panel.length === 0) return null;
  return {
    groups: groups.length > 0 ? normalizeWeights(groups) : [{ tabs: [], active: null, weight: 1 }],
    activeGroup,
    panel,
    panelOpen: v.panelOpen === true && panel.length > 0,
    panelMax: v.panelMax === true && v.panelOpen === true && panel.length > 0,
  };
}

// 저장소: localStorage 하나에 저장소(작업트리 경로)마다 배치 하나, 최근에 연 것부터 MAX_REPOS개.
// localStorage는 막혀 있을 수 있다(사생활 보호 설정 등). 그때는 복원하지 않는다.

const STORAGE_KEY = "plx.layouts";
const MAX_REPOS = 20;
type Stored = { repo: string; layout: Json }[];

function readStored(): Stored {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return [];
    const parsed: Json = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is { repo: string; layout: Json } => isObject(item) && isString(item.repo));
  } catch {
    return [];
  }
}

/** 그 저장소에 남긴 배치 */
export function loadLayout(repo: string): Layout | null {
  const found = readStored().find((item) => item.repo === repo);
  return found ? readLayout(found.layout) : null;
}

let lastSaved: { repo: string; json: string } | null = null;

/** 배치를 남긴다(그 저장소를 맨 앞으로). `layout`이 null이면 그 저장소의 배치를 지운다(다음에는 처음처럼 연다). */
export function saveLayout(repo: string, layout: Layout | null): void {
  const json = JSON.stringify(layout);
  if (lastSaved && lastSaved.repo === repo && lastSaved.json === json) return;
  lastSaved = { repo, json };
  const rest = readStored().filter((item) => item.repo !== repo);
  const next = layout ? [{ repo, layout }, ...rest].slice(0, MAX_REPOS) : rest;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // 다음 실행에는 처음처럼 연다.
  }
}
