// 두 리비전 비교 칸의 입력(순수 함수, range.test.ts).
import type { Compare } from "../nav";

/**
 * 기준 칸에 친 git 범위(`a..b`, `a...b`)를 푼다. git처럼 빈 쪽은 HEAD다(`main...` = `main...HEAD`).
 * 범위가 아니면 null.
 */
export function parseRange(text: string): Compare | null {
  const at = text.indexOf("..");
  if (at < 0) return null;
  const three = text.startsWith("...", at);
  const base = text.slice(0, at).trim() || "HEAD";
  const target = text.slice(at + (three ? 3 : 2)).trim() || "HEAD";
  if (target.includes("..")) return null;
  return { base, target, mergeBase: three };
}

/** 칸 둘과 갈라진 지점 선택으로 비교를 만든다. 기준이 비면 HEAD, 대상이 비면 작업트리다. 기준에 범위를 쳤으면 그것을 쓴다. */
export function compareOf(base: string, target: string, mergeBase: boolean): Compare {
  const range = target.trim() === "" ? parseRange(base) : null;
  if (range) return range;
  return { base: base.trim() || "HEAD", target: target.trim() || null, mergeBase };
}

/** 이름 목록에서 처음 보일 기준(본줄기)을 고른다. 대상과 같은 이름은 고르지 않는다. */
export function defaultBase(names: readonly string[], target: string | null): string | null {
  for (const name of ["main", "master", "trunk", "develop", "origin/main", "origin/master"]) {
    if (name !== target && names.includes(name)) return name;
  }
  return null;
}
