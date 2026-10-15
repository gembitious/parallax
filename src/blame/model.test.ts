import { describe, expect, it } from "vitest";
import { BlameModel } from "./model";
import type { BlameBlock, BlameCommit } from "./types";

const commit = (sha: string, summary = sha): BlameCommit => ({
  sha,
  author: "김개발",
  authorMail: "<kim@example.com>",
  authorTime: 1_700_000_000,
  authorTz: "+0900",
  committer: "김개발",
  committerMail: "<kim@example.com>",
  committerTime: 1_700_000_000,
  committerTz: "+0900",
  summary,
  boundary: false,
});

const block = (sha: string, finalLine: number, numLines: number): BlameBlock => ({
  sha,
  origLine: finalLine,
  finalLine,
  numLines,
  path: "a.rs",
  previous: null,
});

describe("BlameModel", () => {
  it("줄 순서와 무관하게 온 블록을 줄에 펼친다", () => {
    const model = new BlameModel(6);
    model.add({ commits: [commit("b")], blocks: [block("b", 4, 3)] });
    expect(model.blamedLines).toBe(3);
    expect(model.complete).toBe(false);
    expect(model.sha(3)).toBeUndefined();
    expect(model.sha(4)).toBe("b");

    model.add({ commits: [commit("a")], blocks: [block("a", 1, 2), block("b", 3, 1)] });
    expect(model.blamedLines).toBe(6);
    expect(model.complete).toBe(true);
    expect([1, 2, 3, 4, 5, 6].map((l) => model.sha(l))).toEqual(["a", "a", "b", "b", "b", "b"]);
    expect(model.commit(5)?.summary).toBe("b");
    expect(model.version).toBe(2);
  });

  it("같은 커밋이 이어지면 첫 줄만 묶음의 시작이다(블록이 달라도)", () => {
    const model = new BlameModel(5);
    model.add({ commits: [commit("a"), commit("b")], blocks: [block("a", 1, 2), block("a", 3, 1), block("b", 4, 2)] });
    expect([1, 2, 3, 4, 5].map((l) => model.isRunStart(l))).toEqual([true, false, false, true, false]);
  });

  it("위 줄을 아직 모르면 시작으로 보고, 채워지면 다시 계산한다", () => {
    const model = new BlameModel(3);
    model.add({ commits: [commit("a")], blocks: [block("a", 2, 2)] });
    expect(model.isRunStart(1)).toBe(false);
    expect(model.isRunStart(2)).toBe(true);
    model.add({ commits: [], blocks: [block("a", 1, 1)] });
    expect(model.isRunStart(1)).toBe(true);
    expect(model.isRunStart(2)).toBe(false);
  });

  it("범위 밖 줄과 넘치는 블록", () => {
    const model = new BlameModel(2);
    model.add({ commits: [commit("a")], blocks: [block("a", 2, 5)] });
    expect(model.blamedLines).toBe(1);
    expect(model.sha(0)).toBeUndefined();
    expect(model.sha(3)).toBeUndefined();
    expect(model.block(Number.NaN)).toBeUndefined();
  });

  it("빈 파일", () => {
    const model = new BlameModel(0);
    expect(model.complete).toBe(true);
    expect(model.sha(1)).toBeUndefined();
  });
});
