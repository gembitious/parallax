// Quick Open에서 고른 파일로 가는 탐색과 여는 키. 순수 함수라 단위 테스트한다(pick.test.ts).
import type { Navigation } from "../nav";

export type QuickOpenPick = {
  /** 작업트리 경로 */
  repo: string;
  /** 저장소 기준 경로 */
  path: string;
  /** `파일:줄`의 줄 */
  line: number | null;
  /** 파일 히스토리로(Shift+Enter) */
  history: boolean;
};

/** 고른 파일로 가는 탐색: 작업트리 파일의 blame(그 줄) 또는 파일 히스토리 */
export function quickOpenNavigation(pick: QuickOpenPick): Navigation {
  const target = { repo: pick.repo, path: pick.path, rev: null };
  return pick.history ? { kind: "history", target } : { kind: "file", target, line: pick.line };
}

type KeyLike = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "target">;

/** 이 키로 Quick Open을 연다: `f`(입력란·코드 뷰 밖), Ctrl+P·⌘P(어디서나) */
export function isQuickOpenKey(e: KeyLike): boolean {
  if (e.altKey || e.shiftKey) return false;
  if (e.ctrlKey || e.metaKey) return e.key === "p" || e.key === "P";
  const target = e.target as Element | null;
  return e.key === "f" && !target?.closest?.("input, textarea, .cm-editor");
}
