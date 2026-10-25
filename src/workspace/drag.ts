// 탭·패널 끌어 놓기의 놓을 곳. HTML 끌기(draggable) 대신 포인터 이벤트로 끈다: 창에 파일을 끌어 놓는 Tauri의 처리
// (dragDropEnabled, 저장소 열기)가 켜져 있으면 Windows WebView2에는 HTML 끌기 이벤트가 오지 않는다. 자리 셈은 순수
// 함수라 단위 테스트한다(drag.test.ts).
import type { TabDrop } from "./state";

/** 끄는 것: 탭 하나, 또는 패널의 화면 스택 */
export type DragSource = { kind: "tab"; tab: number } | { kind: "panel" };
/** 놓을 곳: 에디터 그룹(탭 막대의 자리, 화면 칸, 옆), 아래 패널(탭만) */
export type DropTarget = TabDrop | { kind: "panel" };

/** 그룹 화면 칸의 왼쪽·오른쪽 끝에서 이 몫 안에 놓으면 그쪽에 나눈다 */
export const SPLIT_EDGE = 0.25;
/** 이만큼(px) 움직여야 끌기다(그 전에 놓으면 누르기) */
export const DRAG_THRESHOLD = 6;

/** 화면 칸 안의 가로 자리(0~1, 왼쪽 끝이 0) → 왼쪽·오른쪽 끝이면 그쪽에 나누기, 가운데면 그 그룹 */
export function zoneAt(fraction: number, canSplit: boolean): "left" | "center" | "right" {
  if (!canSplit) return "center";
  if (fraction < SPLIT_EDGE) return "left";
  if (fraction > 1 - SPLIT_EDGE) return "right";
  return "center";
}

/** 탭 막대에서 끼울 자리: 포인터가 가운데보다 왼쪽인 첫 탭의 앞, 없으면 끝 */
export function insertIndex(centers: readonly number[], x: number): number {
  const at = centers.findIndex((center) => x < center);
  return at < 0 ? centers.length : at;
}

/**
 * 포인터 자리(창 좌표)의 놓을 곳. 탭 막대(`data-tabbar`=그룹)면 그 자리, 그룹의 화면 칸(`data-views`=그룹)이면 끝은 나누기·
 * 가운데는 그 그룹, 탭을 끌 때 아래 패널이면 패널. `canSplit`이 아니면(그룹이 이미 MAX_GROUPS) 나누지 않는다.
 */
export function dropTargetAt(x: number, y: number, source: DragSource, canSplit: boolean): DropTarget | null {
  const el = document.elementFromPoint(x, y);
  if (!el) return null;
  const bar = el.closest<HTMLElement>("[data-tabbar]");
  if (bar) {
    const centers = [...bar.querySelectorAll<HTMLElement>("[data-tab]")].map((tab) => {
      const rect = tab.getBoundingClientRect();
      return rect.left + rect.width / 2;
    });
    return { kind: "group", group: Number(bar.dataset.tabbar), index: insertIndex(centers, x) };
  }
  const views = el.closest<HTMLElement>("[data-views]");
  if (views) {
    const rect = views.getBoundingClientRect();
    const group = Number(views.dataset.views);
    const zone = rect.width > 0 ? zoneAt((x - rect.left) / rect.width, canSplit) : "center";
    return zone === "center" ? { kind: "group", group } : { kind: "split", group, side: zone };
  }
  if (source.kind === "tab" && el.closest(".panel")) return { kind: "panel" };
  return null;
}
