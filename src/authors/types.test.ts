import { describe, expect, it } from "vitest";
import { ident, keepAuthor, matchesQuery, ordered, sharePercent, type Author } from "./types";

const a = (name: string, email: string, commits: number, last: number): Author => ({ name, email, commits, first: 0, last });

describe("작성자 보기", () => {
  const list = [a("김철수", "kim@x", 10, 100), a("Lee", "lee@x", 3, 300), a("Park", "park@x", 3, 200)];

  it("최근 차례와 커밋 수 차례", () => {
    expect(ordered(list, "commits").map((x) => x.name)).toEqual(["김철수", "Lee", "Park"]);
    expect(ordered(list, "recent").map((x) => x.name)).toEqual(["Lee", "Park", "김철수"]);
    // 원래 목록은 그대로
    expect(list[0].name).toBe("김철수");
  });

  it("이름·메일로 거른다", () => {
    expect(list.filter((x) => matchesQuery(x, "LEE")).map(ident)).toEqual(["Lee <lee@x>"]);
    expect(list.filter((x) => matchesQuery(x, "철수"))).toHaveLength(1);
    expect(list.filter((x) => matchesQuery(x, " "))).toHaveLength(3);
  });

  it("몫", () => {
    expect(sharePercent(10, 16)).toBe("63%");
    expect(sharePercent(1, 3000)).toBe("0.1%");
    expect(sharePercent(1, 400)).toBe("0.3%");
    expect(sharePercent(0, 0)).toBe("0%");
  });

  it("다시 읽어도 고른 사람을 잇는다", () => {
    const next = [a("New", "n@x", 20, 500), ...list];
    expect(keepAuthor(list[1], next, 1)).toBe(2);
    expect(keepAuthor(a("Gone", "g@x", 1, 1), next, 9)).toBe(3);
    expect(keepAuthor(undefined, [], 2)).toBe(0);
  });
});
