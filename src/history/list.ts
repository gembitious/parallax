// 히스토리 목록의 순수 계산: 가상 스크롤 범위와 검색. 단위 테스트한다(list.test.ts).
import type { LogEntry } from "./types";

/** 보이는 행과 앞뒤 여유 행의 범위 [start, end) */
export function visibleRange(
  scrollTop: number,
  height: number,
  rowHeight: number,
  count: number,
  overscan = 10,
): { start: number; end: number } {
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const last = Math.ceil((Math.max(0, scrollTop) + Math.max(0, height)) / rowHeight);
  return { start: Math.max(0, first - overscan), end: Math.min(count, last + overscan) };
}

/** `index` 행이 보이게 하는 scrollTop. 이미 다 보이면 그대로 둔다. */
export function scrollToRow(index: number, scrollTop: number, height: number, rowHeight: number): number {
  const top = index * rowHeight;
  const bottom = top + rowHeight;
  if (top < scrollTop) return top;
  if (bottom > scrollTop + height) return Math.max(0, bottom - height);
  return scrollTop;
}

/** 요약·작성자·메일·경로에 들어 있거나 sha가 그것으로 시작한다(대소문자 무시). */
export function matches(entry: LogEntry, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return false;
  return (
    entry.sha.startsWith(q) ||
    entry.summary.toLowerCase().includes(q) ||
    entry.author.toLowerCase().includes(q) ||
    entry.authorMail.toLowerCase().includes(q) ||
    entry.path.toLowerCase().includes(q)
  );
}

/** `from` 다음(또는 앞) 행부터 맞는 첫 행. 끝에 닿으면 반대쪽 끝에서 이어 찾는다. 없으면 -1. */
export function findMatch(entries: readonly LogEntry[], from: number, query: string, direction: 1 | -1): number {
  const n = entries.length;
  for (let step = 1; step <= n; step++) {
    const i = (((from + step * direction) % n) + n) % n;
    if (matches(entries[i], query)) return i;
  }
  return -1;
}

/** 이름이 바뀐 이력의 옛 경로들(최근에 바뀐 것부터, `path` 자신은 뺀다) */
export function formerPaths(entries: readonly LogEntry[], path: string): string[] {
  const seen = new Set<string>([path]);
  const out: string[] = [];
  for (const entry of entries) {
    for (const p of [entry.oldPath, entry.path]) {
      if (p && !seen.has(p)) {
        seen.add(p);
        out.push(p);
      }
    }
  }
  return out;
}
