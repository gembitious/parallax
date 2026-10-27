// 검색 결과 목록의 행(순수 함수, rows.test.ts): 파일 머리 행과 그 아래 맞은 줄 행. 접은 파일은 머리만 보인다.
import type { GrepFile } from "./types";

export type Row = { kind: "file"; file: number } | { kind: "line"; file: number; line: number };

/** 새로 온 묶음을 붙인다. 묶음의 첫 파일이 앞 묶음의 마지막 파일과 같으면 그 파일에 잇는다(백엔드가 경로 순으로 보낸다). */
export function appendFiles(files: readonly GrepFile[], more: readonly GrepFile[]): GrepFile[] {
  if (more.length === 0) return files as GrepFile[];
  const out = files.slice();
  let rest = more;
  const last = out[out.length - 1];
  if (last && last.path === more[0].path) {
    out[out.length - 1] = { path: last.path, lines: [...last.lines, ...more[0].lines] };
    rest = more.slice(1);
  }
  out.push(...rest);
  return out;
}

/** 보이는 행들. `collapsed`는 접은 파일의 경로다. */
export function buildRows(files: readonly GrepFile[], collapsed: ReadonlySet<string>): Row[] {
  const rows: Row[] = [];
  files.forEach((file, f) => {
    rows.push({ kind: "file", file: f });
    if (collapsed.has(file.path)) return;
    file.lines.forEach((_, l) => rows.push({ kind: "line", file: f, line: l }));
  });
  return rows;
}

/** `from`에서 `delta`(+1 다음, -1 앞)쪽으로 처음 만나는 맞은 줄 행. 없으면 null */
export function nextLineRow(rows: readonly Row[], from: number, delta: 1 | -1): number | null {
  for (let i = from + delta; i >= 0 && i < rows.length; i += delta) {
    if (rows[i].kind === "line") return i;
  }
  return null;
}

/** `from`에서 `delta`쪽 다음 파일의 머리 행. 없으면 null */
export function nextFileRow(rows: readonly Row[], from: number, delta: 1 | -1): number | null {
  for (let i = from + delta; i >= 0 && i < rows.length; i += delta) {
    if (rows[i].kind === "file") return i;
  }
  return null;
}

/** 다시 만든 행에서 앞에 고른 것(파일·줄)의 자리. 없으면 그 파일의 머리, 그것도 없으면 0 */
export function findRow(rows: readonly Row[], files: readonly GrepFile[], path: string | null, line: number | null): number {
  if (path === null) return 0;
  let head = -1;
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    if (files[row.file].path !== path) continue;
    if (row.kind === "file") head = i;
    else if (files[row.file].lines[row.line].line === line) return i;
  }
  return Math.max(0, head);
}

/** 입력란의 경로 칸을 pathspec들로: 공백·쉼표로 나눈다. */
export function splitPaths(text: string): string[] {
  return text.split(/[\s,]+/).filter((p) => p.length > 0);
}

/** 맞은 자리로 글을 나눈다(칠할 조각). */
export function splitRanges(text: string, ranges: readonly [number, number][]): { text: string; hit: boolean }[] {
  const out: { text: string; hit: boolean }[] = [];
  let at = 0;
  for (const [start, end] of ranges) {
    if (start > at) out.push({ text: text.slice(at, start), hit: false });
    out.push({ text: text.slice(start, end), hit: true });
    at = end;
  }
  if (at < text.length) out.push({ text: text.slice(at), hit: false });
  return out;
}
