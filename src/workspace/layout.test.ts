import { afterEach, describe, expect, it } from "vitest";
import type { Navigation, PlxRequest } from "../nav";
import { readLayout, toLayout, type Layout } from "./layout";
import {
  activeGroupOf,
  activeTabOf,
  allTabs,
  emptyWorkspace,
  restoreWorkspace,
  setLayoutSource,
  topOf,
  workspaceReducer,
  type Place,
  type Workspace,
  type WorkspaceAction,
} from "./state";

const request = (id: number, view: PlxRequest["request"], located: Partial<PlxRequest> = {}): PlxRequest => ({
  id,
  request: view,
  label: `요청 ${id}`,
  args: [],
  cwd: "/repo",
  t0Ms: 0,
  mainMs: 0,
  recvMs: 0,
  cold: false,
  repo: "/repo",
  path: null,
  ...located,
});
const run = (state: Workspace, ...actions: WorkspaceAction[]) => actions.reduce(workspaceReducer, state);
const req = (r: PlxRequest): WorkspaceAction => ({ type: "request", request: r, receivedMs: 0 });
const nav = (navigation: Navigation, from: Place): WorkspaceAction => ({ type: "navigate", navigation, from });
const open = (path: string, extra: Partial<Extract<WorkspaceAction, { type: "open" }>> = {}): WorkspaceAction => ({
  type: "open",
  repo: "/repo",
  path,
  missing: false,
  preview: false,
  ...extra,
});
const files = (state: Workspace) =>
  state.groups.map((g) => g.tabs.map((t) => {
    const top = topOf(t);
    return top.view === "blame" ? top.target.path : top.view;
  }));

/** 탭 셋(그룹 둘), 패널에 그래프 → 커밋 상세 */
function sample(): Workspace {
  let s = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs"), open("c.rs", { side: true }));
  s = run(s, nav({ kind: "graph", repo: "/repo", rev: null }, { pane: "editor", tab: activeTabOf(s)!.key }));
  s = run(s, nav({ kind: "commit", repo: "/repo", rev: "abc", path: null }, { pane: "panel" }));
  return s;
}

afterEach(() => setLayoutSource(() => null));

describe("배치 남기기·되살리기", () => {
  it("그룹·탭·패널 스택을 그대로 되살린다", () => {
    const s = sample();
    const layout = toLayout(s)!;
    expect(layout.groups.map((g) => g.tabs.length)).toEqual([2, 1]);
    // JSON을 거쳐도 같다
    const read = readLayout(JSON.parse(JSON.stringify(layout)))!;
    expect(read).toEqual(layout);
    const restored = restoreWorkspace("/repo", read);
    expect(files(restored)).toEqual(files(s));
    expect(restored.panel.map((e) => e.view)).toEqual(["graph", "commit"]);
    expect(restored.panelOpen).toBe(true);
    // 그룹 너비 몫과 키를 받던 그룹
    expect(restored.groups.map((g) => g.weight)).toEqual(s.groups.map((g) => g.weight));
    expect(restored.groups.indexOf(activeGroupOf(restored))).toBe(s.groups.indexOf(activeGroupOf(s)));
    // 보이는 탭
    expect(restored.groups.map((g) => g.tabs.findIndex((t) => t.key === g.active))).toEqual(
      s.groups.map((g) => g.tabs.findIndex((t) => t.key === g.active)),
    );
    // 화면 key는 새로 만든다(겹치지 않는다)
    const keys = [...allTabs(restored).flatMap((t) => t.stack), ...restored.panel].map((e) => e.key);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("blame은 지금 커서 줄로 남긴다", () => {
    const s = run(emptyWorkspace("/repo"), open("a.rs"));
    const key = topOf(activeTabOf(s)!).key;
    const layout = toLayout(s, (k) => (k === key ? 42 : undefined))!;
    expect(layout.groups[0].tabs[0].stack[0]).toMatchObject({ view: "blame", line: 42 });
  });

  it("plx 요청으로 연 화면은 요청 id 없이 남긴다", () => {
    const s = run(emptyWorkspace(), req(request(1, { kind: "blame", path: "/repo/a.rs", line: 3 }, { path: "a.rs" })));
    const entry = toLayout(s)!.groups[0].tabs[0].stack[0];
    expect(entry).toEqual({ view: "blame", target: { repo: "/repo", path: "a.rs", rev: null }, line: 3, via: "file" });
  });

  it("남길 것이 없으면 null", () => {
    expect(toLayout(emptyWorkspace("/repo"))).toBeNull();
    expect(toLayout(emptyWorkspace())).toBeNull();
  });

  it("보기 탭에서 연 탭(opener)을 새 key로 잇는다", () => {
    let s = run(emptyWorkspace("/repo"), open("a.rs"));
    s = run(s, nav({ kind: "history", target: { repo: "/repo", path: "a.rs", rev: null } }, { pane: "editor", tab: activeTabOf(s)!.key }));
    s = run(s, { type: "panelToEditor" });
    const view = activeTabOf(s)!;
    s = run(s, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: "r1" }, line: 1 }, { pane: "editor", tab: view.key }));
    const opened = allTabs(s).find((t) => t.opener !== undefined)!;
    expect(opened.opener).toBe(view.key);
    const restored = restoreWorkspace("/repo", readLayout(JSON.parse(JSON.stringify(toLayout(s))))!);
    const again = allTabs(restored).find((t) => t.opener !== undefined)!;
    expect(topOf(allTabs(restored).find((t) => t.key === again.opener)!).view).toBe("history");
  });
});

