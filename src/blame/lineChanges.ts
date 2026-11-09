// 작업트리 blame의 바뀐 줄 표시(M13): 백엔드가 meta에 보낸 HEAD ↔ 작업트리의 바뀐 줄을 줄마다의 표시로 바꾸고,
// `]`/`[`로 옮겨 갈 곳을 찾는다. 순수 함수라 단위 테스트한다(lineChanges.test.ts).

/** plx-core/src/line_changes.rs LineChange. 줄은 작업트리 내용의 줄(1부터). */
export type LineChange = {
  kind: "added" | "modified" | "deleted";
  /** 추가·수정: 첫 줄. 지움: 지운 자리 바로 앞 줄(맨 앞을 지웠으면 0) */
  start: number;
  /** 추가·수정: 마지막 줄(포함). 지움: start와 같다 */
  end: number;
  /** 지운 줄 수(수정이면 HEAD 쪽 줄 수) */
  removed: number;
};

/** 한 줄의 표시 */
export type LineMark = {
  kind: "added" | "modified" | null;
  /** 이 줄 아래(맨 앞을 지웠으면 첫 줄 위)에 지운 줄이 있다 */
  deleted: "below" | "above" | null;
  /** 그 자리에서 지운 줄 수 */
  deletedLines: number;
  /** 이 줄이 든 바뀐 묶음(지운 자리만 있으면 그 지움) */
  change: LineChange;
};

/** 줄 번호 → 표시. 표시가 없는 줄은 빠진다. `lineCount`를 넘는 줄은 버린다. */
export function lineMarks(changes: readonly LineChange[], lineCount: number): Map<number, LineMark> {
  const marks = new Map<number, LineMark>();
  for (const change of changes) {
    if (change.kind === "deleted") {
      const line = change.start === 0 ? 1 : change.start;
      if (line > lineCount) continue;
      const deleted = change.start === 0 ? "above" : "below";
      const mark = marks.get(line);
      if (mark) {
        mark.deleted = deleted;
        mark.deletedLines = change.removed;
      } else marks.set(line, { kind: null, deleted, deletedLines: change.removed, change });
      continue;
    }
    for (let line = change.start; line <= Math.min(change.end, lineCount); line++) {
      const mark = marks.get(line);
      if (mark) {
        mark.kind = change.kind;
        mark.change = change;
      } else marks.set(line, { kind: change.kind, deleted: null, deletedLines: 0, change });
    }
  }
  return marks;
}

/** 바뀐 묶음이 보이는 첫 줄(지움은 그 앞 줄, 맨 앞이면 1) */
const firstLine = (change: LineChange) => Math.max(1, change.start);

/**
 * `line`에서 `direction` 쪽으로 가장 가까운 다른 묶음의 첫 줄. 끝에 닿으면 반대쪽 끝으로 돈다(VS Code와 같다).
 * 바뀐 곳이 없으면 null.
 */
export function nextChange(changes: readonly LineChange[], line: number, direction: 1 | -1): number | null {
  if (changes.length === 0) return null;
  const starts = changes.map(firstLine);
  // 커서가 묶음 안에 있으면 그 묶음은 건너뛴다.
  const inside = changes.findIndex((c) => c.kind !== "deleted" && line >= c.start && line <= c.end);
  if (direction === 1) {
    const found = starts.find((start, i) => start > line && i !== inside);
    return found ?? starts[0];
  }
  for (let i = starts.length - 1; i >= 0; i--) {
    if (starts[i] < line && i !== inside) return starts[i];
  }
  return starts[starts.length - 1];
}

/** 풍선 도움말 */
export function markTitle(mark: LineMark): string {
  const parts: string[] = [];
  const { change } = mark;
  if (mark.kind === "added") parts.push(`HEAD 뒤에 더한 줄 (${change.end - change.start + 1}줄)`);
  if (mark.kind === "modified") parts.push(`HEAD 뒤에 바꾼 줄 (${change.removed}줄 → ${change.end - change.start + 1}줄)`);
  if (mark.deleted) parts.push(`이 줄 ${mark.deleted === "above" ? "위" : "아래"}에서 지운 줄 (${mark.deletedLines}줄)`);
  parts.push("누르면 작업트리 변경에서 이 파일을 본다");
  return parts.join("\n");
}
