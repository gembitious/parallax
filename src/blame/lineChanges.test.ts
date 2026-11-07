import { describe, expect, it } from "vitest";
import { lineMarks, markTitle, nextChange, type LineChange } from "./lineChanges";

const c = (kind: LineChange["kind"], start: number, end: number, removed = 0): LineChange => ({ kind, start, end, removed });

describe("lineMarks", () => {
  it("줄마다 추가·수정, 지운 자리는 앞 줄 아래", () => {
    const marks = lineMarks([c("added", 1, 1), c("modified", 3, 4, 1), c("deleted", 5, 5, 2)], 10);
    expect([...marks.keys()]).toEqual([1, 3, 4, 5]);
    expect(marks.get(1)).toMatchObject({ kind: "added", deleted: null });
    expect(marks.get(4)).toMatchObject({ kind: "modified" });
    expect(marks.get(5)).toMatchObject({ kind: null, deleted: "below", deletedLines: 2 });
  });

  it("맨 앞을 지웠으면 첫 줄 위, 바뀐 줄과 겹치면 함께", () => {
    const marks = lineMarks([c("deleted", 0, 0, 3), c("modified", 2, 2, 1), c("deleted", 2, 2, 1)], 3);
    expect(marks.get(1)).toMatchObject({ kind: null, deleted: "above", deletedLines: 3 });
    expect(marks.get(2)).toMatchObject({ kind: "modified", deleted: "below", deletedLines: 1 });
    expect(markTitle(marks.get(2)!)).toContain("1줄 → 1줄");
  });

  it("줄 수를 넘는 것은 버린다", () => {
    expect([...lineMarks([c("added", 2, 5)], 3).keys()]).toEqual([2, 3]);
    expect(lineMarks([c("deleted", 4, 4, 1)], 3).size).toBe(0);
  });
});

describe("nextChange", () => {
  const changes = [c("added", 3, 5), c("deleted", 8, 8, 1), c("modified", 12, 12, 1)];
  it("다음·이전 묶음의 첫 줄, 끝에서 돈다", () => {
    expect(nextChange(changes, 1, 1)).toBe(3);
    expect(nextChange(changes, 4, 1)).toBe(8);
    expect(nextChange(changes, 12, 1)).toBe(3);
    expect(nextChange(changes, 4, -1)).toBe(12);
    expect(nextChange(changes, 10, -1)).toBe(8);
    expect(nextChange([], 1, 1)).toBeNull();
  });
  it("맨 앞을 지운 자리는 1번 줄", () => {
    expect(nextChange([c("deleted", 0, 0, 2)], 5, 1)).toBe(1);
  });
});
