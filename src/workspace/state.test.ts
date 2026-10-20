import { describe, expect, it } from "vitest";
import { MAX_STACK, backDirectionAt, type Navigation, type PlxRequest } from "../nav";
import {
  MAX_GROUPS,
  activeGroupOf,
  activeTabOf,
  allTabs,
  besideTarget,
  canGoBack,
  emptyWorkspace,
  focusedEntry,
  groupOfTab,
  tabFile,
  targetPane,
  topOf,
  visibleTab,
  windowTitle,
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

/** 백엔드가 찾은 저장소·경로가 있는 blame 요청 */
const blame = (id: number, path = "a.rs", line: number | null = 3) =>
  request(id, { kind: "blame", path: `/repo/${path}`, line }, { path });

const run = (state: Workspace, ...actions: WorkspaceAction[]) => actions.reduce(workspaceReducer, state);
const req = (r: PlxRequest): WorkspaceAction => ({ type: "request", request: r, receivedMs: 0 });
const nav = (navigation: Navigation, from: Place): WorkspaceAction => ({ type: "navigate", navigation, from });
const back = (from: Place): WorkspaceAction => ({ type: "back", from });

const activeTab = (state: Workspace) => activeTabOf(state)!;
const inTab = (state: Workspace): Place => ({ pane: "editor", tab: activeTab(state).key });
/** 키를 받는 그룹의 보이는 탭 key */
const active = (state: Workspace) => activeTabOf(state)?.key ?? null;
const tabs = (state: Workspace) => allTabs(state);
const PANEL: Place = { pane: "panel" };
const step = (kind: "parent" | "older" | "newer", rev: string): Navigation => ({
  kind,
  target: { repo: "/repo", path: "a.rs", rev },
  line: 1,
});

describe("plx 요청", () => {
  it("blame은 탭으로 열고, 같은 파일이면 그 탭을 새 화면 하나로 바꾼다", () => {
    let state = run(emptyWorkspace(), req(blame(1)));
    expect(state.repo).toBe("/repo");
    expect(tabs(state)).toHaveLength(1);
    expect(topOf(activeTab(state))).toMatchObject({
      view: "blame",
      target: { repo: "/repo", path: "a.rs", rev: null },
      line: 3,
      via: "request",
      request: 1,
    });
    expect(state.focus).toBe("editor");
    state = run(state, nav(step("parent", "p1"), inTab(state)));
    expect(activeTab(state).stack).toHaveLength(2);

    // 다른 파일은 새 탭(보이는 탭 뒤), 같은 파일은 그 탭으로 가서 화면을 새로 연다
    state = run(state, req(blame(2, "b.rs")));
    expect(tabs(state).map(tabFile)).toEqual(["a.rs", "b.rs"]);
    const first = tabs(state)[0];
    state = run(state, req(blame(3, "a.rs", 9)));
    expect(tabs(state)).toHaveLength(2);
    expect(active(state)).toBe(first.key);
    expect(activeTab(state).stack).toMatchObject([{ line: 9, via: "request", request: 3 }]);
    expect(activeTab(state).stack[0].key).not.toBe(first.stack[0].key);
  });

  it("이벤트와 current_request가 같은 요청을 두 번 줄 수 있다: 같거나 오래된 요청은 무시한다", () => {
    const state = run(emptyWorkspace(), req(blame(2)));
    expect(run(state, req(blame(2)))).toBe(state);
    expect(run(state, req(blame(1, "b.rs")))).toBe(state);
  });

  it("히스토리·라인 히스토리·커밋 상세·그래프는 패널에 연다", () => {
    let state = run(emptyWorkspace(), req(blame(1)));
    state = run(state, req(request(2, { kind: "log", path: "/repo/a.rs" }, { path: "a.rs" })));
    expect(state.panel).toMatchObject([
      { view: "history", target: { repo: "/repo", path: "a.rs", rev: null }, lines: null, via: "request", request: 2 },
    ]);
    expect([state.panelOpen, state.focus]).toEqual([true, "panel"]);
    expect(tabs(state)).toHaveLength(1);
    state = run(state, req(request(3, { kind: "lineLog", path: "/repo/a.rs", start: 3, end: 9 }, { path: "a.rs" })));
    expect(state.panel).toMatchObject([{ view: "history", lines: { start: 3, end: 9 } }]);
    state = run(state, req(request(4, { kind: "show", rev: "HEAD~1" }, { cwd: "/repo/sub" })));
    expect(state.panel).toMatchObject([{ view: "commit", repo: "/repo", rev: "HEAD~1", path: null, only: false }]);
    state = run(state, req(request(5, { kind: "graph" })));
    expect(state.panel).toMatchObject([{ view: "graph", repo: "/repo", focus: null, via: "request" }]);
    expect(windowTitle(state)).toBe("요청 5");
  });

  it("다른 저장소의 요청은 작업 공간을 새로 연다", () => {
    let state = run(emptyWorkspace(), req(blame(1)), req(request(2, { kind: "graph" })));
    state = run(state, req(request(3, { kind: "graph" }, { repo: "/other", cwd: "/other" })));
    expect(state.repo).toBe("/other");
    expect(tabs(state)).toEqual([]);
    expect(state.panel).toMatchObject([{ view: "graph", repo: "/other" }]);
  });

  it("콜드 스타트 요청은 절대 경로로 열고, 화면이 알린 저장소·경로로 고친다", () => {
    let state = run(emptyWorkspace(), req(blame(1, "a.rs", null)));
    const cold = request(2, { kind: "blame", path: "/repo/src/한글.rs", line: 4 }, { repo: null, path: null, cold: true });
    state = run(emptyWorkspace(), req(cold));
    expect(state.repo).toBeNull();
    const entry = topOf(activeTab(state));
    expect(entry).toMatchObject({ view: "blame", target: { repo: null, path: "/repo/src/한글.rs", rev: null } });
    expect(tabFile(activeTab(state))).toBeNull();
    state = run(state, { type: "located", key: entry.key, repo: "/repo", path: "src/한글.rs" });
    expect(state.repo).toBe("/repo");
    expect(topOf(activeTab(state))).toMatchObject({ key: entry.key, target: { repo: "/repo", path: "src/한글.rs", rev: null } });
    // 이제 같은 파일의 요청이 그 탭을 찾는다
    state = run(state, req(blame(3, "src/한글.rs")));
    expect(tabs(state)).toHaveLength(1);
    // 저장소를 이미 알면 그대로
    expect(run(state, { type: "repo", repo: "/x" }).repo).toBe("/repo");
  });
});

describe("탭 안 탐색", () => {
  it("뒤로는 탭 맨 아래에서 멈춘다(그다음은 창 숨기기)", () => {
    let state = run(emptyWorkspace(), req(blame(1)));
    state = run(state, nav(step("parent", "p1"), inTab(state)), nav(step("older", "o1"), inTab(state)));
    expect(activeTab(state).stack.map((e) => e.via)).toEqual(["request", "parent", "older"]);
    state = run(state, back(inTab(state)), back(inTab(state)));
    expect(activeTab(state).stack.map((e) => e.via)).toEqual(["request"]);
    expect(canGoBack(state, inTab(state))).toBe(false);
    expect(run(state, back(inTab(state)))).toBe(state);
  });

  it("너무 깊어지면 가장 아래 화면부터 버리고, 맨 아래가 된 `<` 화면의 `>`는 새로 연다", () => {
    let state = run(emptyWorkspace(), req(blame(1)));
    for (let i = 0; i < MAX_STACK + 5; i++) state = run(state, nav(step("older", `o${i}`), inTab(state)));
    const { stack } = activeTab(state);
    expect(stack).toHaveLength(MAX_STACK);
    expect(stack[MAX_STACK - 1]).toMatchObject({ target: { rev: `o${MAX_STACK + 4}` } });
    expect(backDirectionAt(stack, 0)).toBeUndefined();
    expect(backDirectionAt(stack, 1)).toBe("newer");
  });

  it("탭 안에서 옮겨 가면 미리보기 탭이 고정된다", () => {
    let state = run(emptyWorkspace("/repo"), { type: "open", repo: "/repo", path: "a.rs", missing: false, preview: true });
    expect(activeTab(state).preview).toBe(true);
    state = run(state, nav(step("parent", "p1"), inTab(state)));
    expect(activeTab(state).preview).toBe(false);
  });
});

describe("에디터와 패널", () => {
  it("blame → 히스토리(패널) → 커밋 상세(그 파일만) → 그 리비전 blame(미리보기 탭) → q로 패널에 돌아온다", () => {
    let state = run(emptyWorkspace(), req(blame(1)));
    const editor = inTab(state);
    state = run(state, nav({ kind: "history", target: { repo: "/repo", path: "a.rs", rev: "r1" } }, editor));
    expect(state.panel).toMatchObject([{ view: "history", via: "history", from: "editor" }]);
    expect(state.focus).toBe("panel");
    state = run(state, nav({ kind: "commit", repo: "/repo", rev: "c1", path: "a.rs", only: true }, PANEL));
    expect(state.panel.map((e) => [e.view, e.from])).toEqual([
      ["history", "editor"],
      ["commit", "panel"],
    ]);
    expect(state.panel[1]).toMatchObject({ only: true, path: "a.rs" });
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: "c1" }, line: null }, PANEL));
    expect(tabs(state)).toHaveLength(2);
    expect(activeTab(state)).toMatchObject({ preview: true, from: "panel" });
    expect(state.focus).toBe("editor");
    // 패널에서 연 다른 파일은 그 미리보기 탭 자리에 연다
    const preview = activeTab(state).key;
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "b.rs", rev: "c1" }, line: null }, PANEL));
    expect(tabs(state)).toHaveLength(2);
    expect(activeTab(state).key).not.toBe(preview);
    expect(topOf(activeTab(state))).toMatchObject({ target: { path: "b.rs" } });
    // 맨 아래에서 q: 탭을 닫고 패널로
    expect(canGoBack(state, inTab(state))).toBe(true);
    state = run(state, back(inTab(state)));
    expect(tabs(state)).toHaveLength(1);
    expect(state.focus).toBe("panel");
    expect(focusedEntry(state)).toMatchObject({ view: "commit" });
  });

  it("에디터에서 연 패널 화면을 q로 빼면 에디터로 돌아가고, 패널 맨 아래에서 q면 패널을 닫는다", () => {
    let state = run(emptyWorkspace(), req(blame(1)));
    state = run(state, nav({ kind: "history", target: { repo: "/repo", path: "a.rs", rev: null } }, inTab(state)));
    state = run(state, { type: "focus", focus: "editor" });
    // 패널이 열려 있으면 에디터에서 연 화면도 패널 스택에 쌓는다
    state = run(state, nav({ kind: "commit", repo: "/repo", rev: "c1", path: "a.rs" }, inTab(state)));
    expect(state.panel.map((e) => e.view)).toEqual(["history", "commit"]);
    state = run(state, back(PANEL));
    expect([state.panel.length, state.focus]).toEqual([1, "editor"]);
    state = run(state, back(PANEL));
    expect([state.panel.length, state.panelOpen, state.focus]).toEqual([0, false, "editor"]);
    // 닫힌 패널에 여는 화면은 새로 시작한다
    state = run(state, nav({ kind: "graph", repo: "/repo", rev: "c1" }, inTab(state)));
    expect(state.panel).toMatchObject([{ view: "graph", focus: "c1", via: "graph" }]);
  });

  it("탭이 없으면 패널 맨 아래에서 q는 창을 숨긴다", () => {
    const state = run(emptyWorkspace(), req(request(1, { kind: "graph" })));
    expect(canGoBack(state, PANEL)).toBe(false);
    expect(run(state, back(PANEL))).toBe(state);
  });

  it("Mod+J: 패널 닫기·열기, 빈 패널은 그래프로 연다", () => {
    let state = run(emptyWorkspace(), req(blame(1)));
    state = run(state, { type: "togglePanel" });
    expect(state.panel).toMatchObject([{ view: "graph", repo: "/repo", via: "graph" }]);
    expect([state.panelOpen, state.focus]).toEqual([true, "panel"]);
    const graph = state.panel[0];
    state = run(state, { type: "togglePanel" });
    expect([state.panelOpen, state.focus]).toEqual([false, "editor"]);
    state = run(state, { type: "togglePanel" });
    expect(state.panel).toEqual([graph]);
    // 저장소를 모르면 빈 패널을 열지 않는다
    const unknown = emptyWorkspace();
    expect(run(unknown, { type: "togglePanel" })).toBe(unknown);
  });

  it("패널 머리줄의 앞 화면으로 가기, 최대화", () => {
    let state = run(emptyWorkspace(), req(request(1, { kind: "graph" })));
    state = run(state, nav({ kind: "commit", repo: "/repo", rev: "c1", path: null }, PANEL));
    state = run(state, nav({ kind: "history", target: { repo: "/repo", path: "a.rs", rev: "c1" } }, PANEL));
    state = run(state, { type: "panelTo", key: state.panel[0].key });
    expect(state.panel.map((e) => e.view)).toEqual(["graph"]);
    state = run(state, { type: "maximizePanel" });
    expect(state.panelMax).toBe(true);
    // 에디터에 탭을 열면 최대화를 푼다
    state = run(state, { type: "open", repo: "/repo", path: "a.rs", missing: false, preview: false });
    expect(state.panelMax).toBe(false);
  });
});

