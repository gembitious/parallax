// 화면 키 나누기: 어느 화면에서든 먹는 키(Quick Open·명령 팔레트)와 화면이 받는 키. 순수 함수라 단위 테스트한다
// (keys.test.ts). 네 화면(blame·커밋 상세·히스토리·그래프)이 같은 규칙을 쓴다.
import { isPaletteKey } from "./palette/keys";
import { isQuickOpenKey } from "./quickopen/pick";

type KeyLike = Pick<
  KeyboardEvent,
  "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "target" | "defaultPrevented" | "preventDefault"
>;

/** 어느 화면에서든 여는 것 */
export type GlobalKeys = { files(): void; palette(): void };

/**
 * 맨 위 화면이 받은 keydown을 나눈다.
 * - `"done"`: Quick Open·명령 팔레트 키라 열고 기본 동작(Ctrl+P의 인쇄 등)을 막았다.
 * - `"skip"`: 화면 키가 아니다. 다른 곳(코드 뷰·입력란)이 이미 먹었거나, 수정 키(Ctrl·⌘·Alt)가 붙었거나, 입력란에서 쳤다.
 * - `"view"`: 화면이 받을 키다.
 */
export function routeKey(e: KeyLike, open: GlobalKeys): "done" | "skip" | "view" {
  if (e.defaultPrevented) return "skip";
  const global = isPaletteKey(e) ? open.palette : isQuickOpenKey(e) ? open.files : null;
  if (global) {
    global();
    e.preventDefault();
    return "done";
  }
  if (e.ctrlKey || e.metaKey || e.altKey) return "skip";
  return (e.target as Element | null)?.closest?.("input, textarea") ? "skip" : "view";
}
