import { describe, expect, it } from "vitest";
import { highlightRuns, splitPath } from "./highlight";

describe("highlightRuns", () => {
  it("글 전체를 덩어리로 묶는다", () => {
    expect(highlightRuns("파일 히스토리", [0, 3])).toEqual([
      { text: "파", hit: true },
      { text: "일 ", hit: false },
      { text: "히", hit: true },
      { text: "스토리", hit: false },
    ]);
    expect(highlightRuns("", [])).toEqual([]);
    expect(highlightRuns("ab", [])).toEqual([{ text: "ab", hit: false }]);
  });
});

describe("splitPath", () => {
  it("파일 이름과 디렉터리로 나누고 맞은 글자를 묶는다", () => {
    expect(splitPath("src/graph/GraphView.tsx", [10, 15])).toEqual({
      dir: [{ text: "src/graph/", hit: false }],
      name: [
        { text: "G", hit: true },
        { text: "raph", hit: false },
        { text: "V", hit: true },
        { text: "iew.tsx", hit: false },
      ],
    });
  });

  it("디렉터리 쪽 일치도 칠한다", () => {
    expect(splitPath("src/a.rs", [0, 1, 4])).toEqual({
      dir: [
        { text: "sr", hit: true },
        { text: "c/", hit: false },
      ],
      name: [
        { text: "a", hit: true },
        { text: ".rs", hit: false },
      ],
    });
  });

  it("위치는 코드 포인트 단위다", () => {
    // 🎉는 UTF-16 두 단위지만 한 글자다
    expect(splitPath("소스/🎉파일.rs", [3, 4])).toEqual({
      dir: [{ text: "소스/", hit: false }],
      name: [
        { text: "🎉파", hit: true },
        { text: "일.rs", hit: false },
      ],
    });
  });

  it("디렉터리 없는 경로", () => {
    expect(splitPath("README.md", [])).toEqual({ dir: [], name: [{ text: "README.md", hit: false }] });
  });
});
