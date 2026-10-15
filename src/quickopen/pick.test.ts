import { describe, expect, it } from "vitest";
import { isQuickOpenKey, quickOpenNavigation } from "./pick";

const key = (k: string, mods: Partial<Record<"ctrlKey" | "metaKey" | "altKey" | "shiftKey", boolean>> = {}) => ({
  key: k,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  target: null,
  ...mods,
});

describe("quickOpenNavigation", () => {
  it("Enter는 작업트리 파일 blame(줄 포함)", () => {
    expect(quickOpenNavigation({ repo: "/r", path: "src/a.rs", line: 12, history: false })).toEqual({
      kind: "file",
      target: { repo: "/r", path: "src/a.rs", rev: null },
      line: 12,
    });
  });

  it("Shift+Enter는 파일 히스토리", () => {
    expect(quickOpenNavigation({ repo: "/r", path: "src/a.rs", line: null, history: true })).toEqual({
      kind: "history",
      target: { repo: "/r", path: "src/a.rs", rev: null },
    });
  });
});

describe("isQuickOpenKey", () => {
  it("f와 Ctrl·⌘+P", () => {
    expect(isQuickOpenKey(key("f"))).toBe(true);
    expect(isQuickOpenKey(key("p", { ctrlKey: true }))).toBe(true);
    expect(isQuickOpenKey(key("p", { metaKey: true }))).toBe(true);
    expect(isQuickOpenKey(key("P", { ctrlKey: true })), "Caps Lock").toBe(true);
  });

  it("다른 키는 아니다", () => {
    expect(isQuickOpenKey(key("F"))).toBe(false);
    expect(isQuickOpenKey(key("p"))).toBe(false);
    expect(isQuickOpenKey(key("f", { ctrlKey: true }))).toBe(false);
    expect(isQuickOpenKey(key("P", { ctrlKey: true, shiftKey: true })), "커맨드 팔레트 자리").toBe(false);
    expect(isQuickOpenKey(key("p", { ctrlKey: true, altKey: true }))).toBe(false);
  });
});
