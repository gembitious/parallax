import { describe, expect, it } from "vitest";
import { authorQuery, codeQuery, filterKind, filterLabel, nextRow, parseQuery, rowMatches, sameFilter } from "./search";
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

  it("adds author, message and date filters (M18)", () => {
    expect(parseQuery("author:김철수 <kim@example.com>")).toEqual({
      kind: "log",
      filter: { path: null, pickaxe: null, author: "김철수 <kim@example.com>" },
    });
    expect(parseQuery("작성자: kim path:src/")).toEqual({
      kind: "log",
      filter: { path: "src/", pickaxe: null, author: "kim" },
    });
    expect(parseQuery("since:2 weeks ago until: 2026-11-01")).toEqual({
      kind: "log",
      filter: { path: null, pickaxe: null, since: "2 weeks ago", until: "2026-11-01" },
    });
    // 거르기 앞의 글은 메시지에서 찾는다
    expect(parseQuery("고침 이후:2026-01-01")).toEqual({
      kind: "log",
      filter: { path: null, pickaxe: null, message: "고침", since: "2026-01-01" },
    });
    // 코드 찾기 뒤에 어느 차례로든 이어 붙인다(값은 다음 거르기까지)
    expect(parseQuery("code:fn main() author:Lee Kim path:src")).toEqual({
      kind: "log",
      filter: { path: "src", pickaxe: { text: "fn main()", regex: false }, author: "Lee Kim" },
    });
    expect(parseQuery("re:a|b BEFORE:2020-01-01")).toEqual({
      kind: "log",
      filter: { path: null, pickaxe: { text: "a|b", regex: true }, until: "2020-01-01" },
    });
    // 값이 빈 거르기만 있으면 찾을 것이 없다. 빈 거르기는 버린다.
    expect(parseQuery("author:   ")).toBeNull();
    expect(parseQuery("since: author:kim")).toEqual({ kind: "log", filter: { path: null, pickaxe: null, author: "kim" } });
    expect(parseQuery("fix author:")).toEqual({ kind: "log", filter: { path: null, pickaxe: null, message: "fix" } });
    // 거르기 이름이 아닌 `글:`은 그대로 메시지·작성자 검색
    expect(parseQuery("fix: crash")).toEqual({ kind: "text", text: "fix: crash" });
    expect(authorQuery("Kim", "k@x", null)).toBe("author:Kim <k@x>");
    expect(parseQuery(authorQuery("Kim Lee", "k@x", "src/a b.ts"))).toEqual({
      kind: "log",
      filter: { path: "src/a b.ts", pickaxe: null, author: "Kim Lee <k@x>" },
    });
    expect(filterLabel({ path: "src", pickaxe: null, author: "kim", since: "2026-01-01", message: "fix" })).toBe(
      "메시지 fix · 작성자 kim · 2026-01-01 이후 · 경로 src",
    );
    expect(filterKind({ path: "src", pickaxe: null, author: "kim" })).toBe("작성자");
    expect(sameFilter({ path: null, pickaxe: null, author: "a" }, { path: null, pickaxe: null, author: "b" })).toBe(false);
    expect(sameFilter({ path: null, pickaxe: null, author: "a" }, { path: null, pickaxe: null, author: "a" })).toBe(true);
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
