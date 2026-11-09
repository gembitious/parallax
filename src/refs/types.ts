// 브랜치·태그 목록(M14). plx-core/src/refs.rs와 같은 모양.
import type { RefKind } from "../nav";

export type RefWorktree = { name: string; path: string; current: boolean };

export type RefItem = {
  kind: RefKind;
  /** 짧은 이름(`main`, `origin/main`, `v1.0`) */
  name: string;
  /** 전체 이름(`refs/heads/main`) */
  full: string;
  /** 끝 커밋(태그는 벗긴 커밋) */
  sha: string;
  /** 지금 워크트리의 브랜치 */
  current: boolean;
  /** 이 브랜치를 체크아웃한 워크트리들(지금 워크트리 포함) */
  worktrees: RefWorktree[];
  /** 태그 객체가 있는 태그 */
  annotated: boolean;
  summary: string | null;
  author: string | null;
  /** 끝 커밋의 커미터 시각(초) */
  time: number;
  tzOffset: number;
  exists: boolean;
};

export type RefList = { repo: string; head: string | null; detached: boolean; refs: RefItem[]; truncated: boolean };

/** 로컬 브랜치의 upstream과 앞섬·뒤처짐(git for-each-ref) */
export type Tracking = { full: string; upstream: string; ahead: number; behind: number; gone: boolean };

export const KINDS: readonly RefKind[] = ["branch", "remote", "tag"];

export const KIND_LABEL: Record<RefKind, string> = { branch: "브랜치", remote: "원격", tag: "태그" };

/** `s`: 다음 종류 */
export const nextKind = (kind: RefKind): RefKind => KINDS[(KINDS.indexOf(kind) + 1) % KINDS.length];

/** `refs/remotes/origin/main` → `origin/main`, `refs/heads/dev` → `dev` */
export function shortRefName(full: string): string {
  for (const prefix of ["refs/heads/", "refs/remotes/", "refs/tags/"]) {
    if (full.startsWith(prefix)) return full.slice(prefix.length);
  }
  return full;
}

/** 앞섬·뒤처짐 글: `↑2 ↓1`, 같으면 `=`(upstream과 같다), upstream이 지워졌으면 `gone` */
export function trackingText(tracking: Tracking): string {
  if (tracking.gone) return "gone";
  if (tracking.ahead === 0 && tracking.behind === 0) return "=";
  return [tracking.ahead > 0 ? `↑${tracking.ahead}` : "", tracking.behind > 0 ? `↓${tracking.behind}` : ""]
    .filter(Boolean)
    .join(" ");
}

/** 거르기(`/`): 이름·sha·요약·작성자·upstream에서 대소문자 없이, 낱말은 모두 맞아야 한다 */
export function matchesQuery(item: RefItem, tracking: Tracking | undefined, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q === "") return true;
  const fields = [item.name, item.sha, item.summary ?? "", item.author ?? "", tracking ? shortRefName(tracking.upstream) : ""];
  for (const worktree of item.worktrees) fields.push(worktree.name);
  return q.split(/\s+/).every((word) => fields.some((field) => field.toLowerCase().includes(word)));
}

/** 다시 읽은 목록에서 앞에 고른 참조의 자리: 같은 이름, 없으면 같은 번호(목록 안으로) */
export function keepSelection(before: RefItem | undefined, after: readonly RefItem[], index: number): number {
  if (after.length === 0) return 0;
  if (before) {
    const same = after.findIndex((item) => item.full === before.full);
    if (same >= 0) return same;
  }
  return Math.max(0, Math.min(after.length - 1, index));
}
