import { describe, expect, it } from "vitest";
import { workspaceKey } from "./keys";

const key = (key: string, mods: Partial<KeyboardEvent> = {}, code = "") => ({
  key,
  code,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe("workspaceKey", () => {
  it("Windows·Linux는 Ctrl, macOS는 ⌘", () => {
    expect(workspaceKey(key("b", { ctrlKey: true }), false)).toBe("toggleTree");
    expect(workspaceKey(key("b", { metaKey: true }), false)).toBeNull();
    expect(workspaceKey(key("b", { metaKey: true }), true)).toBe("toggleTree");
    expect(workspaceKey(key("b", { ctrlKey: true }), true)).toBeNull();
    expect(workspaceKey(key("j", { ctrlKey: true }), false)).toBe("togglePanel");
    expect(workspaceKey(key("E", { ctrlKey: true, shiftKey: true }), false)).toBe("focusTree");
    expect(workspaceKey(key("F", { ctrlKey: true, shiftKey: true }), false)).toBe("search");
    expect(workspaceKey(key("ㄹ", { metaKey: true, shiftKey: true }, "KeyF"), true)).toBe("search");
    expect(workspaceKey(key("b", { ctrlKey: true, shiftKey: true }), false)).toBeNull();
  });

  it("한글 입력 상태에서도 자판 자리로 안다", () => {
    expect(workspaceKey(key("ㅠ", { ctrlKey: true }, "KeyB"), false)).toBe("toggleTree");
    expect(workspaceKey(key("ㅓ", { metaKey: true }, "KeyJ"), true)).toBe("togglePanel");
  });

  it("탭 옮기기는 어느 플랫폼이든 Ctrl, 탭 닫기는 Mod+W", () => {
    expect(workspaceKey(key("Tab", { ctrlKey: true }), true)).toBe("nextTab");
    expect(workspaceKey(key("Tab", { ctrlKey: true, shiftKey: true }), false)).toBe("previousTab");
    expect(workspaceKey(key("PageDown", { ctrlKey: true }), false)).toBe("nextTab");
    expect(workspaceKey(key("PageUp", { ctrlKey: true }), true)).toBe("previousTab");
    expect(workspaceKey(key("w", { ctrlKey: true }), false)).toBe("closeTab");
    expect(workspaceKey(key("w", { metaKey: true }), true)).toBe("closeTab");
    expect(workspaceKey(key("w", { ctrlKey: true }), true)).toBeNull();
    expect(workspaceKey(key("Tab"), false)).toBeNull();
    expect(workspaceKey(key("b"), false)).toBeNull();
    expect(workspaceKey(key("b", { ctrlKey: true, altKey: true }), false)).toBeNull();
  });

  it("Mod+\\ 에디터 나누기(한국어 자판의 ₩), Mod+1~4 그룹으로", () => {
    expect(workspaceKey(key("\\", { ctrlKey: true }, "Backslash"), false)).toBe("splitEditor");
    expect(workspaceKey(key("₩", { metaKey: true }, "Backslash"), true)).toBe("splitEditor");
    expect(workspaceKey(key("₩", { ctrlKey: true }), false)).toBe("splitEditor");
    expect(workspaceKey(key("1", { ctrlKey: true }, "Digit1"), false)).toBe("group1");
    expect(workspaceKey(key("4", { metaKey: true }, "Digit4"), true)).toBe("group4");
    expect(workspaceKey(key("!", { ctrlKey: true, shiftKey: true }, "Digit1"), false)).toBeNull();
    expect(workspaceKey(key("5", { ctrlKey: true }, "Digit5"), false)).toBeNull();
    expect(workspaceKey(key("1"), false)).toBeNull();
  });
});