describe("탭", () => {
  const open = (path: string, preview = false, missing = false): WorkspaceAction => ({
    type: "open",
    repo: "/repo",
    path,
    missing,
    preview,
  });

  it("트리에서 연 파일: 미리보기는 한 자리를 쓰고, 이미 연 파일은 그 탭으로 간다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs", true));
    expect(tabs(state).map((t) => [tabFile(t), t.preview])).toEqual([
      ["a.rs", false],
      ["b.rs", true],
    ]);
    state = run(state, { type: "activate", tab: tabs(state)[0].key }, open("c.rs", true));
    expect(tabs(state).map((t) => [tabFile(t), t.preview])).toEqual([
      ["a.rs", false],
      ["c.rs", true],
    ]);
    // 이미 연 파일을 고정으로 열면 그 탭을 고정한다
    state = run(state, open("c.rs"));
    expect(tabs(state).map((t) => [tabFile(t), t.preview])).toEqual([
      ["a.rs", false],
      ["c.rs", false],
    ]);
    // 지운 파일은 HEAD 버전
    state = run(state, open("gone.rs", false, true));
    expect(topOf(activeTab(state))).toMatchObject({ target: { repo: "/repo", path: "gone.rs", rev: "HEAD" } });
  });

  it("같은 화면을 줄과 함께 열면 그 줄로 다시 연다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"));
    const before = topOf(activeTab(state));
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: null }, line: 12 }, inTab(state)));
    expect(tabs(state)).toHaveLength(1);
    expect(topOf(activeTab(state))).toMatchObject({ line: 12 });
    expect(topOf(activeTab(state)).key).not.toBe(before.key);
    const same = topOf(activeTab(state));
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: null }, line: null }, inTab(state)));
    expect(topOf(activeTab(state))).toBe(same);
  });

  it("패널에서 Quick Open으로 연 파일은 미리보기가 아닌 탭이고, q는 패널로 돌아간다", () => {
    let state = run(emptyWorkspace("/repo"), { type: "togglePanel" });
    const quick: Navigation = { kind: "file", target: { repo: "/repo", path: "a.rs", rev: null }, line: 4, pinned: true };
    state = run(state, nav(quick, PANEL));
    expect(activeTab(state)).toMatchObject({ preview: false, from: "panel" });
    expect(state.focus).toBe("editor");
    state = run(state, back(inTab(state)));
    expect([tabs(state).length, state.focus]).toEqual([0, "panel"]);
  });

  it("닫으면 그 앞에 본 탭으로, 마지막 탭을 닫으면 패널이나 빈 화면", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs"), open("c.rs"));
    const [a, b, c] = tabs(state).map((t) => t.key);
    expect(tabs(state).map(tabFile)).toEqual(["a.rs", "b.rs", "c.rs"]);
    state = run(state, { type: "activate", tab: a }, { type: "activate", tab: c });
    state = run(state, { type: "close", tab: c });
    expect(active(state)).toBe(a);
    state = run(state, { type: "cycle", delta: 1 });
    expect(active(state)).toBe(b);
    state = run(state, { type: "cycle", delta: 1 });
    expect(active(state)).toBe(a);
    state = run(state, { type: "cycle", delta: -1 });
    expect(active(state)).toBe(b);
    state = run(state, { type: "togglePanel" }, { type: "focus", focus: "editor" });
    state = run(state, { type: "close", tab: b }, { type: "close", tab: a });
    expect([active(state), state.focus]).toEqual([null, "panel"]);
    state = run(state, { type: "togglePanel" });
    expect(state.focus).toBe("editor");
    expect(windowTitle(state)).toBe("repo");
  });

  it("탭 메뉴: 다른 탭 닫기는 그 탭을 보이고, 모든 탭 닫기는 패널이나 빈 화면", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs"), open("c.rs"));
    const [a, b] = tabs(state).map((t) => t.key);
    state = run(state, { type: "closeOthers", tab: b });
    expect([tabs(state).map(tabFile), active(state), activeGroupOf(state).recent]).toEqual([["b.rs"], b, [b]]);
    expect(run(state, { type: "closeOthers", tab: a })).toBe(state);
    expect(run(state, { type: "closeOthers", tab: b })).toBe(state);
    state = run(state, open("d.rs"), { type: "togglePanel" }, { type: "focus", focus: "editor" }, { type: "closeAll" });
    expect([tabs(state), active(state), activeGroupOf(state).recent, state.focus]).toEqual([[], null, [], "panel"]);
    expect(run(state, { type: "closeAll" })).toBe(state);
  });

  it("도구 막대가 명령을 보내는 칸: 키를 받는 칸, 트리면 보이는 화면", () => {
    let state = emptyWorkspace("/repo");
    expect(targetPane(state)).toBeNull();
    state = run(state, open("a.rs"));
    expect(targetPane(state)).toBe("editor");
    state = run(state, { type: "focus", focus: "tree" });
    expect(targetPane(state)).toBe("editor");
    state = run(state, { type: "togglePanel" });
    expect(targetPane(state)).toBe("panel");
    state = run(state, { type: "focus", focus: "tree" });
    expect(targetPane(state)).toBe("editor");
    // 패널이 에디터를 가리면 패널, 탭이 없으면 패널
    state = run(state, { type: "maximizePanel" });
    expect(targetPane(state)).toBe("panel");
    state = run(state, { type: "maximizePanel" }, { type: "closeAll" });
    expect([state.focus, targetPane(state)]).toEqual(["tree", "panel"]);
    state = run(state, { type: "togglePanel" });
    expect(targetPane(state)).toBeNull();
  });

  it("다시 열기(reload)는 그 자리의 화면만 새 key로 바꾼다", () => {
    let state = run(emptyWorkspace(), req(blame(1)));
    state = run(state, nav({ kind: "history", target: { repo: "/repo", path: "a.rs", rev: null }, lines: { start: 40, end: 50 } }, inTab(state)));
    const root = topOf(activeTab(state));
    const lines = state.panel[0];
    const search = { query: { search: "fn", caseSensitive: false, regexp: false, wholeWord: true }, open: false };
    state = run(state, { type: "reload", key: root.key, line: 42, search });
    expect(topOf(activeTab(state))).toMatchObject({ view: "blame", line: 42, via: "request", search });
    expect(topOf(activeTab(state)).key).not.toBe(root.key);
    expect(state.panel[0]).toBe(lines);
    state = run(state, { type: "reload", key: lines.key, lines: { start: 50, end: 60 }, query: "수정" });
    expect(state.panel[0]).toMatchObject({ view: "history", lines: { start: 50, end: 60 }, query: "수정" });
    // 줄 없이 다시 열면 앞 줄 그대로, 없는 화면은 그대로
    const again = run(state, { type: "reload", key: topOf(activeTab(state)).key });
    expect(topOf(activeTab(again))).toMatchObject({ line: 42 });
    expect(run(again, { type: "reload", key: 99999 })).toBe(again);
  });
});

