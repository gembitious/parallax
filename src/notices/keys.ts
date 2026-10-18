// 오픈 소스 라이선스 창의 키. 순수 함수라 단위 테스트한다(keys.test.ts).
type KeyLike = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">;

export type NoticeAction =
  | { kind: "close" }
  | { kind: "lines"; count: number }
  | { kind: "pages"; count: number }
  | { kind: "edge"; to: "top" | "bottom" }
  /** 브라우저에 맡긴다(고른 글 복사) */
  | { kind: "pass" }
  /** 아무것도 하지 않지만 아래 화면에도 가지 않게 먹는다 */
  | { kind: "block" };

/** 창이 열려 있는 동안의 키. 아래 화면의 키(`f`, `G`, Ctrl+P 등)는 모두 먹는다. */
export function noticeKey(e: KeyLike): NoticeAction {
  if (e.ctrlKey || e.metaKey) return e.key === "c" || e.key === "C" ? { kind: "pass" } : { kind: "block" };
  if (e.altKey) return { kind: "block" };
  switch (e.key) {
    case "Escape":
    case "q":
      return { kind: "close" };
    case "j":
    case "ArrowDown":
      return { kind: "lines", count: 1 };
    case "k":
    case "ArrowUp":
      return { kind: "lines", count: -1 };
    case "PageDown":
      return { kind: "pages", count: 1 };
    case "PageUp":
      return { kind: "pages", count: -1 };
    case " ":
      return { kind: "pages", count: e.shiftKey ? -1 : 1 };
    case "Home":
      return { kind: "edge", to: "top" };
    case "End":
      return { kind: "edge", to: "bottom" };
    default:
      return { kind: "block" };
  }
}

/** 받은 글이 고지인가(없는 파일이면 개발 서버가 index.html을 돌려줄 수 있다) */
export function isNotices(text: string): boolean {
  return text.trim().length > 0 && !/^\s*<!doctype html/i.test(text);
}
