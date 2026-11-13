// 이미지 diff의 순수 로직(M17): 화면에 맞춘 배율과 두 버전의 크기 글.

export type Size = { width: number; height: number };

/**
 * 두 그림을 같은 배율로 그릴 때의 배율. 큰 쪽(가로·세로 따로)이 자리에 들어가게 줄이고 키우지는 않는다
 * (아이콘을 흐리게 늘리지 않는다). 자리를 아직 모르면 1이다.
 */
export function fitScale(room: Size, sizes: (Size | null)[]): number {
  const known = sizes.filter((s): s is Size => s !== null && s.width > 0 && s.height > 0);
  if (known.length === 0 || room.width <= 0 || room.height <= 0) return 1;
  const width = Math.max(...known.map((s) => s.width));
  const height = Math.max(...known.map((s) => s.height));
  return Math.min(1, room.width / width, room.height / height);
}

/** `16×16` (모르면 빈 글) */
export const dimensions = (size: Size | null): string => (size ? `${size.width}×${size.height}` : "");

/** 그림 크기가 바뀌었으면 `16×16 → 32×32`, 아니면 null */
export function resized(old: Size | null, next: Size | null): string | null {
  if (!old || !next || (old.width === next.width && old.height === next.height)) return null;
  return `${dimensions(old)} → ${dimensions(next)}`;
}

/** 밀어 보기 손잡이 자리(0~100%)를 키로 옮긴다. */
export function nudge(at: number, step: number): number {
  return Math.max(0, Math.min(100, at + step));
}
