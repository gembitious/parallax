import { describe, expect, it } from "vitest";
import { insertIndex, zoneAt } from "./drag";

describe("zoneAt", () => {
  it("왼쪽·오른쪽 끝은 그쪽에 나누기, 가운데는 그 그룹", () => {
    expect(zoneAt(0.1, true)).toBe("left");
    expect(zoneAt(0.5, true)).toBe("center");
    expect(zoneAt(0.9, true)).toBe("right");
    expect(zoneAt(0.25, true)).toBe("center");
    expect(zoneAt(0.75, true)).toBe("center");
  });

  it("더 나눌 수 없으면 어디든 그 그룹", () => {
    expect(zoneAt(0.1, false)).toBe("center");
    expect(zoneAt(0.9, false)).toBe("center");
  });
});

describe("insertIndex", () => {
  it("가운데보다 왼쪽인 첫 탭의 앞, 없으면 끝", () => {
    const centers = [50, 150, 250];
    expect(insertIndex(centers, 10)).toBe(0);
    expect(insertIndex(centers, 100)).toBe(1);
    expect(insertIndex(centers, 249)).toBe(2);
    expect(insertIndex(centers, 300)).toBe(3);
    expect(insertIndex([], 10)).toBe(0);
  });
});
