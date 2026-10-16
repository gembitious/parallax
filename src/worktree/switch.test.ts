import { describe, expect, it } from "vitest";
import { blameSwitch, commitSwitch, historySwitch } from "./switch";
import type { Worktree } from "./WorktreePicker";

const wt = (current = false): Worktree => ({
  path: "/w/에이전트",
  name: "에이전트",
  id: "에이전트",
  main: false,
  current,
  branch: "agent/1",
  head: "b".repeat(40),
  summary: null,
  time: null,
  locked: false,
  missing: false,
});
const A = "a".repeat(40);

describe("blameSwitch", () => {
  it("작업트리 blame은 그 워크트리의 같은 경로·줄", () => {
    expect(blameSwitch(wt(), { path: "src/a.rs", rev: null }, 12)).toEqual({
      kind: "file",
      target: { repo: "/w/에이전트", path: "src/a.rs", rev: null },
      line: 12,
    });
  });

  it("리비전 blame은 같은 리비전 그대로(옛 이름·옛 줄을 작업트리 좌표로 쓰지 않는다)", () => {
    expect(blameSwitch(wt(), { path: "old/name.rs", rev: A }, 7)).toEqual({
      kind: "file",
      target: { repo: "/w/에이전트", path: "old/name.rs", rev: A },
      line: 7,
    });
  });

  it("아직 열지 못한 blame은 그 워크트리의 그래프, 지금 워크트리는 그대로", () => {
    expect(blameSwitch(wt(), null, null)).toEqual({ kind: "graph", repo: "/w/에이전트", rev: null });
    expect(blameSwitch(wt(true), { path: "a", rev: null }, 1)).toBeNull();
  });
});

describe("historySwitch", () => {
  const meta = { path: "src/a.rs", rev: null, start: A, lines: { start: 10, end: 20 } };

  it("리비전에서 시작한 히스토리는 그대로", async () => {
    const pinned = { ...meta, rev: A };
    expect(await historySwitch(wt(), pinned, () => Promise.reject(new Error("부르지 않는다")))).toEqual({
      kind: "history",
      target: { repo: "/w/에이전트", path: "src/a.rs", rev: A },
      lines: { start: 10, end: 20 },
    });
  });

  it("HEAD에서 시작한 라인 히스토리는 줄 범위를 그 워크트리로 옮긴다", async () => {
    const calls: unknown[] = [];
    const nav = await historySwitch(wt(), meta, async (from, lines) => {
      calls.push([from, lines]);
      return { start: 12, end: 22 };
    });
    expect(calls).toEqual([[{ rev: A, path: "src/a.rs" }, { start: 10, end: 20 }]]);
    expect(nav).toEqual({
      kind: "history",
      target: { repo: "/w/에이전트", path: "src/a.rs", rev: null },
      lines: { start: 12, end: 22 },
    });
  });

  it("옮기지 못하면 파일 히스토리, 파일 히스토리는 그 워크트리의 HEAD에서", async () => {
    const plain = { kind: "history", target: { repo: "/w/에이전트", path: "src/a.rs", rev: null } };
    expect(await historySwitch(wt(), meta, () => Promise.reject(new Error("파일이 없다")))).toEqual(plain);
    expect(await historySwitch(wt(), meta, async () => null)).toEqual(plain);
    expect(await historySwitch(wt(), { ...meta, lines: null }, async () => null)).toEqual(plain);
    expect(await historySwitch(wt(true), meta, async () => null)).toBeNull();
  });
});

describe("commitSwitch", () => {
  it("그 워크트리의 그래프에서 같은 커밋", () => {
    expect(commitSwitch(wt(), A)).toEqual({ kind: "graph", repo: "/w/에이전트", rev: A });
    expect(commitSwitch(wt(true), A)).toBeNull();
  });
});
