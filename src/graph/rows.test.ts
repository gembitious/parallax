import { describe, expect, it } from "vitest";
import { CHUNK, MAX_CHUNKS, RowCache, rowWidth } from "./rows";
import type { GraphRow, GraphWindow } from "./types";

const row = (i: number, extra: Partial<GraphRow> = {}): GraphRow => ({
  sha: String(i).padStart(40, "0"),
  parents: [],
  summary: `커밋 ${i}`,
  author: "a",
  authorMail: "a@example.com",
  authorTime: 0,
  authorTzOffset: 0,
  labels: [],
  lane: 0,
  color: 0,
  through: [],
  up: [],
  down: [],
  ...extra,
});

const window = (start: number, count: number): GraphWindow => ({
  start,
  rows: Array.from({ length: count }, (_, k) => row(start + k)),
  total: 10_000,
  available: 10_000,
  width: 1,
  done: true,
});

describe("RowCache", () => {
  it("asks for missing chunks once and serves rows", () => {
    const cache = new RowCache();
    cache.seed(window(0, 100).rows);
    expect(cache.get(5)?.summary).toBe("커밋 5");
    // 0번 묶음은 100행만 있다(레이아웃이 다 정해지면 200행을 다시 읽는다).
    expect(cache.take(0, 150, 100)).toEqual([]);
    expect(cache.take(0, 150, 1000)).toEqual([0]);
    expect(cache.take(0, 150, 1000), "읽는 중이면 다시 묻지 않는다").toEqual([]);
    cache.put(window(0, CHUNK));
    expect(cache.get(150)?.summary).toBe("커밋 150");
    expect(cache.take(350, 450, 1000)).toEqual([1, 2]);
    expect(cache.get(450)).toBeUndefined();
  });

  it("does not ask beyond the laid out rows", () => {
    const cache = new RowCache();
    expect(cache.take(500, 600, 300)).toEqual([]);
    expect(cache.take(250, 600, 300)).toEqual([1]);
  });

  it("retries failed chunks and evicts the least recently seen", () => {
    const cache = new RowCache();
    expect(cache.take(0, 10, 100)).toEqual([0]);
    cache.failed(0);
    expect(cache.take(0, 10, 100)).toEqual([0]);
    for (let index = 0; index <= MAX_CHUNKS; index++) cache.put(window(index * CHUNK, CHUNK));
    expect(cache.get(0)).toBeUndefined();
    expect(cache.get(CHUNK)?.summary).toBe(`커밋 ${CHUNK}`);
  });
});

describe("rowWidth", () => {
  it("counts every lane a row touches", () => {
    expect(rowWidth(row(0))).toBe(1);
    expect(rowWidth(row(0, { lane: 1, through: [[3, 2, 0]], down: [[5, 1]] }))).toBe(6);
  });
});
