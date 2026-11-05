// reflog·stash 목록(M10). plx-core/src/reflog.rs와 같은 모양.
import type { ReflogWhich } from "../nav";

export type ReflogPerson = { name: string; email: string; time: number; tzOffset: number };

export type ReflogItem = {
  /** `HEAD@{n}`·`stash@{n}`의 n */
  index: number;
  sha: string;
  previous: string | null;
  /** 일의 종류(commit, checkout, reset …). stash는 null */
  action: string | null;
  message: string;
  who: ReflogPerson;
  /** 커밋 요약. 커밋이 없으면(gc로 지웠다) null */
  summary: string | null;
  exists: boolean;
  /** stash -u의 추적하지 않던 파일을 담은 커밋 */
  untracked: string | null;
};

export type Reflog = { repo: string; which: ReflogWhich; entries: ReflogItem[]; truncated: boolean };

/** `HEAD@{n}`·`stash@{n}` */
export const refName = (which: ReflogWhich, index: number) => `${which === "stash" ? "stash" : "HEAD"}@{${index}}`;

/** 검색(`/`): 이름·sha·종류·메시지·요약·사람에서 대소문자 없이 찾는다. 빈 검색어는 모두 */
export function matchesQuery(item: ReflogItem, which: ReflogWhich, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (q === "") return true;
  const fields = [refName(which, item.index), item.sha, item.action ?? "", item.message, item.summary ?? "", item.who.name];
  return q.split(/\s+/).every((word) => fields.some((field) => field.toLowerCase().includes(word)));
}

/**
 * 다시 읽은 목록에서 앞에 고른 항목의 자리. 새 항목이 위에 붙으므로(번호가 밀린다) 번호가 아니라 내용으로 찾는다.
 * 없으면 같은 번호(목록 안으로 줄인다). 맨 위(가장 최근)를 고르고 있었으면 새 맨 위다(그래프·히스토리와 같다).
 */
export function keepSelection(before: ReflogItem | undefined, after: readonly ReflogItem[], index: number): number {
  if (after.length === 0 || index === 0) return 0;
  if (before) {
    const same = after.findIndex(
      (item) => item.sha === before.sha && item.who.time === before.who.time && item.message === before.message,
    );
    if (same >= 0) return same;
  }
  return Math.max(0, Math.min(after.length - 1, index));
}
