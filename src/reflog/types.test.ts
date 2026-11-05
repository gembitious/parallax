import { describe, expect, it } from "vitest";
import { keepSelection, matchesQuery, refName, type ReflogItem } from "./types";

const item = (index: number, sha: string, message: string, time = 1000 + index): ReflogItem => ({
  index,
  sha,
  previous: null,
  action: "commit",
  message,
  who: { name: "T", email: "t@example.com", time, tzOffset: 0 },
  summary: message,
  exists: true,
  untracked: null,
});

describe("reflog", () => {
  it("names entries like git", () => {
    expect(refName("head", 0)).toBe("HEAD@{0}");
    expect(refName("stash", 3)).toBe("stash@{3}");
  });

  it("matches every word in some field", () => {
    const a = item(2, "abcdef0123", "고친 둘째");
    expect(matchesQuery(a, "head", "")).toBe(true);
    expect(matchesQuery(a, "head", "HEAD@{2}")).toBe(true);
    expect(matchesQuery(a, "head", "abcdef 둘째")).toBe(true);
    expect(matchesQuery(a, "head", "COMMIT")).toBe(true);
    expect(matchesQuery(a, "head", "없음")).toBe(false);
  });

  it("keeps the selected entry when new entries arrive on top", () => {
    const before = [item(0, "b", "둘", 2), item(1, "a", "하나", 1)];
    const after = [item(0, "c", "셋", 3), item(1, "b", "둘", 2), item(2, "a", "하나", 1)];
    expect(keepSelection(before[1], after, 1)).toBe(2);
    // 맨 위를 보고 있었으면 새 맨 위
    expect(keepSelection(before[0], after, 0)).toBe(0);
    // 사라졌으면(stash drop) 같은 자리, 목록 안으로
    expect(keepSelection(item(5, "z", "없다"), after, 7)).toBe(2);
    expect(keepSelection(undefined, [], 3)).toBe(0);
  });
});
