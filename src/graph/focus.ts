// 그래프를 다시 읽을 때(저장소가 바뀜) 새 세션에서 고를 자리. 순수 함수라 단위 테스트한다(focus.test.ts).

/** 새 세션의 첫 화면에서 고를 행: 커밋(`offset`이 있으면 화면의 그 높이에), HEAD, 맨 위 */
export type Focus = { kind: "rev"; rev: string; offset?: number } | { kind: "head" } | { kind: "top" };

export type ReloadView = {
  /** 아직 이루지 못한 처음 요청(`G`로 연 커밋 등) */
  pending: Focus | null;
  scrollTop: number;
  rowHeight: number;
  selected: number;
  /** 고른 행의 sha(캐시에 없으면 기억한 값, 모르면 null) */
  selectedSha: string | null;
  /** 보이는 첫 행의 sha(모르면 null) */
  firstVisibleSha: string | null;
};

/**
 * 처음 요청을 아직 이루지 못했으면 그것을 이어 간다. 맨 위를 보고 있었으면 맨 위(새 커밋)를, 아니면 고른 커밋을
 * 화면의 같은 자리에 둔다. 고른 커밋을 모르면 보이는 첫 행을 같은 자리에 둔다.
 */
export function reloadFocus(view: ReloadView): Focus {
  if (view.pending) return view.pending;
  const { scrollTop: top, rowHeight, selected } = view;
  if (top < rowHeight && selected === 0) return { kind: "top" };
  if (view.selectedSha) return { kind: "rev", rev: view.selectedSha, offset: selected * rowHeight - top };
  if (view.firstVisibleSha) {
    const first = Math.floor(top / rowHeight);
    return { kind: "rev", rev: view.firstVisibleSha, offset: first * rowHeight - top };
  }
  return { kind: "top" };
}
