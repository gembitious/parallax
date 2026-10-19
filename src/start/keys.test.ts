import { describe, expect, it } from "vitest";
import { isOpenRepoKey, startKey } from "./keys";

type Mods = Partial<Record<"ctrlKey" | "metaKey" | "altKey" | "shiftKey", boolean>>;

function key(k: string, mods: Mods = {}) {
  return { key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, target: null, ...mods };
}

describe("isOpenRepoKey", () => {
  it("Ctrl+O·⌘O", () => {
    expect(isOpenRepoKey(key("o", { ctrlKey: true }))).toBe(true);
    expect(isOpenRepoKey(key("o", { metaKey: true }))).toBe(true);
    expect(isOpenRepoKey(key("O", { metaKey: true }))).toBe(true);
  });
  it("수정 키 없는 o나 다른 조합은 아니다", () => {
    expect(isOpenRepoKey(key("o"))).toBe(false);
    expect(isOpenRepoKey(key("o", { ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(isOpenRepoKey(key("o", { metaKey: true, altKey: true }))).toBe(false);
    expect(isOpenRepoKey(key("p", { ctrlKey: true }))).toBe(false);
  });
});

describe("startKey", () => {
  it("목록 키", () => {
    expect(startKey(key("j"))).toBe("down");
    expect(startKey(key("ArrowDown"))).toBe("down");
    expect(startKey(key("k"))).toBe("up");
    expect(startKey(key("ArrowUp"))).toBe("up");
    expect(startKey(key("Enter"))).toBe("open");
    expect(startKey(key("Delete"))).toBe("forget");
    expect(startKey(key("Backspace"))).toBe("forget");
  });
  it("수정 키가 붙거나 다른 키면 아니다", () => {
    expect(startKey(key("j", { ctrlKey: true }))).toBe(null);
    expect(startKey(key("Enter", { metaKey: true }))).toBe(null);
    expect(startKey(key("x"))).toBe(null);
  });
});
