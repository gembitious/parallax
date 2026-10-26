import { describe, expect, it } from "vitest";
import { appendFiles, buildRows, findRow, nextFileRow, nextLineRow, splitPaths, splitRanges } from "./rows";
import type { GrepFile } from "./types";

const line = (n: number, text = "x") => ({ line: n, text, ranges: [] as [number, number][], cutStart: false, cutEnd: false });
const file = (path: string, ...lines: number[]): GrepFile => ({ path, lines: lines.map((n) => line(n)) });

describe("appendFiles", () => {
  it("continues the last file when a batch starts with it", () => {
    const merged = appendFiles([file("a", 1), file("b", 2)], [file("b", 5), file("c", 1)]);
    expect(merged.map((f) => [f.path, f.lines.map((l) => l.line)])).toEqual([
      ["a", [1]],
      ["b", [2, 5]],
      ["c", [1]],
    ]);
  });
});

describe("rows", () => {
  const files = [file("a", 1, 2), file("b", 7)];
  it("puts a header before each file and hides collapsed lines", () => {
    expect(buildRows(files, new Set()).map((r) => r.kind)).toEqual(["file", "line", "line", "file", "line"]);
    expect(buildRows(files, new Set(["a"])).map((r) => r.kind)).toEqual(["file", "file", "line"]);
  });
  it("moves between lines and files", () => {
    const rows = buildRows(files, new Set());
    expect(nextLineRow(rows, 0, 1)).toBe(1);
    expect(nextLineRow(rows, 2, 1)).toBe(4);
    expect(nextLineRow(rows, 1, -1)).toBeNull();
    expect(nextFileRow(rows, 1, 1)).toBe(3);
  });
  it("finds the same match again after rebuilding", () => {
    const rows = buildRows(files, new Set());
    expect(findRow(rows, files, "b", 7)).toBe(4);
    expect(findRow(rows, files, "a", 99)).toBe(0);
    expect(findRow(buildRows(files, new Set(["b"])), files, "b", 7)).toBe(3);
    expect(findRow(rows, files, null, null)).toBe(0);
  });
});

describe("text helpers", () => {
  it("splits path specs and highlighted text", () => {
    expect(splitPaths(" src/, *.ts  !*.lock ")).toEqual(["src/", "*.ts", "!*.lock"]);
    expect(splitRanges("let value = v", [[4, 9], [12, 13]])).toEqual([
      { text: "let ", hit: false },
      { text: "value", hit: true },
      { text: " = ", hit: false },
      { text: "v", hit: true },
    ]);
  });
});
