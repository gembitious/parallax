// 명령 팔레트를 여는 키. 순수 함수라 단위 테스트한다(keys.test.ts).

type KeyLike = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "target">;

/**
 * 이 키로 팔레트를 연다: `:`(입력란 밖), Ctrl+Shift+P·⌘⇧P(어디서나). 코드 뷰(커밋 상세의 diff) 안에서도 연다.
 * blame 코드 뷰는 자기 키맵이 먼저 먹으므로(defaultPrevented) 두 번 열지 않는다.
 */
export function isPaletteKey(e: KeyLike): boolean {
  if (e.altKey) return false;
  if (e.ctrlKey || e.metaKey) return e.shiftKey && (e.key === "p" || e.key === "P");
  const target = e.target as Element | null;
  return e.key === ":" && !target?.closest?.("input, textarea");
}
