// 그래프 레인 그리기: 그리는 범위(화면과 그 위아래)의 행의 선과 커밋 점을 캔버스 하나에 그린다. 캔버스는 스크롤 내용
// 안에 있어 범위 안의 스크롤은 다시 그리지 않는다(GraphView).
// 행 레이아웃(plx-core graph::RowLayout)은 행 하나만 보고 그릴 수 있게 반 칸 선으로 나뉘어 있다.
import type { GraphRow } from "./types";

/** 행 높이(px). 목록 행 높이와 같다. */
export const ROW_HEIGHT = 24;
export const LANE_WIDTH = 12;
/** 이보다 넓은 그래프는 잘라 그린다(오른쪽 끝에 표시). */
export const MAX_LANES = 24;
const PAD = 6;
const DOT_RADIUS = 3.5;

export type Palette = {
  lanes: string[];
  /** 속이 빈 점(merge) 안쪽 */
  background: string;
  /** HEAD 커밋 고리 */
  head: string;
  /** 잘린 레인 표시 */
  muted: string;
};

export const laneX = (lane: number): number => PAD + lane * LANE_WIDTH + LANE_WIDTH / 2;

/** 레인 `lanes`개를 그리는 칸의 너비(px) */
export const graphWidth = (lanes: number): number => PAD * 2 + Math.max(1, Math.min(lanes, MAX_LANES)) * LANE_WIDTH;

/** CSS 변수(`--lane-0`…)에서 색을 읽는다. 테마가 바뀌면 다시 읽는다. */
export function readPalette(element: Element): Palette {
  const style = getComputedStyle(element);
  const get = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
  return {
    lanes: Array.from({ length: 8 }, (_, i) => get(`--lane-${i}`, "#888")),
    background: get("--bg", "#fff"),
    head: get("--fg", "#000"),
    muted: get("--muted", "#888"),
  };
}

export type Visible = { index: number; row: GraphRow };

/**
 * `rows`를 그린다. 캔버스 좌표의 y = 행 번호 × ROW_HEIGHT − `scrollTop`(캔버스 맨 위 행의 위치).
 * `width`는 캔버스의 CSS 너비(px)다. 그보다 오른쪽 레인은 잘리고, 잘린 행은 오른쪽 끝을 흐리게 표시한다.
 */
export function drawRows(
  ctx: CanvasRenderingContext2D,
  rows: readonly Visible[],
  scrollTop: number,
  width: number,
  palette: Palette,
  head: string | null,
): void {
  const color = (c: number) => palette.lanes[c % palette.lanes.length];
  const h = ROW_HEIGHT;
  ctx.lineWidth = 1.6;
  ctx.lineCap = "round";
  const clipped: number[] = [];
  for (const { index, row } of rows) {
    const y0 = index * h - scrollTop;
    const ym = y0 + h / 2;
    const y1 = y0 + h;
    const xc = laneX(row.lane);
    for (const [from, to, c] of row.through) {
      ctx.strokeStyle = color(c);
      ctx.beginPath();
      const xf = laneX(from);
      const xt = laneX(to);
      ctx.moveTo(xf, y0);
      if (from === to) {
        ctx.lineTo(xf, y1);
      } else {
        // 같은 부모를 기다리던 레인을 합친다: 커밋 점 옆을 곧게 지난 뒤 아래에서 꺾는다.
        ctx.lineTo(xf, ym);
        ctx.bezierCurveTo(xf, ym + h * 0.3, xt, y1 - h * 0.25, xt, y1);
      }
      ctx.stroke();
    }
    for (const [lane, c] of row.up) {
      ctx.strokeStyle = color(c);
      ctx.beginPath();
      const x = laneX(lane);
      ctx.moveTo(x, y0);
      if (lane === row.lane) ctx.lineTo(xc, ym);
      else ctx.bezierCurveTo(x, y0 + h * 0.35, xc, ym - h * 0.25, xc, ym);
      ctx.stroke();
    }
    for (const [lane, c] of row.down) {
      ctx.strokeStyle = color(c);
      ctx.beginPath();
      const x = laneX(lane);
      ctx.moveTo(xc, ym);
      if (lane === row.lane) ctx.lineTo(x, y1);
      else ctx.bezierCurveTo(xc, ym + h * 0.25, x, y1 - h * 0.35, x, y1);
      ctx.stroke();
    }
    if (rowRight(row) > width) clipped.push(index);
  }
  for (const { index, row } of rows) {
    const ym = index * h - scrollTop + h / 2;
    // 잘린 레인의 커밋은 오른쪽 끝에 놓는다.
    const x = Math.min(laneX(row.lane), width - DOT_RADIUS - 1);
    const c = color(row.color);
    if (row.sha === head) {
      ctx.strokeStyle = palette.head;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(x, ym, DOT_RADIUS + 2.5, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.beginPath();
    ctx.arc(x, ym, DOT_RADIUS, 0, Math.PI * 2);
    if (row.parents.length > 1) {
      // merge는 속이 빈 점
      ctx.fillStyle = palette.background;
      ctx.fill();
      ctx.strokeStyle = c;
      ctx.lineWidth = 1.8;
      ctx.stroke();
    } else {
      ctx.fillStyle = c;
      ctx.fill();
    }
  }
  if (clipped.length > 0) {
    const fade = ctx.createLinearGradient(width - LANE_WIDTH, 0, width, 0);
    fade.addColorStop(0, "transparent");
    fade.addColorStop(1, palette.background);
    ctx.fillStyle = fade;
    for (const index of clipped) ctx.fillRect(width - LANE_WIDTH, index * h - scrollTop, LANE_WIDTH, h);
  }
}

/** 행이 쓰는 가장 오른쪽 x(px) */
function rowRight(row: GraphRow): number {
  let max = row.lane;
  for (const [from, to] of row.through) max = Math.max(max, from, to);
  for (const [lane] of row.down) max = Math.max(max, lane);
  return laneX(max) + LANE_WIDTH / 2;
}
