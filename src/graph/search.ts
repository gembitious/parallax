// 그래프 검색의 순수 계산: 입력 해석, 보이는 행 강조, 경로 일치 행 사이 이동. 단위 테스트한다(search.test.ts).
import type { GraphRow } from "./types";

export type Query = { kind: "text"; text: string } | { kind: "path"; path: string };

/** `path:경로`(또는 `경로:`)는 그 경로를 바꾼 커밋, 나머지는 메시지·작성자·sha. 빈 입력은 null. */
export function parseQuery(input: string): Query | null {
  const trimmed = input.trim();
  const prefixed = /^(?:path|경로):\s*(.*)$/i.exec(trimmed);
  if (prefixed) {
    const path = prefixed[1].trim().replace(/^\.\//, "");
    return path ? { kind: "path", path } : null;
  }
  return trimmed ? { kind: "text", text: trimmed } : null;
}

/** 행에 보이는 것(요약·작성자·메일·sha 앞부분)만으로 본 일치. 메시지 본문은 백엔드 검색이 본다. */
export function rowMatches(row: GraphRow, text: string): boolean {
  const q = text.toLowerCase();
  return (
    row.sha.startsWith(q) ||
    row.summary.toLowerCase().includes(q) ||
    row.author.toLowerCase().includes(q) ||
    row.authorMail.toLowerCase().includes(q)
  );
}

/** 오름차순 `rows`에서 `from` 다음(또는 앞)의 행. 끝에 닿으면 반대쪽 끝으로 돈다. 없으면 null. */
export function nextRow(rows: readonly number[], from: number, forward: boolean): number | null {
  if (rows.length === 0) return null;
  // `from`보다 큰 첫 위치
  let lo = 0;
  let hi = rows.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (rows[mid] <= from) lo = mid + 1;
    else hi = mid;
  }
  if (forward) return lo < rows.length ? rows[lo] : rows[0];
  // `from`보다 작은 마지막 위치
  let before = lo - 1;
  if (before >= 0 && rows[before] === from) before -= 1;
  return before >= 0 ? rows[before] : rows[rows.length - 1];
}
