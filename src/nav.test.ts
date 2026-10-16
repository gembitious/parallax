import { describe, expect, it } from "vitest";
import {
  MAX_STACK,
  backDirectionAt,
  backDirectionOf,
  lastName,
  titleOf,
  viewReducer,
  type PlxRequest,
  type ViewState,
} from "./nav";

const request = (id: number, request: PlxRequest["request"]): PlxRequest => ({
  id,
  request,
  label: "",
  args: [],
  cwd: "/repo",
  t0Ms: 0,
  recvMs: 0,
  cold: false,
});

const blame = (id: number, path = "/repo/a.rs", line: number | null = 3) =>
  request(id, { kind: "blame", path, line });

const empty: ViewState = { shown: null, stack: [] };

const navigate = (state: ViewState, kind: "parent" | "older" | "newer", rev: string) =>
  viewReducer(state, {
    type: "navigate",
    navigation: { kind, target: { repo: "/repo", path: "a.rs", rev }, line: 1 },
  });

describe("viewReducer", () => {
  it("plx 요청은 탐색 스택을 새로 시작하고, 같거나 오래된 요청은 무시한다", () => {
    let state = viewReducer(empty, { type: "request", request: blame(2), receivedMs: 1 });
    expect(state.stack).toHaveLength(1);
    expect(state.stack[0]).toMatchObject({ view: "blame", target: { repo: null, path: "/repo/a.rs", rev: null }, line: 3, via: "request" });

    state = navigate(state, "parent", "p1");
    expect(state.stack).toHaveLength(2);
    // 이벤트와 current_request가 같은 요청을 두 번 줄 수 있다
    expect(viewReducer(state, { type: "request", request: blame(2), receivedMs: 2 })).toBe(state);
    expect(viewReducer(state, { type: "request", request: blame(1), receivedMs: 2 })).toBe(state);

    const next = viewReducer(state, { type: "request", request: blame(3, "/repo/b.rs"), receivedMs: 3 });
    expect(next.stack).toMatchObject([{ view: "blame", target: { path: "/repo/b.rs" } }]);
    expect(next.stack[0].key).not.toBe(state.stack[0].key);
  });

  it("인자 없는 plx는 cwd의 그래프로, plx show는 커밋 상세로 시작한다", () => {
    const state = viewReducer(empty, { type: "request", request: request(1, { kind: "graph" }), receivedMs: 0 });
    expect(state.stack).toMatchObject([{ view: "graph", repo: "/repo", focus: null, via: "request" }]);
    expect(state.shown?.request.id).toBe(1);
    const show = viewReducer(state, { type: "request", request: request(2, { kind: "show", rev: "HEAD~1" }), receivedMs: 0 });
    expect(show.stack).toMatchObject([{ view: "commit", repo: "/repo", rev: "HEAD~1", path: null, only: false, via: "request" }]);
    const log = viewReducer(show, { type: "request", request: request(3, { kind: "log", path: "/repo/a.rs" }), receivedMs: 0 });
    expect(log.stack).toMatchObject([
      { view: "history", target: { repo: null, path: "/repo/a.rs", rev: null }, lines: null, via: "request" },
    ]);
    const lineLog = viewReducer(log, {
      type: "request",
      request: request(4, { kind: "lineLog", path: "/repo/a.rs", start: 3, end: 9 }),
      receivedMs: 0,
    });
    expect(lineLog.stack).toMatchObject([{ view: "history", lines: { start: 3, end: 9 }, via: "request" }]);
    expect(titleOf(lineLog.stack[0])).toBe("라인 히스토리 /repo/a.rs:3-9");
  });

  it("blame → 히스토리 → 커밋 상세(그 파일만) → 그 리비전 blame", () => {
    let state = viewReducer(empty, { type: "request", request: blame(1), receivedMs: 0 });
    state = viewReducer(state, {
      type: "navigate",
      navigation: { kind: "history", target: { repo: "/repo", path: "a.rs", rev: "r1" } },
    });
    state = viewReducer(state, {
      type: "navigate",
      navigation: { kind: "commit", repo: "/repo", rev: "c1", path: "a.rs", only: true },
    });
    state = viewReducer(state, {
      type: "navigate",
      navigation: { kind: "file", target: { repo: "/repo", path: "a.rs", rev: "c1" }, line: null },
    });
    expect(state.stack.map((e) => [e.view, e.via])).toEqual([
      ["blame", "request"],
      ["history", "history"],
      ["commit", "commit"],
      ["blame", "file"],
    ]);
    expect(state.stack[2]).toMatchObject({ only: true, path: "a.rs" });
    expect(state.stack[1]).toMatchObject({ lines: null });
    expect(titleOf(state.stack[1])).toBe("히스토리 a.rs @r1");
    state = viewReducer(state, {
      type: "navigate",
      navigation: { kind: "history", target: { repo: "/repo", path: "a.rs", rev: null }, lines: { start: 2, end: 2 } },
    });
    expect(state.stack[4]).toMatchObject({ view: "history", lines: { start: 2, end: 2 }, via: "history" });
  });

  it("blame → 커밋 상세 → 파일 blame", () => {
    let state = viewReducer(empty, { type: "request", request: blame(1), receivedMs: 0 });
    state = viewReducer(state, {
      type: "navigate",
      navigation: { kind: "commit", repo: "/repo", rev: "abc", path: "a.rs" },
    });
    state = viewReducer(state, {
      type: "navigate",
      navigation: { kind: "file", target: { repo: "/repo", path: "a.rs", rev: "abc" }, line: 7 },
    });
    expect(state.stack.map((e) => [e.view, e.via])).toEqual([
      ["blame", "request"],
      ["commit", "commit"],
      ["blame", "file"],
    ]);
    expect(titleOf(state.stack[1])).toBe("커밋 abc");
  });

  it("뒤로는 맨 아래 화면에서 멈춘다", () => {
    let state = viewReducer(empty, { type: "request", request: blame(1), receivedMs: 0 });
    state = navigate(state, "parent", "p1");
    state = navigate(state, "older", "o1");
    expect(state.stack.map((e) => e.via)).toEqual(["request", "parent", "older"]);
    state = viewReducer(state, { type: "back" });
    state = viewReducer(state, { type: "back" });
    expect(state.stack.map((e) => e.via)).toEqual(["request"]);
    expect(viewReducer(state, { type: "back" })).toBe(state);
  });

  it("너무 깊어지면 가장 아래 화면부터 버린다", () => {
    let state = viewReducer(empty, { type: "request", request: blame(1), receivedMs: 0 });
    for (let i = 0; i < MAX_STACK + 5; i++) state = navigate(state, "parent", `p${i}`);
    expect(state.stack).toHaveLength(MAX_STACK);
    expect(state.stack[MAX_STACK - 1]).toMatchObject({ target: { rev: `p${MAX_STACK + 4}` } });
  });

  it("아래를 버려 맨 아래가 된 `<` 화면의 `>`는 돌아가지 않고 새로 연다", () => {
    let state = viewReducer(empty, { type: "request", request: blame(1), receivedMs: 0 });
    for (let i = 0; i < MAX_STACK; i++) state = navigate(state, "older", `o${i}`);
    expect(state.stack).toHaveLength(MAX_STACK);
    expect(state.stack.map((e) => e.via)).toEqual(Array(MAX_STACK).fill("older"));
    expect(backDirectionAt(state.stack, 0)).toBeUndefined();
    expect(backDirectionAt(state.stack, 1)).toBe("newer");
    while (state.stack.length > 1) state = viewReducer(state, { type: "back" });
    expect(backDirectionAt(state.stack, 0)).toBeUndefined();
  });
});

