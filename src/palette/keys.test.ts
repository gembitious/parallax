import { describe, expect, it } from "vitest";
import { isPaletteKey } from "./keys";

const key = (k: string, mods: Partial<Record<"ctrlKey" | "metaKey" | "altKey" | "shiftKey", boolean>> = {}) => ({
  key: k,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  target: null,
  ...mods,
});

describe("isPaletteKey", () => {
  it(": 와 Ctrl·⌘+Shift+P", () => {
    expect(isPaletteKey(key(":", { shiftKey: true }))).toBe(true);
    expect(isPaletteKey(key("P", { ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(isPaletteKey(key("p", { metaKey: true, shiftKey: true }))).toBe(true);
  });

  it("Quick Open 키와 겹치지 않는다", () => {
    expect(isPaletteKey(key("p", { ctrlKey: true }))).toBe(false);
    expect(isPaletteKey(key(";"))).toBe(false);
    expect(isPaletteKey(key("P", { ctrlKey: true, shiftKey: true, altKey: true }))).toBe(false);
  });
});
