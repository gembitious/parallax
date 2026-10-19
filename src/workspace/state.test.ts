import { describe, expect, it } from "vitest";
import { MAX_STACK, backDirectionAt, type Navigation, type PlxRequest } from "../nav";
import {
  canGoBack,
  emptyWorkspace,
  focusedEntry,
  tabFile,
  topOf,
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

const activeTab = (state: Workspace) => state.tabs.find((tab) => tab.key === state.active)!;
const inTab = (state: Workspace): Place => ({ pane: "editor", tab: state.active! });
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
    expect(state.tabs).toHaveLength(1);
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
    expect(state.tabs.map(tabFile)).toEqual(["a.rs", "b.rs"]);
    const first = state.tabs[0];
    state = run(state, req(blame(3, "a.rs", 9)));
    expect(state.tabs).toHaveLength(2);
    expect(state.active).toBe(first.key);
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
    expect(state.tabs).toHaveLength(1);
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
    expect(state.tabs).toEqual([]);
    expect(state.panel).toMatchObject([{ view: "graph", repo: "/other" }]);
  });

  it("콜드 스타트 요청은 절대 경로로 열고, 화면이 알린 저장소·경로로 고친다", () => {
    let state = run(emptyWorkspace(), req(blame(1, "a.rs", null)));
    const cold = request(2, { kind: "blame", path: "/repo/src/한글.rs", line: 4 }, { repo: null, path: null, cold: true });
    state = run(emptyWorkspace(), req(cold));
    expect(state.repo).toBeNull();
    const entry = topOf(activeTab(state));
    expect(entry.target).toEqual({ repo: null, path: "/repo/src/한글.rs", rev: null });
    expect(tabFile(activeTab(state))).toBeNull();
    state = run(state, { type: "located", key: entry.key, repo: "/repo", path: "src/한글.rs" });
    expect(state.repo).toBe("/repo");
    expect(topOf(activeTab(state))).toMatchObject({ key: entry.key, target: { repo: "/repo", path: "src/한글.rs", rev: null } });
    // 이제 같은 파일의 요청이 그 탭을 찾는다
    state = run(state, req(blame(3, "src/한글.rs")));
    expect(state.tabs).toHaveLength(1);
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
    expect(state.tabs).toHaveLength(2);
    expect(activeTab(state)).toMatchObject({ preview: true, from: "panel" });
    expect(state.focus).toBe("editor");
    // 패널에서 연 다른 파일은 그 미리보기 탭 자리에 연다
    const preview = activeTab(state).key;
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "b.rs", rev: "c1" }, line: null }, PANEL));
    expect(state.tabs).toHaveLength(2);
    expect(activeTab(state).key).not.toBe(preview);
    expect(topOf(activeTab(state)).target.path).toBe("b.rs");
    // 맨 아래에서 q: 탭을 닫고 패널로
    expect(canGoBack(state, inTab(state))).toBe(true);
    state = run(state, back(inTab(state)));
    expect(state.tabs).toHaveLength(1);
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
    expect(state.tabs.map((t) => [tabFile(t), t.preview])).toEqual([
      ["a.rs", false],
      ["b.rs", true],
    ]);
    state = run(state, { type: "activate", tab: state.tabs[0].key }, open("c.rs", true));
    expect(state.tabs.map((t) => [tabFile(t), t.preview])).toEqual([
      ["a.rs", false],
      ["c.rs", true],
    ]);
    // 이미 연 파일을 고정으로 열면 그 탭을 고정한다
    state = run(state, open("c.rs"));
    expect(state.tabs.map((t) => [tabFile(t), t.preview])).toEqual([
      ["a.rs", false],
      ["c.rs", false],
    ]);
    // 지운 파일은 HEAD 버전
    state = run(state, open("gone.rs", false, true));
    expect(topOf(activeTab(state)).target).toEqual({ repo: "/repo", path: "gone.rs", rev: "HEAD" });
  });

  it("같은 화면을 줄과 함께 열면 그 줄로 다시 연다", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"));
    const before = topOf(activeTab(state));
    state = run(state, nav({ kind: "file", target: { repo: "/repo", path: "a.rs", rev: null }, line: 12 }, inTab(state)));
    expect(state.tabs).toHaveLength(1);
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
    expect([state.tabs.length, state.focus]).toEqual([0, "panel"]);
  });

  it("닫으면 그 앞에 본 탭으로, 마지막 탭을 닫으면 패널이나 빈 화면", () => {
    let state = run(emptyWorkspace("/repo"), open("a.rs"), open("b.rs"), open("c.rs"));
    const [a, b, c] = state.tabs.map((t) => t.key);
    expect(state.tabs.map(tabFile)).toEqual(["a.rs", "b.rs", "c.rs"]);
    state = run(state, { type: "activate", tab: a }, { type: "activate", tab: c });
    state = run(state, { type: "close", tab: c });
    expect(state.active).toBe(a);
    state = run(state, { type: "cycle", delta: 1 });
    expect(state.active).toBe(b);
    state = run(state, { type: "cycle", delta: 1 });
    expect(state.active).toBe(a);
    state = run(state, { type: "cycle", delta: -1 });
    expect(state.active).toBe(b);
    state = run(state, { type: "togglePanel" }, { type: "focus", focus: "editor" });
    state = run(state, { type: "close", tab: b }, { type: "close", tab: a });
    expect([state.active, state.focus]).toEqual([null, "panel"]);
    state = run(state, { type: "togglePanel" });
    expect(state.focus).toBe("editor");
    expect(windowTitle(state)).toBe("repo");
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
    expect(state.tabs).toHaveLength(1);
    expect(topOf(activeTab(state))).toMatchObject({ target: { repo: "/w/에이전트", path: "a.rs" }, line: 5 });
    state = run(state, nav({ kind: "graph", repo: "/repo", rev: "c1", worktree: true }, PANEL));
    expect(state.repo).toBe("/repo");
    expect([state.tabs.length, state.panel.length, state.focus]).toEqual([0, 1, "panel"]);
  });
});
