import { describe, expect, it } from "vitest";
import { filterByLabel, initialOf, matchLabel } from "./match";

const labels = (query: string, items: string[]) => filterByLabel(query, items.map((label) => ({ label }))).map((r) => r.item.label);

describe("matchLabel", () => {
  it("글자를 순서대로 담아야 맞는다", () => {
    expect(matchLabel("히스토리", "파일 히스토리")).not.toBeNull();
    expect(matchLabel("리토스히", "파일 히스토리")).toBeNull();
    expect(matchLabel("", "아무거나")).toEqual({ score: 0, positions: [] });
  });

  it("공백은 무시하고 대소문자를 가리지 않는다", () => {
    expect(matchLabel("파일히스", "파일 히스토리")?.positions).toEqual([0, 1, 3, 4]);
    expect(matchLabel("SHA", "커밋 sha 복사")?.positions).toEqual([3, 4, 5]);
  });

  it("초성으로 찾는다", () => {
    expect(matchLabel("ㅍㅎ", "파일 히스토리")?.positions).toEqual([0, 3]);
    expect(matchLabel("ㄹㅇ", "라인 히스토리")?.positions).toEqual([0, 1]);
    expect(initialOf("파")).toBe("ㅍ");
    expect(initialOf("a")).toBeNull();
  });

  it("단어 머리와 연속 일치를 앞세운다", () => {
    expect(labels("히", ["단축키 도움말", "라인 히스토리", "파일 히스토리"])).toEqual(["라인 히스토리", "파일 히스토리"]);
    expect(labels("그래프", ["커밋 그래프", "그 래 프 아님"])).toEqual(["커밋 그래프", "그 래 프 아님"]);
    expect(labels("ㅂㅅ", ["부모 리비전 blame", "커밋 sha 복사", "범위: 브랜치"])[0]).toBe("커밋 sha 복사");
  });

  it("빈 검색어는 원래 순서", () => {
    expect(labels("", ["b", "a", "c"])).toEqual(["b", "a", "c"]);
  });
});
