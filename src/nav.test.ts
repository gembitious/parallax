import { describe, expect, it } from "vitest";
import { backDirectionAt, backDirectionOf, lastName, titleOf, type BlameEntry, type StackEntry } from "./nav";

const blameAt = (key: number, via: BlameEntry["via"], rev: string | null = null): BlameEntry => ({
  key,
  view: "blame",
  target: { repo: "/repo", path: "a.rs", rev },
  line: 1,
  via,
});

describe("탐색 도우미", () => {
  it("반대 방향 키는 아래 화면으로 돌아간다", () => {
    expect(backDirectionOf("older")).toBe("newer");
    expect(backDirectionOf("newer")).toBe("older");
    expect(backDirectionOf("parent")).toBeUndefined();
    expect(backDirectionOf("request")).toBeUndefined();
    const stack = [blameAt(1, "request"), blameAt(2, "older", "o1"), blameAt(3, "newer", "n1")];
    expect([0, 1, 2].map((i) => backDirectionAt(stack, i))).toEqual([undefined, "newer", "older"]);
    // 아래를 버려 맨 아래가 된 `<` 화면에는 돌아갈 곳이 없다
    expect(backDirectionAt([blameAt(4, "older", "o2")], 0)).toBeUndefined();
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
    const others: StackEntry[] = [
      { key: 2, view: "graph", repo: "/repo/sub", focus: "c1", via: "graph" },
      { key: 3, view: "commit", repo: "/repo", rev: "abcdef0123456789", path: null, only: false, via: "commit" },
      { key: 4, view: "history", target: { repo: "/r", path: "a.rs", rev: "r1" }, lines: null, via: "history" },
      { key: 5, view: "history", target: { repo: "/r", path: "a.rs", rev: null }, lines: { start: 3, end: 9 }, via: "request" },
    ];
    expect(others.map(titleOf)).toEqual(["그래프 sub", "커밋 abcdef01", "히스토리 a.rs @r1", "라인 히스토리 a.rs:3-9"]);
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
