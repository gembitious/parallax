// 리비전 하나 고르기(리비전 둘러보기 "리비전 둘러보기…"): 브랜치·태그 이름이나 아무 리비전(sha, `HEAD~2`)을 치고,
// 잠깐 멈추면 그 커밋을 칸 아래에 보인다. 풀지 못한 리비전은 고르지 않는다. 백엔드: plx-core/src/changes.rs ref_names.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { shortSha } from "../blame/format";
import type { CommitInfo } from "../blame/types";
import { call } from "../ipc";
import { isComposing } from "../keys";
import { filterByLabel } from "../palette/match";
import { highlightRuns } from "../quickopen/highlight";
import { Runs } from "../quickopen/QuickOpen";

/** plx-core/src/changes.rs RefName */
type RefName = { name: string; kind: "branch" | "remote" | "tag"; current: boolean };
type Suggestion = RefName & { label: string };

const KIND_NAME: Record<RefName["kind"], string> = { branch: "브랜치", remote: "원격", tag: "태그" };
const SHOWN = 50;
const RESOLVE_MS = 120;

type Resolved = { state: "empty" | "checking" | "ok" | "failed"; info?: CommitInfo; error?: string };

export function RevisionPicker({
  repo,
  title,
  onPick,
  onClose,
}: {
  /** 저장소 안의 아무 경로 */
  repo: string;
  title: string;
  /** 고른 커밋(푼 결과) */
  onPick(info: CommitInfo): void;
  onClose(): void;
}) {
  const [names, setNames] = useState<Suggestion[]>([]);
  const [text, setText] = useState("");
  const [selected, setSelected] = useState(-1);
  const [resolved, setResolved] = useState<Resolved>({ state: "empty" });
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    let disposed = false;
    call<RefName[]>("ref_names", { repo }).then(
      (list) => !disposed && setNames(list.map((r) => ({ ...r, label: r.name }))),
      () => {},
    );
    inputRef.current?.focus();
    return () => {
      disposed = true;
    };
  }, [repo]);

  useEffect(() => {
    const rev = text.trim();
    if (rev === "") {
      setResolved({ state: "empty" });
      return;
    }
    setResolved((r) => ({ state: "checking", info: r.info }));
    let disposed = false;
    const timer = window.setTimeout(() => {
      call<CommitInfo>("commit_info", { repo, rev }).then(
        (info) => !disposed && setResolved({ state: "ok", info }),
        (e: unknown) => !disposed && setResolved({ state: "failed", error: String(e) }),
      );
    }, RESOLVE_MS);
    return () => {
      disposed = true;
      window.clearTimeout(timer);
    };
  }, [text, repo]);

  const matched = useMemo(() => filterByLabel(text.trim(), names).slice(0, SHOWN), [text, names]);
  useEffect(() => setSelected(-1), [text]);
  useLayoutEffect(() => {
    if (selected >= 0) listRef.current?.children[selected]?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  /** 이름을 넣고(그 이름을 풀면) 바로 고른다 */
  const pickName = (name: string) => {
    setText(name);
    call<CommitInfo>("commit_info", { repo, rev: name }).then(onPick, (e: unknown) =>
      setResolved({ state: "failed", error: String(e) }),
    );
  };
  const submit = () => {
    if (resolved.state === "ok" && resolved.info) onPick(resolved.info);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    e.stopPropagation();
    if (isComposing(e.nativeEvent)) return;
    let handled = true;
    if (e.key === "ArrowDown" || (e.ctrlKey && (e.key === "n" || e.key === "j"))) {
      setSelected((i) => Math.min(matched.length - 1, i + 1));
    } else if (e.key === "ArrowUp" || (e.ctrlKey && (e.key === "p" || e.key === "k"))) {
      setSelected((i) => Math.max(-1, i - 1));
    } else if (e.key === "Enter") {
      const pick = matched[selected];
      if (pick) pickName(pick.item.name);
      else submit();
    } else if (e.key === "Escape") {
      if (selected >= 0) setSelected(-1);
      else onClose();
    } else {
      handled = false;
    }
    if (handled) e.preventDefault();
  };

  const status = () => {
    if (resolved.state === "empty") return <span className="muted">브랜치·태그·sha를 친다</span>;
    if (resolved.state === "failed") return <span className="error-text">{resolved.error}</span>;
    if (!resolved.info) return <span className="muted">찾는 중…</span>;
    return (
      <span className={resolved.state === "checking" ? "muted" : undefined}>
        <span className="mono">{shortSha(resolved.info.sha)}</span> {resolved.info.summary}
      </span>
    );
  };

  return (
    <div className="overlay top" role="dialog" aria-label={title} onClick={onClose}>
      <div className="help picker quick-open compare-picker" onClick={(e) => e.stopPropagation()}>
        <h2>{title}</h2>
        <div className="compare-fields">
          <label className="compare-field">
            <span className="compare-label">리비전</span>
            <input
              ref={inputRef}
              value={text}
              placeholder="브랜치·태그·sha (HEAD~3도 된다)"
              aria-label="리비전"
              spellCheck={false}
              autoComplete="off"
              onChange={(e) => setText(e.target.value)}
              onKeyDown={onKeyDown}
            />
            <span className="compare-resolved">{status()}</span>
          </label>
        </div>
        <ul className="picker-list quick-open-list" role="listbox" aria-label="브랜치·태그" ref={listRef}>
          {matched.map(({ item, match }, i) => (
            <li
              key={`${item.kind}:${item.name}`}
              role="option"
              aria-selected={i === selected}
              className={`quick-open-item palette-item${i === selected ? " selected" : ""}`}
              onMouseMove={() => i !== selected && setSelected(i)}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => pickName(item.name)}
            >
              <span className="quick-open-name mono">
                <Runs runs={highlightRuns(item.name, match.positions)} />
              </span>
              <span className="quick-open-dir">
                {KIND_NAME[item.kind]}
                {item.current ? " · HEAD" : ""}
              </span>
            </li>
          ))}
        </ul>
        <p className="muted">↑↓ 이름 고르기 · Enter 둘러보기 · Esc 닫기</p>
      </div>
    </div>
  );
}
