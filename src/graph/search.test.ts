import { describe, expect, it } from "vitest";
import { nextRow, parseQuery, rowMatches } from "./search";
import type { GraphRow } from "./types";

describe("parseQuery", () => {
  it("separates path queries", () => {
    expect(parseQuery("  고침 ")).toEqual({ kind: "text", text: "고침" });
    expect(parseQuery("path: src/lib.rs")).toEqual({ kind: "path", path: "src/lib.rs" });
    expect(parseQuery("경로:./한글/파일 이름.txt")).toEqual({ kind: "path", path: "한글/파일 이름.txt" });
    expect(parseQuery("PATH:docs")).toEqual({ kind: "path", path: "docs" });
    expect(parseQuery("path:  ")).toBeNull();
    expect(parseQuery("   ")).toBeNull();
  });
});

describe("rowMatches", () => {
  const row = {
    sha: "abcdef0123456789abcdef0123456789abcdef01",
    summary: "검색 결과 정렬",
    author: "Kim Dev",
    authorMail: "kim@example.com",
  } as GraphRow;
  it("looks at what the row shows", () => {
    expect(rowMatches(row, "결과")).toBe(true);
    expect(rowMatches(row, "kim dev")).toBe(true);
    expect(rowMatches(row, "EXAMPLE")).toBe(true);
    expect(rowMatches(row, "abcdef0")).toBe(true);
    expect(rowMatches(row, "0123")).toBe(false);
  });
});

describe("nextRow", () => {
  const rows = [3, 8, 20];
  it("moves forward and backward with wrap-around", () => {
    expect(nextRow(rows, 0, true)).toBe(3);
    expect(nextRow(rows, 3, true)).toBe(8);
    expect(nextRow(rows, 9, true)).toBe(20);
    expect(nextRow(rows, 20, true)).toBe(3);
    expect(nextRow(rows, 9, false)).toBe(8);
    expect(nextRow(rows, 8, false)).toBe(3);
    expect(nextRow(rows, 3, false)).toBe(20);
    expect(nextRow(rows, 100, false)).toBe(20);
    expect(nextRow([], 0, true)).toBeNull();
    expect(nextRow([5], 5, true)).toBe(5);
    expect(nextRow([5], 5, false)).toBe(5);
  });
});
