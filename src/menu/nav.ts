// 메뉴(우클릭 메뉴, 메뉴 막대의 드롭다운)의 자리와 키 이동. 순수 함수라 단위 테스트한다(nav.test.ts).

export type Point = { x: number; y: number };
export type Box = { left: number; top: number; right: number; bottom: number };
export type Size = { width: number; height: number };

/** 화면 가장자리에서 띄우는 간격(px) */
export const MENU_MARGIN = 4;

/**
 * 메뉴를 놓을 자리. 우클릭은 마우스 자리 오른쪽 아래, 메뉴 막대는 제목 단추 아래에 놓고, 창 밖으로 나가면 반대쪽으로
 * 접거나 창 안으로 민다. 메뉴가 창보다 높으면 위에 붙인다(메뉴가 스크롤한다).
 */
export function placeMenu(at: Point | Box, size: Size, viewport: Size): { left: number; top: number } {
  const maxLeft = viewport.width - MENU_MARGIN - size.width;
  const maxTop = viewport.height - MENU_MARGIN - size.height;
  let left: number;
  let top: number;
  // 제목 단추의 DOMRect에는 x·y도 있어 bottom으로 가른다.
  if ("bottom" in at) {
    left = at.left;
    top = at.bottom <= maxTop ? at.bottom : at.top - size.height;
  } else {
    left = at.x <= maxLeft ? at.x : at.x - size.width;
    top = at.y <= maxTop ? at.y : at.y - size.height;
  }
  left = Math.max(MENU_MARGIN, Math.min(left, maxLeft));
  top = Math.max(MENU_MARGIN, Math.min(top, maxTop));
  return { left, top };
}

/**
 * `from`에서 `delta`만큼 가며 고를 수 있는 다음 칸(끝에서는 처음으로 돈다). 고를 칸이 없으면 -1.
 * `from`이 -1(아무것도 고르지 않음)이면 아래로는 첫 칸, 위로는 마지막 칸이다.
 */
export function stepIndex(selectable: readonly boolean[], from: number, delta: 1 | -1): number {
  const n = selectable.length;
  if (n === 0) return -1;
  let i = from < 0 ? (delta > 0 ? -1 : n) : from;
  for (let step = 0; step < n; step++) {
    i = (i + delta + n) % n;
    if (selectable[i]) return i;
  }
  return -1;
}

/** 처음(`last`면 마지막) 고를 수 있는 칸. 없으면 -1 */
export function edgeIndex(selectable: readonly boolean[], last: boolean): number {
  return stepIndex(selectable, -1, last ? -1 : 1);
}
