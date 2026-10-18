// 명령 팔레트 검색: 이름의 글자를 순서대로 담은 명령만 남기고, 단어 머리·연속 일치를 앞세운다.
// 한글 초성(`ㅍㅎ` → 파일 히스토리)과 대소문자 없는 비교. 명령은 수십 개라 브라우저에서 바로 센다(순수 함수, match.test.ts).

const INITIALS = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ";
const HANGUL_FIRST = 0xac00;
const HANGUL_LAST = 0xd7a3;
/** 초성 하나가 묶는 음절 수(중성 21 × 종성 28) */
const PER_INITIAL = 588;

const MATCH = 16;
const WORD_START = 10;
const CONSECUTIVE = 8;
const GAP = 1;
const NONE = -1e9;

/** 완성형 음절의 초성(호환 자모). 아니면 null */
export function initialOf(c: string): string | null {
  const code = c.codePointAt(0) ?? 0;
  return code >= HANGUL_FIRST && code <= HANGUL_LAST ? INITIALS[Math.floor((code - HANGUL_FIRST) / PER_INITIAL)] : null;
}

function same(q: string, c: string): boolean {
  if (q === c || q.toLowerCase() === c.toLowerCase()) return true;
  return INITIALS.includes(q) && initialOf(c) === q;
}

const isSeparator = (c: string | undefined) => c === undefined || /[\s·,./()\-_:↔]/.test(c);

export type Match = { score: number; positions: number[] };

/** `label`에서 `query`(공백은 무시)를 순서대로 맞춘 가장 좋은 점수와 위치(코드 포인트). 안 맞으면 null. */
export function matchLabel(query: string, label: string): Match | null {
  const q = Array.from(query.replace(/\s+/g, ""));
  if (q.length === 0) return { score: 0, positions: [] };
  const chars = Array.from(label);
  const m = q.length;
  const n = chars.length;
  if (m > n) return null;
  const score: number[][] = [];
  const from: number[][] = [];
  /** 그 칸까지 이어진 연속 덩어리 첫 글자의 단어 머리 보너스(연속 글자가 물려받는다, fzf와 같다) */
  const chunk: number[][] = [];
  for (let i = 0; i < m; i++) {
    score.push(new Array<number>(n).fill(NONE));
    from.push(new Array<number>(n).fill(-1));
    chunk.push(new Array<number>(n).fill(0));
    for (let j = i; j < n; j++) {
      if (!same(q[i], chars[j])) continue;
      const own = isSeparator(chars[j - 1]) ? WORD_START : 0;
      if (i === 0) {
        // 앞쪽 일치를 조금 앞세운다.
        score[i][j] = MATCH + own - j * 0.1;
        chunk[i][j] = own;
        continue;
      }
      let best = NONE;
      let at = -1;
      let bonus = own;
      for (let k = i - 1; k < j; k++) {
        const prev = score[i - 1][k];
        if (prev <= NONE) continue;
        const consecutive = k === j - 1;
        const inherited = consecutive ? Math.max(own, chunk[i - 1][k]) : own;
        const s = prev + MATCH + inherited + (consecutive ? CONSECUTIVE : -GAP * (j - k - 1));
        if (s > best) {
          best = s;
          at = k;
          bonus = inherited;
        }
      }
      if (at >= 0) {
        score[i][j] = best;
        from[i][j] = at;
        chunk[i][j] = bonus;
      }
    }
  }
  let end = -1;
  for (let j = 0; j < n; j++) if (score[m - 1][j] > NONE && (end < 0 || score[m - 1][j] > score[m - 1][end])) end = j;
  if (end < 0) return null;
  const positions: number[] = [];
  for (let i = m - 1, j = end; i >= 0; i--) {
    positions.push(j);
    j = from[i][j];
  }
  return { score: score[m - 1][end], positions: positions.reverse() };
}

/** 맞는 것만 점수 순으로(같으면 원래 순서). */
export function filterByLabel<T extends { label: string }>(query: string, items: readonly T[]): { item: T; match: Match }[] {
  const out: { item: T; match: Match; index: number }[] = [];
  items.forEach((item, index) => {
    const match = matchLabel(query, item.label);
    if (match) out.push({ item, match, index });
  });
  out.sort((a, b) => b.match.score - a.match.score || a.index - b.index);
  return out.map(({ item, match }) => ({ item, match }));
}
