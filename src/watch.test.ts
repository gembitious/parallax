import { describe, expect, it } from "vitest";
import { headMoved } from "./watch";

describe("headMoved", () => {
  it("알림이 없으면 다시 열지 않는다", () => {
    expect(headMoved(undefined, "a", false)).toBe(false);
    expect(headMoved(undefined, undefined, true)).toBe(false);
  });

  it("화면이 이미 읽은 HEAD로 온 알림은 거른다(열기 바로 전의 커밋)", () => {
    expect(headMoved("b", "b", false)).toBe(false);
  });

  it("다른 커밋으로 움직였으면 다시 연다", () => {
    expect(headMoved("c", "b", false)).toBe(true);
    // 커밋이 없던 저장소에 첫 커밋
    expect(headMoved("a", null, false)).toBe(true);
  });

  it("기준을 아직 모르면 기다린다", () => {
    expect(headMoved("b", undefined, false)).toBe(false);
  });

  it("열기에 실패했으면 HEAD가 움직이면 다시 연다", () => {
    expect(headMoved("b", undefined, true)).toBe(true);
    expect(headMoved("b", "b", true)).toBe(true);
  });
});
