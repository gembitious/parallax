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

describe("BlameModel.aiAt", () => {
  it("블록의 원래 커밋·경로·줄로 AI 작성자를 찾는다", () => {
    const model = new BlameModel(6);
    // 지금 파일의 3~5줄은 커밋 a의 옛이름.rs 10~12줄에서 왔다.
    model.add({
      commits: [],
      blocks: [
        { sha: "a", origLine: 10, finalLine: 3, numLines: 3, path: "옛이름.rs", previous: null },
        { sha: "b", origLine: 1, finalLine: 1, numLines: 2, path: "새이름.rs", previous: null },
      ],
    });
    const claude = {
      kind: "agent" as const,
      tool: "claude",
      model: "opus",
      id: "1",
      human: null,
      messagesUrl: null,
      attributes: [],
      stats: null,
    };
    model.ai.merge({ notes: true, parsed: 0, authors: [claude], files: [{ sha: "a", path: "옛이름.rs", spans: [[11, 12, 0]] }] });
    expect(model.aiAt(3)).toBeNull(); // 원래 10줄
    expect(model.aiAt(4)).toEqual(claude); // 원래 11줄
    expect(model.aiAt(5)).toEqual(claude); // 원래 12줄
    expect(model.aiAt(1)).toBeNull(); // 다른 커밋
    expect(model.aiAt(6)).toBeNull(); // 아직 모르는 줄
  });
});
