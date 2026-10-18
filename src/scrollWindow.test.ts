import { describe, expect, it } from "vitest";
import { drawWindow, insideDrawn, WINDOW_PAD_PX } from "./scrollWindow";

const ROW = 24;

describe("drawWindow", () => {
  it("화면과 그 위아래를 그린다", () => {
    // 화면 600px: 위아래 600px씩 → 1,800px = 75행
    expect(drawWindow(2400, 600, ROW, 10_000)).toEqual({ start: 75, end: 150 });
  });

  it("위아래 여유는 WINDOW_PAD_PX까지", () => {
    const { start, end } = drawWindow(48_000, 2000, ROW, 100_000);
    expect(start).toBe(Math.floor((48_000 - WINDOW_PAD_PX) / ROW));
    expect(end).toBe(Math.ceil((48_000 + 2000 + WINDOW_PAD_PX) / ROW));
  });

  it("목록 끝을 넘지 않는다", () => {
    expect(drawWindow(0, 600, ROW, 10)).toEqual({ start: 0, end: 10 });
    expect(drawWindow(0, 600, ROW, 0)).toEqual({ start: 0, end: 0 });
  });
});

describe("insideDrawn", () => {
  const drawn = { ...drawWindow(2400, 600, ROW, 10_000), height: 600 };

  it("그린 범위 안의 스크롤은 다시 그리지 않는다", () => {
    expect(insideDrawn(2400, 600, ROW, 10_000, drawn)).toBe(true);
    expect(insideDrawn(1800, 600, ROW, 10_000, drawn)).toBe(true);
    expect(insideDrawn(3000, 600, ROW, 10_000, drawn)).toBe(true);
  });

  it("벗어나거나 화면 높이가 바뀌면 다시 그린다", () => {
    expect(insideDrawn(1799, 600, ROW, 10_000, drawn)).toBe(false);
    expect(insideDrawn(3001, 600, ROW, 10_000, drawn)).toBe(false);
    expect(insideDrawn(2400, 700, ROW, 10_000, drawn)).toBe(false);
  });
});
