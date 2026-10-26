import { describe, expect, it } from "vitest";
import { compareOf, defaultBase, parseRange } from "./range";

describe("parseRange", () => {
  it("reads two and three dots", () => {
    expect(parseRange("main..topic")).toEqual({ base: "main", target: "topic", mergeBase: false });
    expect(parseRange("main...topic")).toEqual({ base: "main", target: "topic", mergeBase: true });
  });
  it("fills an empty side with HEAD like git", () => {
    expect(parseRange("main...")).toEqual({ base: "main", target: "HEAD", mergeBase: true });
    expect(parseRange("..topic")).toEqual({ base: "HEAD", target: "topic", mergeBase: false });
  });
  it("is null for a plain revision or a broken range", () => {
    expect(parseRange("HEAD~2")).toBeNull();
    expect(parseRange("a..b..c")).toBeNull();
  });
});

describe("compareOf", () => {
  it("treats an empty target as the work tree and an empty base as HEAD", () => {
    expect(compareOf("main", "", false)).toEqual({ base: "main", target: null, mergeBase: false });
    expect(compareOf(" ", "topic", true)).toEqual({ base: "HEAD", target: "topic", mergeBase: true });
  });
  it("uses a range typed into the base field", () => {
    expect(compareOf("v1...v2", "", false)).toEqual({ base: "v1", target: "v2", mergeBase: true });
  });
});

describe("defaultBase", () => {
  it("picks the main line but not the target itself", () => {
    expect(defaultBase(["topic", "main"], "topic")).toBe("main");
    expect(defaultBase(["main", "origin/main"], "main")).toBe("origin/main");
    expect(defaultBase(["topic"], "topic")).toBeNull();
  });
});
