// 저장소 검색(plx-core/src/grep.rs)의 모양.

/** plx-core/src/grep.rs GrepQuery */
export type GrepQuery = {
  pattern: string;
  regex: boolean;
  caseSensitive: boolean;
  word: boolean;
  /** 없으면 작업트리 */
  rev: string | null;
  paths: string[];
};

/** plx-core/src/grep.rs GrepLine */
export type GrepLine = {
  line: number;
  text: string;
  /** 맞은 자리 [시작, 끝), UTF-16 */
  ranges: [number, number][];
  cutStart: boolean;
  cutEnd: boolean;
};

/** plx-core/src/grep.rs GrepFile */
export type GrepFile = { path: string; lines: GrepLine[] };

/** plx-core/src/grep.rs GrepEvent */
export type GrepEvent =
  | { kind: "meta"; repo: string; rev: string | null }
  | { kind: "files"; files: GrepFile[] }
  | { kind: "done"; matches: number; files: number; truncated: boolean; firstMs: number; totalMs: number }
  | { kind: "error"; message: string }
  | { kind: "cancelled" };
