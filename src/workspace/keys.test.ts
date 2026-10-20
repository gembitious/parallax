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
});
