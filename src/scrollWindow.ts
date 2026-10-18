// 긴 목록(그래프·히스토리)의 미리 그린 범위. 화면 위아래로 조금 더 그려 두고 그 안의 스크롤은 다시 렌더하지 않는다
// (행이 스크롤 내용 안에 있어 브라우저가 옮기기만 한다). 범위를 벗어날 때만 그 자리를 가운데로 다시 그린다.
// 순수 함수라 단위 테스트한다(scrollWindow.test.ts). 측정: bench/results.md "그래프 스크롤".
import { visibleRange } from "./history/list";

/** 화면 위아래로 미리 그려 두는 높이(px, 화면 높이까지). 400px은 다시 그리는 프레임이 늘어 더 나빴다. */
export const WINDOW_PAD_PX = 1000;

export type DrawnRange = { start: number; end: number; height: number };

/** 기준 스크롤 위치(`top`, 화면 `height`)에서 그릴 행 범위: 화면과 그 위아래 WINDOW_PAD_PX(화면 높이까지) */
export function drawWindow(top: number, height: number, rowHeight: number, count: number): { start: number; end: number } {
  const pad = Math.min(height, WINDOW_PAD_PX);
  return visibleRange(top - pad, height + 2 * pad, rowHeight, count, 0);
}

/** 스크롤 위치 `top`에서 보이는 행이 모두 그린 범위 안인가(다시 그리지 않아도 되나). 화면 높이가 바뀌면 아니다. */
export function insideDrawn(top: number, height: number, rowHeight: number, count: number, drawn: DrawnRange): boolean {
  const shown = visibleRange(top, height, rowHeight, count, 0);
  return shown.start >= drawn.start && shown.end <= drawn.end && height === drawn.height;
}
