import { describe, expect, it } from "vitest";
import { progressLabel, progressTitle, type Progress } from "./progress";

const rebase: Progress = {
  operation: "rebase",
  step: 2,
  total: 3,
  branch: "feat",
  onto: "2f8472d7306ac783dead21894060cbe628e93e50",
  applying: [{ sha: "ae9bd7e572869abc74df6328044485090111bbe2", summary: "곁 둘" }],
  sequence: false,
};

describe("progressLabel", () => {
  it("단계와 충돌 수", () => {
    expect(progressLabel(rebase, 1)).toBe("rebase 2/3 · 충돌 1");
    expect(progressLabel(rebase, 0)).toBe("rebase 2/3");
  });
  it("단계가 없으면 '중'", () => {
    expect(progressLabel({ ...rebase, operation: "merge", step: null, total: null }, 2)).toBe("merge 중 · 충돌 2");
    expect(progressLabel({ ...rebase, operation: "cherryPick", step: null, total: null }, 0)).toBe("cherry-pick 중");
  });
});

describe("progressTitle", () => {
  it("브랜치·onto·적용 중인 커밋·이어 가는 명령", () => {
    const title = progressTitle(rebase, 1);
    expect(title).toContain("feat 브랜치를 2f8472d7 위로 rebase하는 중");
    expect(title).toContain("3개 가운데 2번째");
    expect(title).toContain("적용 중인 커밋: ae9bd7e5 곁 둘");
    expect(title).toContain("충돌 파일 1개");
    expect(title).toContain("git rebase --continue");
  });
  it("detached HEAD, 지워진 커밋, octopus merge", () => {
    expect(progressTitle({ ...rebase, branch: null }, 0)).toContain("detached HEAD를");
    expect(progressTitle({ ...rebase, applying: [{ sha: rebase.onto!, summary: null }] }, 0)).toContain("2f8472d7 (커밋 없음)");
    const octopus: Progress = { ...rebase, operation: "merge", step: null, total: null, applying: [rebase.applying[0], rebase.applying[0]] };
    expect(progressTitle(octopus, 0)).toContain("커밋 2개를 합치는 중");
  });
});