describe("탐색 도우미", () => {
  it("반대 방향 키는 아래 화면으로 돌아간다", () => {
    expect(backDirectionOf("older")).toBe("newer");
    expect(backDirectionOf("newer")).toBe("older");
    expect(backDirectionOf("parent")).toBeUndefined();
    expect(backDirectionOf("request")).toBeUndefined();
    let state = viewReducer(empty, { type: "request", request: blame(1), receivedMs: 0 });
    state = navigate(state, "older", "o1");
    state = navigate(state, "newer", "n1");
    expect([0, 1, 2].map((i) => backDirectionAt(state.stack, i))).toEqual([undefined, "newer", "older"]);
  });

  it("창 제목", () => {
    const entry = {
      key: 1,
      view: "blame" as const,
      target: { repo: "/r", path: "src/한글.rs", rev: "1b019344c559bac2" },
      line: 1,
      via: "parent" as const,
    };
    expect(titleOf(entry)).toBe("blame src/한글.rs @1b019344");
    expect(titleOf({ ...entry, target: { ...entry.target, rev: null } })).toBe("blame src/한글.rs");
  });

  it("G는 어느 화면에서든 그 커밋을 고른 그래프로 간다", () => {
    let state = viewReducer(empty, { type: "request", request: blame(1), receivedMs: 0 });
    state = viewReducer(state, { type: "navigate", navigation: { kind: "graph", repo: "/repo/sub", rev: "c1" } });
    expect(state.stack[1]).toMatchObject({ view: "graph", repo: "/repo/sub", focus: "c1", via: "graph" });
    expect(titleOf(state.stack[1])).toBe("그래프 sub");
    state = viewReducer(state, { type: "navigate", navigation: { kind: "commit", repo: "/repo", rev: "c1", path: null } });
    expect(state.stack.map((e) => e.view)).toEqual(["blame", "graph", "commit"]);
    expect(viewReducer(state, { type: "back" }).stack.map((e) => e.view)).toEqual(["blame", "graph"]);
  });
});

