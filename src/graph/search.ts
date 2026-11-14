// 그래프 검색의 순수 계산: 입력 해석, 보이는 행 강조, 경로·코드 일치 행 사이 이동. 단위 테스트한다(search.test.ts).
import type { GraphRow } from "./types";

/**
 * 그래프 행 거르기(plx-core graph::LogFilter): 경로를 바꾼 커밋, 코드가 생기거나 사라진 커밋, 작성자(M18), 메시지,
 * 작성 기간. 준 것을 모두 만족하는 커밋이다. 뒤의 넷은 없으면 키를 두지 않는다.
 */
export type LogFilter = {
  path: string | null;
  /** `regex`가 아니면 `git log -S`(글이 든 횟수가 바뀐 커밋), 맞으면 `-G`(바뀐 줄이 정규식에 맞는 커밋) */
  pickaxe: { text: string; regex: boolean } | null;
  /** 작성자(`이름 <메일>`, mailmap을 따른다)에 든 글(`--author`, 글 그대로) */
  author?: string;
  /** 메시지에 든 글(`--grep`, 글 그대로). 거르기 앞에 친 글 */
  message?: string;
  /** 이 날짜 뒤(`--since`, `2026-01-01`, `2 weeks ago`) */
  since?: string;
  /** 이 날짜 전(`--until`) */
  until?: string;
};

export type Query = { kind: "text"; text: string } | { kind: "log"; filter: LogFilter };

const CODE = /^(?:code|코드):\s*(.*)$/i;
const REGEX = /^(?:regex|re|정규식):\s*(.*)$/i;
/** 뒤에 붙이는 거르기 이름 → 칸 */
const QUALIFIERS: Record<string, "path" | "author" | "since" | "until"> = {
  path: "path",
  경로: "path",
  author: "author",
  작성자: "author",
  since: "since",
  after: "since",
  이후: "since",
  until: "until",
  before: "until",
  이전: "until",
};
const QUALIFIER = Object.keys(QUALIFIERS).join("|");
/** 맨 뒤의 거르기 하나(` author:이름`): 앞(욕심껏) + 거르기 이름 + 값 */
const TRAILING = new RegExp(`^(.*\\S)\\s+(${QUALIFIER}):\\s*(.*)$`, "i");
/** 거르기로 시작하는 입력 */
const LEADING = new RegExp(`^(${QUALIFIER}):\\s*(.*)$`, "i");
/** 값 없는 코드 찾기 머리(`code:` 뒤에 바로 거르기가 오면 그 글을 찾는다) */
const BARE_CODE = /^(?:code|코드|regex|re|정규식):\s*$/i;

const cleanPath = (path: string) => path.trim().replace(/^\.\//, "");

/**
 * `path:경로`(또는 `경로:`)는 그 경로를 바꾼 커밋, `code:글`(`코드:`)은 그 글을 넣거나 지운 커밋(`git log -S`),
 * `regex:식`(`re:`, `정규식:`)은 바뀐 줄이 정규식에 맞는 커밋(`-G`), `author:이름`(`작성자:`)은 그 사람이 쓴 커밋,
 * `since:날짜`(`이후:`)·`until:날짜`(`이전:`)는 그 기간에 쓴 커밋이다. 거르기는 뒤에 이어 붙이고(`code:x author:kim
 * path:src`, 값은 다음 거르기까지), 거르기 앞에 친 글은 메시지에서 찾는다(`고침 author:kim`). 거르기가 없는 글은
 * 메시지·작성자·sha. 빈 입력은 null.
 */
export function parseQuery(input: string): Query | null {
  let rest = input.trim();
  const found: { path?: string; author?: string; since?: string; until?: string } = {};
  let qualified = false;
  const take = (name: string, value: string) => {
    qualified = true;
    const slot = QUALIFIERS[name.toLowerCase()] ?? QUALIFIERS[name];
    const clean = slot === "path" ? cleanPath(value) : value.trim();
    // 같은 거르기가 둘이면 뒤의 것(먼저 떼어 낸 것)을 쓴다. 빈 값은 버린다.
    if (clean && found[slot] === undefined) found[slot] = clean;
  };
  for (;;) {
    const m = TRAILING.exec(rest);
    if (!m || BARE_CODE.test(m[1])) break;
    take(m[2], m[3]);
    rest = m[1];
  }
  const leading = LEADING.exec(rest);
  if (leading) {
    take(leading[1], leading[2]);
    rest = "";
  }
  const filter: LogFilter = { path: found.path ?? null, pickaxe: null };
  if (found.author) filter.author = found.author;
  if (found.since) filter.since = found.since;
  if (found.until) filter.until = found.until;
  const code = CODE.exec(rest);
  const regex = code ? null : REGEX.exec(rest);
  const text = ((code ?? regex)?.[1] ?? "").trim();
  if (code || regex) {
    if (!text) return null;
    filter.pickaxe = { text, regex: regex !== null };
  } else if (rest) {
    if (!qualified) return { kind: "text", text: rest };
    filter.message = rest;
  }
  const empty =
    !filter.path && !filter.pickaxe && !filter.author && !filter.message && !filter.since && !filter.until;
  return empty ? null : { kind: "log", filter };
}

/** 거르기의 이름(상태 줄·알림) */
export function filterLabel(filter: LogFilter): string {
  const code = filter.pickaxe ? `${filter.pickaxe.regex ? "정규식" : "코드"} ${filter.pickaxe.text}` : null;
  return [
    filter.message ? `메시지 ${filter.message}` : null,
    code,
    filter.author ? `작성자 ${filter.author}` : null,
    filter.since ? `${filter.since} 이후` : null,
    filter.until ? `${filter.until} 이전` : null,
    filter.path ? `경로 ${filter.path}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** 일치 행의 이름(그래프 머리줄 "… 일치 n개") */
export function filterKind(filter: LogFilter): string {
  if (filter.pickaxe) return "코드";
  if (filter.author) return "작성자";
  if (filter.message) return "메시지";
  if (filter.since || filter.until) return "기간";
  return "경로";
}

export function sameFilter(a: LogFilter, b: LogFilter): boolean {
  return (
    a.path === b.path &&
    (a.author ?? null) === (b.author ?? null) &&
    (a.message ?? null) === (b.message ?? null) &&
    (a.since ?? null) === (b.since ?? null) &&
    (a.until ?? null) === (b.until ?? null) &&
    (a.pickaxe === null || b.pickaxe === null
      ? a.pickaxe === b.pickaxe
      : a.pickaxe.text === b.pickaxe.text && a.pickaxe.regex === b.pickaxe.regex)
  );
}

/** 작성자 보기에서 그래프로: 그 사람(`이름 <메일>`)이 쓴 커밋, `path`가 있으면 그 경로를 바꾼 것만 */
export function authorQuery(name: string, email: string, path: string | null): string {
  return `author:${name} <${email}>${path ? ` path:${path}` : ""}`;
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
