// 저장소 열기 키와 첫 화면의 키. 순수 함수라 단위 테스트한다(keys.test.ts).

type KeyLike = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "target">;

/** 저장소 열기(OS 폴더 창): Ctrl+O·⌘O, 어느 화면에서든(입력란 안도) */
export function isOpenRepoKey(e: KeyLike): boolean {
  return (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && (e.key === "o" || e.key === "O");
}

/** 화면에 보일 저장소 열기 키 */
export const OPEN_REPO_KEYS =
  typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent) ? "⌘O" : "Ctrl+O";

export type StartAction = "up" | "down" | "open" | "forget";

/** 첫 화면의 최근 저장소 목록 키: ↑↓·j/k 고르기, Enter 열기, Delete·Backspace 목록에서 빼기 */
export function startKey(e: KeyLike): StartAction | null {
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  switch (e.key) {
    case "ArrowDown":
    case "j":
      return "down";
    case "ArrowUp":
    case "k":
      return "up";
    case "Enter":
      return "open";
    case "Delete":
    case "Backspace":
      return "forget";
    default:
      return null;
  }
}
