import { describe, expect, it } from "vitest";
import { fitScale, nudge, resized } from "./image";

describe("fitScale", () => {
  it("shrinks the larger image to fit and never enlarges", () => {
    expect(fitScale({ width: 400, height: 300 }, [{ width: 800, height: 100 }, { width: 200, height: 600 }])).toBe(0.5);
    expect(fitScale({ width: 400, height: 300 }, [{ width: 16, height: 16 }, null])).toBe(1);
  });
  it("is 1 when nothing is known", () => {
    expect(fitScale({ width: 0, height: 0 }, [{ width: 800, height: 600 }])).toBe(1);
    expect(fitScale({ width: 400, height: 300 }, [null, { width: 0, height: 0 }])).toBe(1);
  });
});

describe("resized", () => {
  it("says only a real change", () => {
    expect(resized({ width: 16, height: 16 }, { width: 32, height: 32 })).toBe("16×16 → 32×32");
    expect(resized({ width: 16, height: 16 }, { width: 16, height: 16 })).toBeNull();
    expect(resized(null, { width: 16, height: 16 })).toBeNull();
  });
});

it("nudge stays within the frame", () => {
  expect(nudge(95, 10)).toBe(100);
  expect(nudge(5, -10)).toBe(0);
  expect(nudge(50, 10)).toBe(60);
});