describe("lastName", () => {
  it("takes the last path component on every platform", () => {
    expect(lastName("/home/me/한글 저장소")).toBe("한글 저장소");
    expect(lastName("C:\\work\\repo\\")).toBe("repo");
    expect(lastName("repo")).toBe("repo");
    expect(lastName("/")).toBe("/");
  });
});

describe("reload", () => {
  it("replaces the entry in place with a new key (blame keeps the given line)", () => {
    let state = viewReducer(empty, { type: "request", request: blame(1), receivedMs: 0 });
    state = viewReducer(state, {
      type: "navigate",
      navigation: { kind: "history", target: { repo: "/repo", path: "a.rs", rev: null } },
    });
    const [root, history] = state.stack;
    const reloaded = viewReducer(state, { type: "reload", key: root.key, line: 42 });
    expect(reloaded.stack[0]).toMatchObject({ view: "blame", line: 42, via: "request" });
    expect(reloaded.stack[0].key).not.toBe(root.key);
    expect(reloaded.stack[1]).toBe(history);
    const again = viewReducer(reloaded, { type: "reload", key: history.key });
    expect(again.stack[1]).toMatchObject({ view: "history", via: "history" });
    expect(again.stack[1].key).not.toBe(history.key);
    expect(viewReducer(again, { type: "reload", key: 9999 })).toBe(again);
  });

  it("라인 히스토리는 옮긴 줄 범위로 다시 연다(파일 히스토리는 범위가 생기지 않는다)", () => {
    let state = viewReducer(empty, { type: "request", request: blame(1), receivedMs: 0 });
    state = viewReducer(state, {
      type: "navigate",
      navigation: { kind: "history", target: { repo: "/repo", path: "a.rs", rev: null }, lines: { start: 40, end: 50 } },
    });
    const line = state.stack[1];
    const moved = viewReducer(state, { type: "reload", key: line.key, lines: { start: 50, end: 60 } });
    expect(moved.stack[1]).toMatchObject({ view: "history", lines: { start: 50, end: 60 } });
    state = viewReducer(moved, { type: "back" });
    state = viewReducer(state, {
      type: "navigate",
      navigation: { kind: "history", target: { repo: "/repo", path: "a.rs", rev: null } },
    });
    const file = state.stack[1];
    const same = viewReducer(state, { type: "reload", key: file.key, lines: { start: 1, end: 2 } });
    expect(same.stack[1]).toMatchObject({ view: "history", lines: null });
  });
});
