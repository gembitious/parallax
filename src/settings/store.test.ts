import { describe, expect, it } from "vitest";
import { DEFAULTS, lineHeightFor, parseSettings } from "./store";
import { isSettingsKey } from "./Settings";

describe("설정 읽기", () => {
  it("틀린 칸만 기본값으로", () => {
    expect(parseSettings(null)).toEqual(DEFAULTS);
    expect(parseSettings([])).toEqual(DEFAULTS);
    expect(parseSettings({ theme: "dark", codeFontSize: 15, editor: " zed " })).toEqual({
      theme: "dark",
      codeFontSize: 15,
      editor: "zed",
    });
    expect(parseSettings({ theme: "blue", codeFontSize: 15.5, editor: 3 })).toEqual({ ...DEFAULTS });
    expect(parseSettings({ codeFontSize: 9 }).codeFontSize).toBe(13);
    expect(parseSettings({ codeFontSize: 21 }).codeFontSize).toBe(13);
    expect(parseSettings({ codeFontSize: 20 }).codeFontSize).toBe(20);
  });

  it("줄 높이는 13px에 20px인 비율", () => {
    expect(lineHeightFor(13)).toBe(20);
    expect(lineHeightFor(10)).toBe(15);
    expect(lineHeightFor(16)).toBe(25);
  });
});

describe("설정 키", () => {
  const key = (k: Partial<KeyboardEvent>) => ({ key: ",", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...k });
  it("macOS ⌘, · 나머지 Ctrl+,", () => {
    expect(isSettingsKey(key({ metaKey: true }), true)).toBe(true);
    expect(isSettingsKey(key({ ctrlKey: true }), true)).toBe(false);
    expect(isSettingsKey(key({ ctrlKey: true }), false)).toBe(true);
    expect(isSettingsKey(key({ ctrlKey: true, shiftKey: true }), false)).toBe(false);
    expect(isSettingsKey(key({}), false)).toBe(false);
  });
});
