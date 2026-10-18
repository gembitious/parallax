// 그래프 행 캐시: 화면이 보는 범위만 묶음(CHUNK행) 단위로 백엔드에서 읽어 둔다. 순수 로직이라 단위 테스트한다(rows.test.ts).
import type { GraphRow, GraphWindow } from "./types";

/** 한 번에 읽는 행 수 */
export const CHUNK = 200;
/** 이만큼 넘는 묶음을 갖고 있으면 가장 오래 안 본 것부터 버린다(약 1만 행) */
export const MAX_CHUNKS = 50;

export class RowCache {
  private chunks = new Map<number, GraphRow[]>();
  /** 읽는 중인 묶음 */
  private pending = new Set<number>();

  /** `i`번 행(없으면 undefined). 본 묶음을 최근으로 올린다. */
  get(i: number): GraphRow | undefined {
    const index = Math.floor(i / CHUNK);
    const chunk = this.chunks.get(index);
    if (!chunk) return undefined;
    this.chunks.delete(index);
    this.chunks.set(index, chunk);
    return chunk[i - index * CHUNK];
  }

  /**
   * [start, end) 범위를 덮는데 아직 없거나 덜 받은 묶음(레이아웃이 `available`까지만 정해졌을 때). 요청을 보낼
   * 묶음은 읽는 중으로 표시한다.
   */
  take(start: number, end: number, available: number): number[] {
    const out: number[] = [];
    const last = Math.min(end, available);
    if (last <= start) return out;
    for (let index = Math.floor(start / CHUNK); index * CHUNK < last; index++) {
      if (this.pending.has(index)) continue;
      const chunk = this.chunks.get(index);
      const want = Math.min(CHUNK, available - index * CHUNK);
      if (chunk && chunk.length >= want) continue;
      this.pending.add(index);
      out.push(index);
    }
    return out;
  }

  /** 받은 창을 넣는다. 묶음 경계에서 시작한 창만 받는다. */
  put(window: GraphWindow): void {
    const index = Math.floor(window.start / CHUNK);
    this.pending.delete(index);
    if (window.start !== index * CHUNK || window.rows.length === 0) return;
    this.chunks.delete(index);
    this.chunks.set(index, window.rows);
    while (this.chunks.size > MAX_CHUNKS) {
      const oldest = this.chunks.keys().next().value as number;
      this.chunks.delete(oldest);
    }
  }

  /** 읽기에 실패한 묶음을 다시 읽을 수 있게 한다. */
  failed(index: number): void {
    this.pending.delete(index);
  }

  /** 첫 화면(0번부터의 행) */
  seed(rows: GraphRow[]): void {
    for (let start = 0; start < rows.length; start += CHUNK) {
      this.put({
        start,
        rows: rows.slice(start, start + CHUNK),
        total: 0,
        available: 0,
        width: 0,
        done: false,
      });
    }
  }
}

/** 행이 쓰는 가장 오른쪽 레인 + 1 */
export function rowWidth(row: GraphRow): number {
  let max = row.lane;
  for (const [from, to] of row.through) max = Math.max(max, from, to);
  for (const [lane] of row.up) max = Math.max(max, lane);
  for (const [lane] of row.down) max = Math.max(max, lane);
  return max + 1;
}
