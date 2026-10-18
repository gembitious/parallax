import { describe, expect, it } from "vitest";
import { isNotices, noticeKey } from "./keys";

const key = (key: string, mods: Partial<Record<"ctrlKey" | "metaKey" | "altKey" | "shiftKey", boolean>> = {}) => ({
  key,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe("noticeKey", () => {
  it("Esc·q는 닫고 j/k·화살표·Page·Space·Home/End는 움직인다", () => {
    expect(noticeKey(key("Escape"))).toEqual({ kind: "close" });
    expect(noticeKey(key("q"))).toEqual({ kind: "close" });
    expect(noticeKey(key("j"))).toEqual({ kind: "lines", count: 1 });
    expect(noticeKey(key("ArrowUp"))).toEqual({ kind: "lines", count: -1 });
    expect(noticeKey(key("PageDown"))).toEqual({ kind: "pages", count: 1 });
    expect(noticeKey(key(" ", { shiftKey: true }))).toEqual({ kind: "pages", count: -1 });
    expect(noticeKey(key("End"))).toEqual({ kind: "edge", to: "bottom" });
  });

  it("아래 화면의 키는 먹고 복사만 브라우저에 맡긴다", () => {
    for (const k of ["f", "G", "o", "?", ":", "Enter", "Tab"]) expect(noticeKey(key(k))).toEqual({ kind: "block" });
    expect(noticeKey(key("p", { ctrlKey: true }))).toEqual({ kind: "block" });
    expect(noticeKey(key("P", { metaKey: true, shiftKey: true }))).toEqual({ kind: "block" });
    expect(noticeKey(key("c", { ctrlKey: true }))).toEqual({ kind: "pass" });
    expect(noticeKey(key("c", { metaKey: true }))).toEqual({ kind: "pass" });
  });
});

describe("isNotices", () => {
  it("빈 글과 개발 서버의 index.html은 고지가 아니다", () => {
    expect(isNotices("parallax 오픈 소스 라이선스\n")).toBe(true);
    expect(isNotices("")).toBe(false);
    expect(isNotices("  \n")).toBe(false);
    expect(isNotices("<!DOCTYPE html>\n<html>")).toBe(false);
  });
});
