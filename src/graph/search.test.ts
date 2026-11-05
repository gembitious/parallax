import { describe, expect, it } from "vitest";
import { codeQuery, filterLabel, nextRow, parseQuery, rowMatches, sameFilter } from "./search";
import type { GraphRow } from "./types";

describe("parseQuery", () => {
  it("separates path and code queries", () => {
    const log = (path: string | null, pickaxe: { text: string; regex: boolean } | null = null) => ({
      kind: "log",
      filter: { path, pickaxe },
    });
    expect(parseQuery("  고침 ")).toEqual({ kind: "text", text: "고침" });
    expect(parseQuery("path: src/lib.rs")).toEqual(log("src/lib.rs"));
    expect(parseQuery("경로:./한글/파일 이름.txt")).toEqual(log("한글/파일 이름.txt"));
    expect(parseQuery("PATH:docs")).toEqual(log("docs"));
    expect(parseQuery("path:  ")).toBeNull();
    expect(parseQuery("   ")).toBeNull();
    expect(parseQuery("code: fn main() ")).toEqual(log(null, { text: "fn main()", regex: false }));
    expect(parseQuery("코드:한글 내용 path: ./문서")).toEqual(log("문서", { text: "한글 내용", regex: false }));
    expect(parseQuery("re:^use .*;$")).toEqual(log(null, { text: "^use .*;$", regex: true }));
    expect(parseQuery("정규식:a|b 경로:src")).toEqual(log("src", { text: "a|b", regex: true }));
    // 코드 없이 경로만 붙이면 그 글 자체를 찾는다.
    expect(parseQuery("code: path:src")).toEqual(log(null, { text: "path:src", regex: false }));
    expect(parseQuery("code:   ")).toBeNull();
    expect(parseQuery("code:x path:  ")).toEqual(log(null, { text: "x", regex: false }));
  });

  it("names and compares filters", () => {
    expect(filterLabel({ path: "src", pickaxe: { text: "foo", regex: false } })).toBe("코드 foo · 경로 src");
    expect(filterLabel({ path: "src", pickaxe: null })).toBe("경로 src");
    const a = { path: null, pickaxe: { text: "x", regex: true } };
    expect(sameFilter(a, { path: null, pickaxe: { text: "x", regex: true } })).toBe(true);
    expect(sameFilter(a, { path: null, pickaxe: { text: "x", regex: false } })).toBe(false);
    expect(sameFilter(a, { path: null, pickaxe: null })).toBe(false);
  });

  it("turns a selection into a code query", () => {
    expect(codeQuery("\n  let total = 0;\n  next\n")).toBe("code:let total = 0;");
    expect(codeQuery(" \n ")).toBeNull();
    expect(codeQuery("x".repeat(300))).toBe(`code:${"x".repeat(200)}`);
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