describe("저장한 배치 읽기", () => {
  it("모양이 틀리면 버린다", () => {
    for (const bad of [null, 1, "x", [], {}, { groups: 1, panel: [] }, { groups: [1], panel: [] }]) {
      expect(readLayout(bad)).toBeNull();
    }
  });

  it("틀린 화면만 빼고 자리를 옮긴다", () => {
    const blame = (path: string) => ({ view: "blame", target: { repo: "/r", path, rev: null }, line: null, via: "file" });
    const layout = readLayout({
      groups: [
        { tabs: [{ stack: [{ view: "nope" }] }], active: 0, weight: 1 },
        {
          tabs: [
            { stack: [{ ...blame("a"), line: -3 }] },
            { stack: [blame("b")], preview: true, from: "panel" },
            { stack: [{ view: "history", target: { repo: "/r", path: "b", rev: null }, lines: null }] },
            { stack: [blame("c")], opener: 3 },
          ],
          active: 3,
          weight: 3,
        },
      ],
      activeGroup: 1,
      panel: [{ view: "graph", repo: "/r", focus: null, from: "editor" }, { view: "graph", repo: 5 }],
      panelOpen: true,
      panelMax: "yes",
    }) as Layout;
    expect(layout.groups).toHaveLength(1);
    expect(layout.activeGroup).toBe(0);
    const [group] = layout.groups;
    expect(group.tabs.map((t) => t.stack[0].view)).toEqual(["blame", "history", "blame"]);
    expect(group.tabs[0]).toMatchObject({ preview: true, from: "panel" });
    expect(group.active).toBe(2);
    expect(group.weight).toBe(1);
    // 저장한 자리 3(네 번째 탭 = 첫 그룹의 탭 포함)은 이제 1번(history)이다
    expect(group.tabs[2].opener).toBe(1);
    expect(layout.panel).toEqual([{ view: "graph", repo: "/r", focus: null, from: "editor" }]);
    expect(layout.panelOpen).toBe(true);
    expect(layout.panelMax).toBe(false);
  });
});

describe("저장소를 열 때", () => {
  it("다른 저장소의 요청은 그 저장소에 남긴 배치를 되살리고 같은 파일의 탭에 연다", () => {
    const saved = toLayout(sample())!;
    setLayoutSource((repo) => (repo === "/repo" ? saved : null));
    const s = run(
      emptyWorkspace("/other"),
      req(request(1, { kind: "blame", path: "/repo/b.rs", line: 9 }, { path: "b.rs" })),
    );
    expect(files(s)).toEqual([["a.rs", "b.rs"], ["c.rs"]]);
    expect(topOf(activeTabOf(s)!)).toMatchObject({ view: "blame", line: 9, target: { path: "b.rs" } });
  });

  it("콜드 스타트: 저장소를 안 뒤에 배치를 되살리고 그사이 연 화면을 둔다", () => {
    const saved = toLayout(sample())!;
    setLayoutSource((repo) => (repo === "/repo" ? saved : null));
    // 앱이 뜨기 전에 받은 요청은 저장소를 모른다
    let s = run(emptyWorkspace(), req(request(1, { kind: "blame", path: "/repo/d.rs", line: 2 }, { repo: null, cold: true })));
    const entry = topOf(activeTabOf(s)!);
    expect(s.repo).toBeNull();
    s = run(s, { type: "located", key: entry.key, repo: "/repo", path: "d.rs" });
    expect(s.repo).toBe("/repo");
    expect(files(s).flat().sort()).toEqual(["a.rs", "b.rs", "c.rs", "d.rs"]);
    // 연 화면은 그대로(같은 key)이고 키를 받는다
    expect(topOf(activeTabOf(s)!).key).toBe(entry.key);
    expect(s.focus).toBe("editor");
    expect(s.panel.map((e) => e.view)).toEqual(["graph", "commit"]);
  });

  it("콜드 스타트의 패널 요청은 패널을 그 화면으로 바꾼다", () => {
    const saved = toLayout(sample())!;
    setLayoutSource(() => saved);
    let s = run(emptyWorkspace(), req(request(1, { kind: "graph" }, { repo: null, cold: true })));
    const graph = s.panel[0];
    s = run(s, { type: "repo", repo: "/repo" });
    expect(s.panel).toEqual([graph]);
    expect(s.focus).toBe("panel");
    expect(files(s)).toEqual([["a.rs", "b.rs"], ["c.rs"]]);
  });

  it("남긴 배치가 없으면 지금처럼 연다", () => {
    const s = run(emptyWorkspace(), req(request(1, { kind: "graph" }, { repo: null })), { type: "repo", repo: "/repo" });
    expect(allTabs(s)).toHaveLength(0);
    expect(s.panel.map((e) => e.view)).toEqual(["graph"]);
  });
});
