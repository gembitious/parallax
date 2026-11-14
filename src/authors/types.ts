// 작성자 보기(M18). plx-core/src/authors.rs와 같은 모양과 화면의 순수 계산(정렬, 거르기, 몫). 단위 테스트: types.test.ts
import type { AuthorScope } from "../nav";

export type Author = {
  /** mailmap을 따른 이름 */
  name: string;
  email: string;
  /** merge가 아닌 커밋 수 */
  commits: number;
  /** 가장 이른·늦은 작성 시각(초) */
  first: number;
  last: number;
};

export type Authors = {
  repo: string;
  /** 이 경로를 바꾼 커밋만 셌다(없으면 저장소 전체) */
  path: string | null;
  scope: AuthorScope;
  commits: number;
  /** 커밋 수가 많은 사람부터 */
  authors: Author[];
  cached: boolean;
};

/** 목록 차례: 커밋 수(백엔드 차례) 또는 최근에 쓴 사람부터 */
export type AuthorOrder = "commits" | "recent";

export const SCOPE_LABEL: Record<AuthorScope, string> = { head: "HEAD", all: "모든 브랜치" };
export const ORDER_LABEL: Record<AuthorOrder, string> = { commits: "커밋 수", recent: "최근" };

export const otherScope = (scope: AuthorScope): AuthorScope => (scope === "head" ? "all" : "head");
export const otherOrder = (order: AuthorOrder): AuthorOrder => (order === "commits" ? "recent" : "commits");

/** `이름 <메일>`(git의 작성자 줄, 복사·그래프 거르기) */
export const ident = (author: Author) => `${author.name} <${author.email}>`;

export function ordered(authors: readonly Author[], order: AuthorOrder): Author[] {
  if (order === "commits") return [...authors];
  return [...authors].sort((a, b) => b.last - a.last || b.commits - a.commits || a.name.localeCompare(b.name));
}

/** 이름·메일에 든 글(대소문자 무시). 빈 글이면 모두 */
export function matchesQuery(author: Author, query: string): boolean {
  const q = query.trim().toLowerCase();
  return !q || author.name.toLowerCase().includes(q) || author.email.toLowerCase().includes(q);
}

/** 전체 커밋 가운데 몫(%). 1% 밑은 소수 한 자리 */
export function sharePercent(commits: number, total: number): string {
  if (total <= 0) return "0%";
  const p = (commits / total) * 100;
  if (p >= 1 || p === 0) return `${Math.round(p)}%`;
  return `${Math.max(0.1, Math.round(p * 10) / 10)}%`;
}

/** 다시 읽은 목록에서 고른 사람을 이어 간다(이름·메일로). 없으면 같은 자리 */
export function keepAuthor(before: Author | undefined, next: readonly Author[], index: number): number {
  if (before) {
    const at = next.findIndex((a) => a.name === before.name && a.email === before.email);
    if (at >= 0) return at;
  }
  return Math.max(0, Math.min(index, next.length - 1));
}

/** 이 컴퓨터 시간대의 날짜(`2026-11-14`) */
export function localDay(epochSec: number): string {
  const d = new Date(epochSec * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
