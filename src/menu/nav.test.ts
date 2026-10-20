import { describe, expect, it } from "vitest";
import { MENU_MARGIN, edgeIndex, placeMenu, stepIndex } from "./nav";

const viewport = { width: 800, height: 600 };
const size = { width: 200, height: 150 };

describe("placeMenu", () => {
  it("우클릭은 마우스 자리 오른쪽 아래", () => {
    expect(placeMenu({ x: 100, y: 120 }, size, viewport)).toEqual({ left: 100, top: 120 });
  });

  it("오른쪽·아래가 모자라면 마우스 자리 왼쪽·위로 접는다", () => {
    expect(placeMenu({ x: 700, y: 550 }, size, viewport)).toEqual({ left: 500, top: 400 });
  });

  it("접어도 넘치면 창 안으로 민다", () => {
    expect(placeMenu({ x: 150, y: 100 }, { width: 200, height: 590 }, viewport)).toEqual({ left: 150, top: MENU_MARGIN });
    expect(placeMenu({ x: 100, y: 20 }, { width: 900, height: 100 }, viewport).left).toBe(MENU_MARGIN);
  });

  it("메뉴 막대는 제목 단추 아래, 모자라면 위", () => {
    const title = { left: 40, top: 0, right: 80, bottom: 30 };
    expect(placeMenu(title, size, viewport)).toEqual({ left: 40, top: 30 });
    const low = { left: 40, top: 500, right: 80, bottom: 530 };
    expect(placeMenu(low, size, viewport)).toEqual({ left: 40, top: 350 });
    // getBoundingClientRect()의 DOMRect는 x·y도 가진다: 마우스 자리로 읽으면 메뉴가 제목을 덮는다.
    const rect = { x: 40, y: 0, width: 40, height: 30, ...title };
    expect(placeMenu(rect, size, viewport)).toEqual({ left: 40, top: 30 });
  });

  it("제목이 오른쪽 끝이면 창 안으로 민다", () => {
    const title = { left: 750, top: 0, right: 790, bottom: 30 };
    expect(placeMenu(title, size, viewport)).toEqual({ left: 800 - MENU_MARGIN - 200, top: 30 });
  });
});

describe("stepIndex", () => {
  // 구분선·머리말·꺼진 항목은 고르지 못한다.
  const selectable = [false, true, false, true, true, false];

  it("고를 수 있는 다음 칸으로, 끝에서는 처음으로", () => {
    expect(stepIndex(selectable, 1, 1)).toBe(3);
    expect(stepIndex(selectable, 4, 1)).toBe(1);
    expect(stepIndex(selectable, 1, -1)).toBe(4);
  });

  it("아무것도 고르지 않았으면 아래로는 첫 칸, 위로는 마지막 칸", () => {
    expect(stepIndex(selectable, -1, 1)).toBe(1);
    expect(stepIndex(selectable, -1, -1)).toBe(4);
    expect(edgeIndex(selectable, false)).toBe(1);
    expect(edgeIndex(selectable, true)).toBe(4);
  });

  it("고를 칸이 없으면 -1", () => {
    expect(stepIndex([false, false], 0, 1)).toBe(-1);
    expect(stepIndex([], -1, 1)).toBe(-1);
    expect(edgeIndex([false], true)).toBe(-1);
  });
});
