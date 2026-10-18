// 워크트리 고르기(`W`): 주 작업트리와 링크된 워크트리를 보이고 하나를 고르면 화면이 그 워크트리로 간다.
// 백엔드: plx-core/src/worktree.rs list.
import { useEffect, useRef, useState } from "react";
import { relativeTime, shortSha } from "../blame/format";
import { call } from "../ipc";

/** plx-core/src/worktree.rs Worktree */
export type Worktree = {
  path: string;
  name: string;
  id: string | null;
  main: boolean;
  current: boolean;
  branch: string | null;
  head: string | null;
  summary: string | null;
  time: number | null;
  locked: boolean;
  missing: boolean;
};

type State = { kind: "loading" } | { kind: "loaded"; worktrees: Worktree[] } | { kind: "failed"; message: string };

export function WorktreePicker({
  repo,
  onPick,
  onClose,
}: {
  /** 저장소 안의 아무 경로 */
  repo: string;
  onPick(worktree: Worktree): void;
  onClose(): void;
}) {
  const [state, setState] = useState<State>({ kind: "loading" });
  const [selected, setSelected] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const now = useRef(Date.now());

  useEffect(() => {
    let disposed = false;
    call<Worktree[]>("worktrees", { repo }).then(
      (worktrees) => {
        if (disposed) return;
        setState({ kind: "loaded", worktrees });
        // 지금 보는 작업트리 다음 것부터 고른다(보통 그리로 가려고 연다).
        const current = worktrees.findIndex((w) => w.current);
        setSelected(worktrees.length > 1 ? (current + 1) % worktrees.length : 0);
      },
      (e: unknown) => !disposed && setState({ kind: "failed", message: String(e) }),
    );
    listRef.current?.focus();
    return () => {
      disposed = true;
    };
  }, [repo]);

  const worktrees = state.kind === "loaded" ? state.worktrees : [];
  const pick = (index: number) => {
    const worktree = worktrees[index];
    if (worktree && !worktree.missing) onPick(worktree);
  };

  return (
    <div className="overlay" role="dialog" aria-label="워크트리" onClick={onClose}>
      <div
        className="help picker"
        ref={listRef}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          // 아래 화면의 키(j/k, q …)로 가지 않게 여기서 멈춘다.
          e.stopPropagation();
          let handled = true;
          switch (e.key) {
            case "j":
            case "ArrowDown":
              setSelected((i) => Math.min(worktrees.length - 1, i + 1));
              break;
            case "k":
            case "ArrowUp":
              setSelected((i) => Math.max(0, i - 1));
              break;
            case "Enter":
              pick(selected);
              break;
            case "Escape":
            case "q":
            case "W":
              onClose();
              break;
            default:
              handled = false;
          }
          if (handled) e.preventDefault();
        }}
      >
        <h2>워크트리</h2>
        {state.kind === "loading" && <p className="muted">읽는 중…</p>}
        {state.kind === "failed" && <p className="error-text">{state.message}</p>}
        {state.kind === "loaded" && (
          <ul className="picker-list" role="listbox" aria-label="워크트리">
            {worktrees.map((worktree, i) => (
              <li
                key={worktree.path || worktree.name}
                role="option"
                aria-selected={i === selected}
                aria-disabled={worktree.missing}
                className={["picker-item", i === selected ? "selected" : "", worktree.missing ? "missing" : ""].join(
                  " ",
                )}
                onClick={() => setSelected(i)}
                onDoubleClick={() => pick(i)}
              >
                <span className="picker-name">{worktree.name}</span>
                <span className="badge mono">
                  {worktree.branch ?? (worktree.head ? `HEAD ${shortSha(worktree.head)}` : "커밋 없음")}
                </span>
                {worktree.current && <span className="badge">지금</span>}
                {worktree.main && <span className="badge">주 작업트리</span>}
                {worktree.locked && <span className="badge">잠김</span>}
                {worktree.missing && <span className="badge warn">폴더 없음</span>}
                <span className="picker-summary">{worktree.summary}</span>
                {worktree.time !== null && (
                  <span className="history-time">{relativeTime(worktree.time, now.current)}</span>
                )}
                <span className="picker-path mono">{worktree.path}</span>
              </li>
            ))}
          </ul>
        )}
        <p className="muted">j/k 고르기 · Enter 열기 · Esc 닫기</p>
      </div>
    </div>
  );
}
