import { describe, expect, it } from "vitest";
import { reloadFocus, type ReloadView } from "./focus";

const ROW = 22;
const view = (patch: Partial<ReloadView>): ReloadView => ({
  pending: null,
  scrollTop: 0,
  rowHeight: ROW,
  selected: 0,
  selectedSha: "s0",
  firstVisibleSha: "s0",
  ...patch,
});

describe("reloadFocus", () => {
  it("처음 요청을 아직 이루지 못했으면 그것을 이어 간다", () => {
    const pending = { kind: "rev" as const, rev: "old" };
    expect(reloadFocus(view({ pending }))).toBe(pending);
    expect(reloadFocus(view({ pending: { kind: "head" } }))).toEqual({ kind: "head" });
  });

  it("맨 위를 보고 있었으면 맨 위(새 커밋)", () => {
    expect(reloadFocus(view({}))).toEqual({ kind: "top" });
    expect(reloadFocus(view({ scrollTop: ROW - 1 }))).toEqual({ kind: "top" });
  });

  it("고른 커밋을 화면의 같은 자리에", () => {
    expect(reloadFocus(view({ scrollTop: 10 * ROW, selected: 15, selectedSha: "s15" }))).toEqual({
      kind: "rev",
      rev: "s15",
      offset: 5 * ROW,
    });
    // 맨 위 행을 골랐어도 스크롤했으면 그 커밋을 지킨다
    expect(reloadFocus(view({ scrollTop: 3 * ROW, selected: 0, selectedSha: "s0" }))).toEqual({
      kind: "rev",
      rev: "s0",
      offset: -3 * ROW,
    });
  });

  it("고른 커밋을 모르면(묶음이 캐시에서 밀려났다) 보이는 첫 행을 같은 자리에", () => {
    expect(
      reloadFocus(view({ scrollTop: 5000 * ROW + 7, selected: 0, selectedSha: null, firstVisibleSha: "f" })),
    ).toEqual({ kind: "rev", rev: "f", offset: -7 });
    expect(reloadFocus(view({ scrollTop: 50 * ROW, selected: 3, selectedSha: null, firstVisibleSha: null }))).toEqual({
      kind: "top",
    });
  });
});