describe("워크트리", () => {
  it("다른 워크트리로 옮기면 그 워크트리의 작업 공간에서 같은 화면을 연다", () => {
    let state = run(emptyWorkspace(), req(blame(1)));
    state = run(
      state,
      nav({ kind: "file", target: { repo: "/w/에이전트", path: "a.rs", rev: null }, line: 5, worktree: true }, inTab(state)),
    );
    expect(state.repo).toBe("/w/에이전트");
    expect(tabs(state)).toHaveLength(1);
    expect(topOf(activeTab(state))).toMatchObject({ target: { repo: "/w/에이전트", path: "a.rs" }, line: 5 });
    state = run(state, nav({ kind: "graph", repo: "/repo", rev: "c1", worktree: true }, PANEL));
    expect(state.repo).toBe("/repo");
    expect([tabs(state).length, state.panel.length, state.focus]).toEqual([0, 1, "panel"]);
  });
});

describe("에디터 그룹", () => {
  const open = (path: string, more: Partial<Extract<WorkspaceAction, { type: "open" }>> = {}): WorkspaceAction => ({
    type: "open",
    repo: "/repo",
    path,
    missing: false,
    preview: false,
    ...more,
  });
  const files = (state: Workspace) => state.groups.map((group) => group.tabs.map(tabFile));
  const history = (path: string): Navigation => ({ kind: "history", target: { repo: "/repo", path, rev: null } });

  it("나누기: 보이는 화면을 오른쪽 새 그룹에 하나 더 연다(그 줄로). 그룹은 넷까지다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs"));
    const [left] = state.groups;
    const b = activeTab(state);
    state = run(state, { type: "split", tab: b.key, side: "right", line: 40 });
    expect(files(state)).toEqual([["a.rs", "b.rs"], ["b.rs"]]);
    expect(state.groups.map((g) => g.weight)).toEqual([0.5, 0.5]);
    expect(state.activeGroup).toBe(state.groups[1].key);
    expect(topOf(activeTab(state))).toMatchObject({ view: "blame", target: { path: "b.rs", rev: null }, line: 40 });
    expect(topOf(activeTab(state)).key).not.toBe(topOf(b).key);
    // 왼쪽 그룹은 그대로
    expect(visibleTab(state.groups[0])?.key).toBe(b.key);
    expect(state.groups[0].key).toBe(left.key);
    for (let i = 0; i < 4; i++) state = run(state, { type: "split", tab: activeTab(state).key, side: "right" });
    expect(state.groups).toHaveLength(MAX_GROUPS);
    // 넷이면 더 나누지 않고 옆 그룹(오른쪽 끝이면 왼쪽)에 연다
    expect(state.groups.map((g) => g.tabs.length)).toEqual([2, 1, 2, 2]);
  });

  it("탭 옮기기: 다른 그룹·그룹 옆(새 그룹)·그룹 안. 옮긴 탭은 고정되고, 빈 그룹은 빠진다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs"), open("c.rs", { preview: true }));
    const [a, b, c] = tabs(state).map((t) => t.key);
    const first = state.groups[0].key;
    state = run(state, { type: "moveTab", tab: c, to: { kind: "split", group: first, side: "right" } });
    expect(files(state)).toEqual([["a.rs", "b.rs"], ["c.rs"]]);
    expect([active(state), activeTab(state).preview]).toEqual([c, false]);
    const second = state.groups[1].key;
    // 왼쪽에 새 그룹
    state = run(state, { type: "moveTab", tab: a, to: { kind: "split", group: first, side: "left" } });
    expect(files(state)).toEqual([["a.rs"], ["b.rs"], ["c.rs"]]);
    expect(state.groups.map((g) => g.weight)).toEqual([0.25, 0.25, 0.5]);
    // 다른 그룹의 자리 앞으로: 빈 그룹이 빠지고 그 그룹이 키를 받는다
    state = run(state, { type: "moveTab", tab: a, to: { kind: "group", group: first, index: 0 } });
    expect(files(state)).toEqual([["a.rs", "b.rs"], ["c.rs"]]);
    expect([state.activeGroup, active(state)]).toEqual([first, a]);
    // 그룹 안에서 자리 바꾸기(옮기기 전 목록의 자리). 제 그룹의 화면 칸에 놓으면 그대로다.
    state = run(state, { type: "moveTab", tab: a, to: { kind: "group", group: first, index: 2 } });
    expect(files(state)).toEqual([["b.rs", "a.rs"], ["c.rs"]]);
    expect(files(run(state, { type: "moveTab", tab: b, to: { kind: "group", group: first } }))).toEqual(files(state));
    // 혼자 있는 탭을 제 그룹 옆으로 옮기면 그대로다
    const alone = run(state, { type: "moveTab", tab: c, to: { kind: "split", group: second, side: "left" } });
    expect(files(alone)).toEqual(files(state));
    expect(active(alone)).toBe(c);
    // 그룹 끝으로(자리를 주지 않으면 보이는 탭 뒤)
    state = run(state, { type: "moveTab", tab: c, to: { kind: "group", group: first } });
    expect(files(state)).toEqual([["b.rs", "a.rs", "c.rs"]]);
    expect(groupOfTab(state, b)?.key).toBe(first);
  });

  it("닫기: 그룹의 마지막 탭을 닫으면 그룹이 빠지고 왼쪽 그룹이 키를 받는다. 모두 닫기는 그 그룹만", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs"));
    state = run(state, { type: "split", tab: activeTab(state).key, side: "right" }, open("c.rs"));
    expect(files(state)).toEqual([["a.rs", "b.rs"], ["b.rs", "c.rs"]]);
    const [left, right] = state.groups;
    state = run(state, { type: "closeAll", group: right.key });
    expect(files(state)).toEqual([["a.rs", "b.rs"]]);
    expect([state.activeGroup, state.focus]).toEqual([left.key, "editor"]);
    state = run(state, { type: "split", tab: activeTab(state).key, side: "left" });
    expect(files(state)).toEqual([["b.rs"], ["a.rs", "b.rs"]]);
    state = run(state, { type: "close", tab: activeTab(state).key });
    expect(files(state)).toEqual([["a.rs", "b.rs"]]);
    expect(state.activeGroup).toBe(left.key);
    // 그룹이 하나면 빈 그룹으로 남는다
    state = run(state, { type: "closeAll" });
    expect([state.groups.length, files(state), active(state)]).toEqual([1, [[]], null]);
  });

  it("Mod+1~4·그룹 누르기: 그 그룹이 키를 받고, Ctrl+Tab은 그 그룹 안에서 돈다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs"));
    state = run(state, { type: "split", tab: activeTab(state).key, side: "right" }, open("c.rs"));
    const [left, right] = state.groups;
    state = run(state, { type: "focus", focus: "tree" }, { type: "focusGroupAt", index: 0 });
    expect([state.activeGroup, state.focus, tabFile(activeTab(state))]).toEqual([left.key, "editor", "b.rs"]);
    state = run(state, { type: "cycle", delta: 1 });
    expect([state.activeGroup, tabFile(activeTab(state))]).toEqual([left.key, "a.rs"]);
    expect(run(state, { type: "focusGroupAt", index: 3 })).toBe(state);
    state = run(state, { type: "focusGroup", group: right.key });
    expect(tabFile(activeTab(state))).toBe("c.rs");
    expect(run(state, { type: "focusGroup", group: right.key })).toBe(state);
    expect(targetPane(state)).toBe("editor");
    expect(windowTitle(state)).toBe("blame c.rs");
  });

  it("파일은 보기 화면을 가리지 않는다: 히스토리 탭이 보이는 그룹에서 연 파일은 옆 그룹에, 옆에 열기는 오른쪽 그룹에", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"));
    state = run(state, nav(history("a.rs"), inTab(state)), { type: "panelToEditor", to: { kind: "split", group: state.groups[0].key, side: "right" } });
    const [left, right] = state.groups;
    expect(topOf(activeTab(state))).toMatchObject({ view: "history" });
    const view = inTab(state);
    // 히스토리 탭에서 연 리비전은 왼쪽 그룹의 미리보기 탭으로
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: "r1" }, line: 3 }, view));
    expect(state.activeGroup).toBe(left.key);
    expect(activeTab(state)).toMatchObject({ preview: true, from: "editor" });
    expect(visibleTab(state.groups[1])).toMatchObject({ key: (view as { tab: number }).tab });
    // 트리에서 연 파일도 키를 받는 그룹에 보기 화면이 보이면 옆 그룹에
    state = run(state, { type: "focusGroup", group: right.key }, open("b.rs"));
    expect([state.activeGroup, tabFile(activeTab(state))]).toEqual([left.key, "b.rs"]);
    // 옆에 열기: 오른쪽 그룹(이미 열린 파일이 다른 그룹에 있어도 그 그룹에 연다), 없으면 새 그룹
    state = run(state, open("b.rs", { side: true }));
    expect(files(state)).toEqual([
      ["a.rs", null, "b.rs"],
      [null, "b.rs"],
    ]);
    state = run(state, open("c.rs", { side: true }));
    expect(state.groups).toHaveLength(3);
    expect(files(state)[2]).toEqual(["c.rs"]);
  });

  it("보기 탭에서 연 파일: 옆 그룹이 없으면 그 그룹의 미리보기 탭이고, 맨 아래에서 q면 닫고 보기 탭으로 돌아간다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"));
    state = run(state, nav(history("a.rs"), inTab(state)), { type: "panelToEditor" });
    const view = activeTab(state);
    expect(topOf(view).view).toBe("history");
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: "r1" }, line: 3 }, inTab(state)));
    expect(state.groups).toHaveLength(1);
    expect(activeTab(state)).toMatchObject({ preview: true, from: "editor", opener: view.key });
    expect(canGoBack(state, inTab(state))).toBe(true);
    state = run(state, back(inTab(state)));
    expect([active(state), state.focus, files(state)]).toEqual([view.key, "editor", [["a.rs", null]]]);
    // 보기 탭을 닫았으면 돌아갈 곳이 없다(창 숨기기)
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: "r2" }, line: 3 }, inTab(state)));
    const file = activeTab(state);
    state = run(state, { type: "close", tab: view.key });
    expect(canGoBack(state, { pane: "editor", tab: file.key })).toBe(false);
  });

  it("파일을 연 화면이 탭과 패널 사이로 옮겨 가도 맨 아래에서 q면 그 화면으로 돌아간다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"));
    state = run(state, nav(history("a.rs"), inTab(state)), { type: "panelToEditor" });
    const view = activeTab(state);
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: "r1" }, line: 3 }, inTab(state)));
    const file: Place = inTab(state);
    // 보기 탭을 패널로 옮기면 그 탭에서 연 파일은 패널로 돌아간다
    state = run(state, { type: "tabToPanel", tab: view.key });
    expect(canGoBack(state, file)).toBe(true);
    state = run(state, back(file));
    expect([files(state), state.focus, focusedEntry(state)?.view]).toEqual([[["a.rs"]], "panel", "history"]);
    // 패널에서 연 파일은 패널의 마지막 화면을 탭으로 옮기면 그 탭으로 돌아간다
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: "r2" }, line: 3 }, PANEL));
    const fromPanel: Place = inTab(state);
    state = run(state, { type: "panelToEditor" });
    const moved = activeTab(state);
    expect([state.panelOpen, topOf(moved).view]).toEqual([false, "history"]);
    state = run(state, { type: "activate", tab: (fromPanel as { tab: number }).tab });
    expect(canGoBack(state, fromPanel)).toBe(true);
    state = run(state, back(fromPanel));
    expect([active(state), files(state)]).toEqual([moved.key, [["a.rs", null]]]);
  });

  it("plx 요청은 다른 그룹에 열린 같은 파일의 탭도 찾는다", () => {
    let state = run(emptyWorkspace(), req(blame(1, "a.rs")));
    state = run(state, req(blame(2, "b.rs")));
    state = run(state, { type: "moveTab", tab: activeTab(state).key, to: { kind: "split", group: state.groups[0].key, side: "right" } });
    state = run(state, { type: "focusGroupAt", index: 0 }, req(blame(3, "b.rs", 7)));
    expect(files(state)).toEqual([["a.rs"], ["b.rs"]]);
    expect([state.activeGroup, topOf(activeTab(state))]).toMatchObject([state.groups[1].key, { line: 7, request: 3 }]);
  });

  it("탭 메뉴의 옆 그룹으로: 옆 그룹, 없으면 그쪽에 나누기. 빈 그룹 옆에는 나누지 않는다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs"));
    const [a, b] = tabs(state).map((t) => t.key);
    const first = state.groups[0].key;
    expect(besideTarget(state, b, "right")).toEqual({ kind: "split", group: first, side: "right" });
    state = run(state, { type: "moveTab", tab: b, to: besideTarget(state, b, "right")! });
    expect(files(state)).toEqual([["a.rs"], ["b.rs"]]);
    expect(besideTarget(state, b, "left")).toEqual({ kind: "group", group: first });
    // 그룹에 혼자인 탭은 그쪽에 나눠도 그대로다
    expect([besideTarget(state, b, "right"), besideTarget(state, a, "left"), besideTarget(state, 999, "left")]).toEqual([
      null,
      null,
      null,
    ]);
    // 그룹이 넷이면 끝에서 더 나누지 않는다
    for (let i = 0; i < 2; i++) state = run(state, { type: "split", tab: b, side: "right" });
    state = run(state, { type: "focusGroupAt", index: MAX_GROUPS - 1 }, open("c.rs"));
    expect(state.groups).toHaveLength(MAX_GROUPS);
    expect(besideTarget(state, activeTab(state).key, "right")).toBeNull();
    // 빈 그룹(하나뿐)에 옆에 열기·옆으로 놓기는 그 그룹에 연다
    expect(run(emptyWorkspace("/repo"), open("c.rs", { side: true })).groups).toHaveLength(1);
    const graph = run(emptyWorkspace(), req(request(1, { kind: "graph" })));
    const moved = run(graph, { type: "panelToEditor", to: { kind: "split", group: graph.groups[0].key, side: "right" } });
    expect([moved.groups.length, topOf(activeTab(moved)).view]).toEqual([1, "graph"]);
  });

  it("그룹 너비: 개수가 맞고 0보다 큰 값만 받는다. 빠진 그룹의 너비는 왼쪽(없으면 오른쪽) 그룹이 받는다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"));
    state = run(state, { type: "split", tab: activeTab(state).key, side: "right" });
    state = run(state, { type: "resizeGroups", weights: [0.3, 0.7] });
    expect(state.groups.map((g) => g.weight)).toEqual([0.3, 0.7]);
    expect(run(state, { type: "resizeGroups", weights: [1] })).toBe(state);
    expect(run(state, { type: "resizeGroups", weights: [0, 1] })).toBe(state);
    const [left, right] = state.groups.map((g) => g.tabs[0].key);
    expect(run(state, { type: "close", tab: right }).groups.map((g) => g.weight)).toEqual([1]);
    expect(run(state, { type: "close", tab: left }).groups.map((g) => g.weight)).toEqual([1]);
    expect(run(state, { type: "tabToPanel", tab: right }).groups.map((g) => g.weight)).toEqual([1]);
  });
});

