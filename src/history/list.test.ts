import { describe, expect, it } from "vitest";
import { findMatch, formerPaths, matches, scrollToRow, visibleRange } from "./list";
import type { LogEntry } from "./types";

const entry = (sha: string, summary: string, extra: Partial<LogEntry> = {}): LogEntry => ({
  sha,
  parents: [],
  author: "Kim",
  authorMail: "kim@example.com",
  authorTime: 0,
  authorTz: "+0900",
  summary,
  status: "modified",
  path: "src/a.rs",
  oldPath: null,
  ...extra,
});

describe("가상 스크롤", () => {
  it("보이는 범위에 여유 행을 더하고 끝을 넘지 않는다", () => {
    expect(visibleRange(0, 240, 24, 1000, 5)).toEqual({ start: 0, end: 15 });
    expect(visibleRange(2400, 240, 24, 1000, 5)).toEqual({ start: 95, end: 115 });
    expect(visibleRange(2400, 240, 24, 100, 5)).toEqual({ start: 95, end: 100 });
    expect(visibleRange(-50, 0, 24, 0)).toEqual({ start: 0, end: 0 });
  });

  it("선택한 행이 보이게 필요한 만큼만 스크롤한다", () => {
    expect(scrollToRow(0, 100, 240, 24)).toBe(0);
    expect(scrollToRow(5, 0, 240, 24)).toBe(0);
    expect(scrollToRow(10, 0, 240, 24)).toBe(24);
    expect(scrollToRow(3, 200, 240, 24)).toBe(72);
  });
});

describe("검색", () => {
  const entries = [
    entry("aaa111", "Fix 한글 경로"),
    entry("bbb222", "Add feature", { author: "Lee" }),
    entry("ccc333", "Refactor", { path: "src/old.rs" }),
  ];

  it("요약·작성자·sha 앞부분·경로, 대소문자 무시", () => {
    expect(matches(entries[0], "한글")).toBe(true);
    expect(matches(entries[0], "FIX")).toBe(true);
    expect(matches(entries[1], "lee")).toBe(true);
    expect(matches(entries[1], "bbb")).toBe(true);
    expect(matches(entries[1], "222")).toBe(false);
    expect(matches(entries[2], "old.rs")).toBe(true);
    expect(matches(entries[2], "   ")).toBe(false);
  });

  it("다음·이전으로 돌아가며 찾는다", () => {
    expect(findMatch(entries, 0, "a", 1)).toBe(1); // "Add feature"
    expect(findMatch(entries, 1, "fix", 1)).toBe(0);
    expect(findMatch(entries, 0, "refactor", -1)).toBe(2);
    expect(findMatch(entries, 0, "없는 말", 1)).toBe(-1);
    expect(findMatch([], 0, "a", 1)).toBe(-1);
  });
});

it("옛 경로는 이름이 바뀐 순서대로 한 번씩", () => {
  const entries = [
    entry("1", "edit", { path: "c.rs" }),
    entry("2", "rename", { path: "c.rs", status: "renamed", oldPath: "b.rs" }),
    entry("3", "edit", { path: "b.rs" }),
    entry("4", "rename", { path: "b.rs", status: "renamed", oldPath: "a.rs" }),
    entry("5", "add", { path: "a.rs", status: "added" }),
  ];
  expect(formerPaths(entries, "c.rs")).toEqual(["b.rs", "a.rs"]);
  expect(formerPaths([], "c.rs")).toEqual([]);
});
