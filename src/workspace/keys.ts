// 작업 공간 키(트리·패널·탭·에디터 그룹). 어느 칸에서든(입력란 안도) 받는다. 순수 함수라 단위 테스트한다(keys.test.ts).

type KeyLike = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">;

export type WorkspaceKey =
  | "toggleTree"
  | "focusTree"
  | "togglePanel"
  | "nextTab"
  | "previousTab"
  | "closeTab"
  | "splitEditor"
  | "group1"
  | "group2"
  | "group3"
  | "group4";

export const IS_MAC = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);

/** 화면에 보일 수정 키 이름: macOS ⌘, 나머지 Ctrl+ */
export const MOD = IS_MAC ? "⌘" : "Ctrl+";

/**
 * Mod(macOS ⌘, 나머지 Ctrl)+B 트리, Mod+Shift+E 트리로, Mod+J 패널, Ctrl+Tab·Ctrl+Shift+Tab(Ctrl+PageDown·PageUp)
 * 탭 옮기기, Mod+W 탭 닫기(탭이 없으면 창 숨기기. macOS 메뉴 막대의 ⌘W와 같다), Mod+\ 에디터 나누기, Mod+1~4 그
 * 그룹으로(VS Code와 같다). 글자는 자판 자리(`code`)로도 본다: 한글 입력 상태면 `key`가 한글 자모고, 한국어 자판의 \는
 * ₩다.
 */
export function workspaceKey(e: KeyLike, mac = IS_MAC): WorkspaceKey | null {
  if (e.altKey) return null;
  const is = (letter: string) => e.code === `Key${letter.toUpperCase()}` || e.key.toLowerCase() === letter;
  if (e.ctrlKey && !e.metaKey) {
    if (e.key === "Tab") return e.shiftKey ? "previousTab" : "nextTab";
    if (!e.shiftKey && e.key === "PageDown") return "nextTab";
    if (!e.shiftKey && e.key === "PageUp") return "previousTab";
  }
  const mod = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!mod) return null;
  if (e.shiftKey) return is("e") ? "focusTree" : null;
  if (is("b")) return "toggleTree";
  if (is("j")) return "togglePanel";
  if (is("w")) return "closeTab";
  if (e.code === "Backslash" || e.key === "\\" || e.key === "₩") return "splitEditor";
  const digit = /^Digit([1-4])$/.exec(e.code)?.[1] ?? (/^[1-4]$/.test(e.key) ? e.key : null);
  if (digit) return `group${digit}` as WorkspaceKey;
  return null;
}
