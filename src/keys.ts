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

/**
 * IME가 글자를 조합하는 중에 친 키(조합을 끝내는 Enter·Esc 등). 화면의 동작으로 받지 않는다. WebKit은
 * compositionend 뒤에 keydown을 보내 isComposing이 꺼진 채 keyCode 229로 온다.
 */
export const isComposing = (e: Pick<KeyboardEvent, "isComposing" | "keyCode">) => e.isComposing || e.keyCode === 229;

/**
 * 성능 HUD를 켜고 끄는 `` ` ``. 입력란(검색 칸)에서 친 글자, IME 조합 중인 키, 다른 곳이 먹은 키는 아니다.
 * 코드 뷰(blame)는 입력란이 아니다.
 */
export function isHudKey(e: KeyLike & Pick<KeyboardEvent, "isComposing" | "keyCode">): boolean {
  if (e.key !== "`" || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented || isComposing(e)) return false;
  return !(e.target as Element | null)?.closest?.("input, textarea");
}
