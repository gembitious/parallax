// 결과 한 줄을 파일 이름과 디렉터리로 나누고 맞은 글자를 덩어리로 묶는다(명령 팔레트도 쓴다). 순수 함수라 단위
// 테스트한다.

export type Run = { text: string; hit: boolean };

/**
 * `path`를 디렉터리(끝 `/` 포함)와 파일 이름으로 나눈다. `positions`는 코드 포인트 위치(백엔드와 같다)라
 * 한글·이모지도 글자 단위로 칠한다.
 */
export function splitPath(path: string, positions: readonly number[]): { dir: Run[]; name: Run[] } {
  const chars = Array.from(path);
  const base = chars.lastIndexOf("/") + 1;
  const hits = new Set(positions);
  return { dir: runs(chars, 0, base, hits), name: runs(chars, base, chars.length, hits) };
}

/** `text` 전체를 맞은 글자(`positions`, 코드 포인트 위치)와 아닌 글자의 덩어리로 묶는다. */
export function highlightRuns(text: string, positions: readonly number[]): Run[] {
  const chars = Array.from(text);
  return runs(chars, 0, chars.length, new Set(positions));
}

function runs(chars: string[], start: number, end: number, hits: Set<number>): Run[] {
  const out: Run[] = [];
  for (let i = start; i < end; i++) {
    const hit = hits.has(i);
    const last = out[out.length - 1];
    if (last && last.hit === hit) last.text += chars[i];
    else out.push({ text: chars[i], hit });
  }
  return out;
}
