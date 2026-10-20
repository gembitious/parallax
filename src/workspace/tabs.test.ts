import { describe, expect, it } from "vitest";
import { tabLabel } from "./tabs";

const repo = "/r";
const sha = "0123456789abcdef0123456789abcdef01234567";

describe("tabLabel", () => {
  it("blame은 파일 이름과 리비전", () => {
    expect(tabLabel({ key: 1, view: "blame", target: { repo, path: "src/a.rs", rev: null }, line: null, via: "file" })).toEqual({
      name: "a.rs",
      detail: null,
      title: "blame src/a.rs",
    });
    expect(tabLabel({ key: 1, view: "blame", target: { repo, path: "src/a.rs", rev: sha }, line: 3, via: "parent" })).toMatchObject({
      name: "a.rs",
      detail: "01234567",
    });
  });

  it("히스토리·라인 히스토리, 저장소 전체", () => {
    expect(tabLabel({ key: 1, view: "history", target: { repo, path: "src/a.rs", rev: null }, lines: null, via: "history" })).toEqual({
      name: "a.rs 히스토리",
      detail: null,
      title: "히스토리 src/a.rs",
    });
    expect(
      tabLabel({ key: 1, view: "history", target: { repo, path: "src/a.rs", rev: sha }, lines: { start: 3, end: 9 }, via: "history" }),
    ).toMatchObject({ name: "a.rs 라인 히스토리", detail: "3-9 @01234567" });
    expect(tabLabel({ key: 1, view: "history", target: { repo, path: ".", rev: null }, lines: null, via: "history" }).name).toBe(
      "저장소 히스토리",
    );
  });

  it("커밋 상세와 그래프", () => {
    expect(tabLabel({ key: 1, view: "commit", repo, rev: sha, path: null, only: false, via: "commit" })).toEqual({
      name: "커밋 01234567",
      detail: null,
      title: "커밋 01234567",
    });
    expect(tabLabel({ key: 1, view: "graph", repo, focus: null, via: "graph" })).toMatchObject({ name: "그래프", detail: null });
  });
});
