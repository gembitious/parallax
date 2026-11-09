import { describe, expect, it } from "vitest";
import { keepSelection, matchesQuery, nextKind, shortRefName, trackingText, type RefItem, type Tracking } from "./types";

const ref = (name: string, extra: Partial<RefItem> = {}): RefItem => ({
  kind: "branch",
  name,
  full: `refs/heads/${name}`,
  sha: "a".repeat(40),
  current: false,
  worktrees: [],
  annotated: false,
  summary: "고친 것",
  author: "T",
  time: 0,
  tzOffset: 0,
  exists: true,
  ...extra,
});

const track = (ahead: number, behind: number, gone = false): Tracking => ({
  full: "refs/heads/main",
  upstream: "refs/remotes/origin/main",
  ahead,
  behind,
  gone,
});

describe("브랜치·태그 목록", () => {
  it("앞섬·뒤처짐 글", () => {
    expect(trackingText(track(0, 0))).toBe("=");
    expect(trackingText(track(2, 0))).toBe("↑2");
    expect(trackingText(track(0, 3))).toBe("↓3");
    expect(trackingText(track(2, 3))).toBe("↑2 ↓3");
    expect(trackingText(track(0, 0, true))).toBe("gone");
  });

  it("짧은 이름과 다음 종류", () => {
    expect(shortRefName("refs/remotes/origin/main")).toBe("origin/main");
    expect(shortRefName("refs/heads/a/b")).toBe("a/b");
    expect(shortRefName("HEAD")).toBe("HEAD");
    expect(nextKind("branch")).toBe("remote");
    expect(nextKind("tag")).toBe("branch");
  });

  it("거르기: 이름·요약·upstream·워크트리, 낱말은 모두", () => {
    const item = ref("claude/fix-login", { worktrees: [{ name: "wt-login", path: "/w", current: false }] });
    expect(matchesQuery(item, undefined, "")).toBe(true);
    expect(matchesQuery(item, undefined, "FIX")).toBe(true);
    expect(matchesQuery(item, undefined, "fix 고친")).toBe(true);
    expect(matchesQuery(item, undefined, "fix 없음")).toBe(false);
    expect(matchesQuery(item, undefined, "wt-login")).toBe(true);
    expect(matchesQuery(item, track(1, 0), "origin/main")).toBe(true);
  });

  it("다시 읽으면 같은 이름을 이어 고른다", () => {
    const before = [ref("a"), ref("b"), ref("c")];
    const after = [ref("new"), ref("a"), ref("b"), ref("c")];
    expect(keepSelection(before[1], after, 1)).toBe(2);
    expect(keepSelection(ref("gone"), after, 9)).toBe(3);
    expect(keepSelection(undefined, [], 2)).toBe(0);
  });
});
