import { describe, expect, it } from "vitest";
import { absolutePath } from "./paths";

describe("absolutePath", () => {
  it("macOS·Linux는 /로 잇는다", () => {
    expect(absolutePath("/home/me/repo", "src/한글.ts")).toBe("/home/me/repo/src/한글.ts");
    expect(absolutePath("/home/me/repo/", "a.rs")).toBe("/home/me/repo/a.rs");
  });

  it("Windows 경로는 \\로 잇는다", () => {
    expect(absolutePath("C:\\Users\\me\\repo", "src/a.rs")).toBe("C:\\Users\\me\\repo\\src\\a.rs");
    expect(absolutePath("D:/work/repo/", "a.rs")).toBe("D:\\work\\repo\\a.rs");
    expect(absolutePath("\\\\server\\share\\repo", "b/c.md")).toBe("\\\\server\\share\\repo\\b\\c.md");
  });
});
