import { describe, expect, it } from "vitest";
import { AiOverlay, aiSummary, type AiAttribution, type AiAuthor } from "./ai";

const claude: AiAuthor = {
  kind: "agent",
  tool: "claude",
  model: "opus",
  id: "c1",
  human: "dev@example.com",
  messagesUrl: null,
  attributes: [],
  stats: null,
};
const cursor: AiAuthor = { ...claude, tool: "cursor", model: "gpt", id: "x" };
const human: AiAuthor = { kind: "human", author: "Dev <dev@example.com>" };

describe("AiOverlay", () => {
  it("구간에서 줄의 작성자를 찾는다", () => {
    const overlay = new AiOverlay();
    overlay.merge({
      notes: true,
      parsed: 0,
      authors: [claude, human],
      files: [{ sha: "a", path: "x.rs", spans: [[10, 12, 1], [1, 3, 0]] }],
    });
    expect(overlay.at("a", "x.rs", 2)).toEqual(claude);
    expect(overlay.at("a", "x.rs", 11)).toEqual(human);
    expect(overlay.at("a", "x.rs", 5)).toBeNull();
    expect(overlay.at("a", "y.rs", 2)).toBeNull();
    expect(overlay.at("b", "x.rs", 2)).toBeNull();
  });

  it("응답마다 다른 작성자 번호를 합친다", () => {
    const overlay = new AiOverlay();
    const first: AiAttribution = { notes: true, parsed: 0, authors: [claude], files: [{ sha: "a", path: "p", spans: [[1, 1, 0]] }] };
    const second: AiAttribution = {
      notes: true,
      parsed: 0,
      authors: [cursor, claude],
      files: [{ sha: "b", path: "p", spans: [[1, 1, 0], [2, 2, 1]] }],
    };
    overlay.merge(first);
    overlay.merge(second);
    expect(overlay.authors).toEqual([claude, cursor]);
    expect(overlay.at("b", "p", 1)).toEqual(cursor);
    expect(overlay.at("b", "p", 2)).toEqual(claude);
    expect(overlay.version).toBe(2);
  });

  it("같은 (커밋, 경로)는 한 번만 묻는다", () => {
    const overlay = new AiOverlay();
    expect(overlay.ask("a", "p")).toBe(true);
    expect(overlay.ask("a", "p")).toBe(false);
    expect(overlay.ask("a", "q")).toBe(true);
  });
});

describe("aiSummary", () => {
  it("에이전트 줄만 세고 도구별로 묶는다", () => {
    expect(aiSummary([claude, null, human, claude, cursor])).toEqual({
      lines: 3,
      tools: [
        ["claude (opus)", 2],
        ["cursor (gpt)", 1],
      ],
    });
  });
});
