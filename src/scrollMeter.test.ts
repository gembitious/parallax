import { describe, expect, it } from "vitest";
import { FRAME_BUDGET_MS, summarize } from "./scrollMeter";

describe("summarize", () => {
  it("그리기와 프레임 간격의 분포, 빠진 프레임", () => {
    const work = [2, 3, 4, 5, 30];
    const intervals = [16.7, 16.6, 16.7, 33.4, 16.7];
    const s = summarize(work, intervals, 1);
    expect(s.frames).toBe(5);
    expect(s.workP50).toBe(4);
    expect(s.workMax).toBe(30);
    expect(s.intervalP50).toBe(16.7);
    expect(s.intervalP95).toBe(33.4);
    expect(s.late).toBe(1);
    expect(s.blank).toBe(1);
  });

  it("한 프레임 반까지는 빠진 프레임이 아니다", () => {
    expect(summarize([1], [FRAME_BUDGET_MS * 1.5], 0).late).toBe(0);
    expect(summarize([1], [FRAME_BUDGET_MS * 1.5 + 0.1], 0).late).toBe(1);
  });

  it("입력 배열을 바꾸지 않는다", () => {
    const work = [5, 1, 3];
    summarize(work, [], 0);
    expect(work).toEqual([5, 1, 3]);
  });
});
