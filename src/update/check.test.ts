import { describe, expect, it } from "vitest";
import { isNewer, parseVersion } from "./check";

describe("버전 견주기", () => {
  it("v를 떼고 세 수를 읽는다", () => {
    expect(parseVersion("v0.1.0")).toEqual([0, 1, 0]);
    expect(parseVersion("10.20.3")).toEqual([10, 20, 3]);
    for (const bad of ["", "v1.2", "1.2.3.4", "v1.2.3-rc1", "x1.2.3", "v1..3"]) expect(parseVersion(bad)).toBeNull();
  });

  it("수로 견준다(글자 순이 아니다)", () => {
    expect(isNewer("v0.2.0", "0.1.0")).toBe(true);
    expect(isNewer("v0.10.0", "0.9.9")).toBe(true);
    expect(isNewer("v1.0.0", "0.99.99")).toBe(true);
    expect(isNewer("v0.1.0", "0.1.0")).toBe(false);
    expect(isNewer("v0.1.0", "0.1.1")).toBe(false);
    expect(isNewer("v0.2.0-rc1", "0.1.0")).toBe(false);
    expect(isNewer("garbage", "0.1.0")).toBe(false);
  });
});
