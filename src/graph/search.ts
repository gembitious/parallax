// 그래프 검색의 순수 계산: 입력 해석, 보이는 행 강조, 경로·코드 일치 행 사이 이동. 단위 테스트한다(search.test.ts).
import type { GraphRow } from "./types";

/** 그래프 행 거르기(plx-core graph::LogFilter): 경로를 바꾼 커밋, 코드가 생기거나 사라진 커밋, 또는 둘 다 */
export type LogFilter = {
  path: string | null;
  /** `regex`가 아니면 `git log -S`(글이 든 횟수가 바뀐 커밋), 맞으면 `-G`(바뀐 줄이 정규식에 맞는 커밋) */
  pickaxe: { text: string; regex: boolean } | null;
};

export type Query = { kind: "text"; text: string } | { kind: "log"; filter: LogFilter };

const PATH = /^(?:path|경로):\s*(.*)$/i;
const CODE = /^(?:code|코드):\s*(.*)$/i;
const REGEX = /^(?:regex|re|정규식):\s*(.*)$/i;
/** 코드 검색어 뒤의 ` path:경로` */
const TRAILING_PATH = /^(.*?)\s+(?:path|경로):\s*(.*)$/i;

const cleanPath = (path: string) => path.trim().replace(/^\.\//, "");

/**
 * `path:경로`(또는 `경로:`)는 그 경로를 바꾼 커밋, `code:글`(`코드:`)은 그 글을 넣거나 지운 커밋(`git log -S`),
 * `regex:식`(`re:`, `정규식:`)은 바뀐 줄이 정규식에 맞는 커밋(`-G`)이다. 코드 검색어 뒤에 ` path:경로`를 붙이면
 * 그 경로에서만 찾는다. 나머지는 메시지·작성자·sha. 빈 입력은 null.
 */
export function parseQuery(input: string): Query | null {
  const trimmed = input.trim();
  const prefixed = PATH.exec(trimmed);
  if (prefixed) {
    const path = cleanPath(prefixed[1]);
    return path ? { kind: "log", filter: { path, pickaxe: null } } : null;
  }
  const code = CODE.exec(trimmed);
  const regex = code ? null : REGEX.exec(trimmed);
  const rest = (code ?? regex)?.[1];
  if (rest !== undefined) {
    const scoped = TRAILING_PATH.exec(rest);
    const text = (scoped ? scoped[1] : rest).trim();
    const path = scoped ? cleanPath(scoped[2]) : "";
    if (!text) return null;
    return { kind: "log", filter: { path: path || null, pickaxe: { text, regex: regex !== null } } };
  }
  return trimmed ? { kind: "text", text: trimmed } : null;
}

/** 거르기의 이름(상태 줄·알림) */
export function filterLabel(filter: LogFilter): string {
  const code = filter.pickaxe ? `${filter.pickaxe.regex ? "정규식" : "코드"} ${filter.pickaxe.text}` : null;
  const path = filter.path ? `경로 ${filter.path}` : null;
  return [code, path].filter(Boolean).join(" · ");
}

export function sameFilter(a: LogFilter, b: LogFilter): boolean {
  return (
    a.path === b.path &&
    (a.pickaxe === null || b.pickaxe === null
      ? a.pickaxe === b.pickaxe
      : a.pickaxe.text === b.pickaxe.text && a.pickaxe.regex === b.pickaxe.regex)
  );
}

/** 고른 글로 만든 코드 검색 입력(blame·diff에서 그래프로). 여러 줄이면 첫 줄, 공백만이면 null */
export function codeQuery(selected: string): string | null {
  const line = selected.split(/\r?\n/).find((l) => l.trim() !== "")?.trim();
  return line ? `code:${line.slice(0, 200)}` : null;
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
