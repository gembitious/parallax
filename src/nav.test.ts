import { describe, expect, it } from "vitest";
import { MAX_STACK, backDirectionOf, titleOf, viewReducer, type PlxRequest, type ViewState } from "./nav";

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
    expect(state.stack[0]).toMatchObject({ target: { repo: null, path: "/repo/a.rs", rev: null }, line: 3, via: "request" });

    state = navigate(state, "parent", "p1");
    expect(state.stack).toHaveLength(2);
    // 이벤트와 current_request가 같은 요청을 두 번 줄 수 있다
    expect(viewReducer(state, { type: "request", request: blame(2), receivedMs: 2 })).toBe(state);
    expect(viewReducer(state, { type: "request", request: blame(1), receivedMs: 2 })).toBe(state);

    const next = viewReducer(state, { type: "request", request: blame(3, "/repo/b.rs"), receivedMs: 3 });
    expect(next.stack.map((e) => e.target.path)).toEqual(["/repo/b.rs"]);
    expect(next.stack[0].key).not.toBe(state.stack[0].key);
  });

  it("blame이 아닌 요청은 스택을 비운다", () => {
    const state = viewReducer(empty, { type: "request", request: request(1, { kind: "graph" }), receivedMs: 0 });
    expect(state.stack).toEqual([]);
    expect(state.shown?.request.id).toBe(1);
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
    expect(state.stack[MAX_STACK - 1].target.rev).toBe(`p${MAX_STACK + 4}`);
  });
});

describe("탐색 도우미", () => {
  it("반대 방향 키는 아래 화면으로 돌아간다", () => {
    expect(backDirectionOf("older")).toBe("newer");
    expect(backDirectionOf("newer")).toBe("older");
    expect(backDirectionOf("parent")).toBeUndefined();
    expect(backDirectionOf("request")).toBeUndefined();
  });

  it("창 제목", () => {
    const entry = { key: 1, target: { repo: "/r", path: "src/한글.rs", rev: "1b019344c559bac2" }, line: 1, via: "parent" as const };
    expect(titleOf(entry)).toBe("blame src/한글.rs @1b019344");
    expect(titleOf({ ...entry, target: { ...entry.target, rev: null } })).toBe("blame src/한글.rs");
  });
});