describe("탭 ↔ 패널", () => {
  it("패널 화면을 탭으로: 보이는 화면만 탭이 되고 패널에는 그 아래 화면이 남는다. 보기 탭에서 연 커밋 상세는 패널에", () => {
    let state = run(emptyWorkspace(), req(request(1, { kind: "graph" })));
    state = run(state, nav({ kind: "commit", repo: "/repo", rev: "c1", path: null }, PANEL));
    const [graph, commit] = state.panel;
    // 끄는 사이 놓을 그룹이 없어졌으면(그 그룹의 탭을 닫았다) 옮기지 않는다
    expect(run(state, { type: "panelToEditor", to: { kind: "group", group: -1 } })).toBe(state);
    state = run(state, { type: "panelToEditor" });
    expect(activeTab(state).stack.map((e) => e.key)).toEqual([commit.key]);
    expect([state.panel.map((e) => e.key), state.panelOpen, state.focus]).toEqual([[graph.key], true, "editor"]);
    expect(tabFile(activeTab(state))).toBeNull();
    expect(windowTitle(state)).toBe("커밋 c1");
    // 그 탭의 맨 아래에서 q는 탭을 닫고 패널(그래프)로 돌아간다
    expect(canGoBack(state, inTab(state))).toBe(true);
    state = run(state, back(inTab(state)));
    expect([tabs(state), state.focus, focusedEntry(state)?.key]).toEqual([[], "panel", graph.key]);
    // 패널의 마지막 화면을 옮기면 패널은 닫힌다
    state = run(state, { type: "panelToEditor" });
    expect([state.panel, state.panelOpen, topOf(activeTab(state)).key]).toEqual([[], false, graph.key]);
    expect(canGoBack(state, inTab(state))).toBe(false);
    // 보기 탭에서 연 커밋 상세는 패널에
    state = run(state, nav({ kind: "commit", repo: "/repo", rev: "c2", path: null }, inTab(state)));
    expect(state.panel).toMatchObject([{ view: "commit", rev: "c2", from: "editor" }]);
    expect(state.focus).toBe("panel");
    // 빈 패널은 옮기지 않는다
    const empty = emptyWorkspace("/repo");
    expect(run(empty, { type: "panelToEditor" })).toBe(empty);
  });

  it("탭을 패널로: 그 스택을 패널 위에 쌓고, 패널의 blame도 탭에서처럼 쌓인다", () => {
    let state = run(emptyWorkspace(), req(blame(1)), req(request(2, { kind: "graph" })));
    const file = activeGroupOf(state).tabs[0];
    state = run(state, { type: "tabToPanel", tab: file.key });
    expect(tabs(state)).toEqual([]);
    expect(state.panel.map((e) => [e.view, e.from])).toEqual([
      ["graph", "panel"],
      ["blame", "editor"],
    ]);
    expect([state.panelOpen, state.focus]).toEqual([true, "panel"]);
    expect(focusedEntry(state)).toMatchObject({ view: "blame", key: file.stack[0].key });
    // 패널 blame의 `,`는 패널 스택에, 리비전 파일은 탭에
    state = run(state, nav(step("parent", "p1"), PANEL));
    expect(state.panel.map((e) => e.view)).toEqual(["graph", "blame", "blame"]);
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: "p1" }, line: 2 }, PANEL));
    expect(activeTab(state)).toMatchObject({ preview: true, from: "panel" });
    expect(state.panel).toHaveLength(3);
  });
});
