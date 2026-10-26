// 두 리비전 비교 고르기(명령 팔레트·보기 메뉴의 "두 리비전 비교…"): 기준과 대상(비우면 작업트리)을 브랜치·태그
// 이름이나 아무 리비전(sha, `HEAD~2`)으로 고르고, 두 점(`a b`)과 갈라진 지점부터(`a...b`) 가운데 하나로 연다.
// 기준 칸에 `a..b`·`a...b`를 쳐도 된다. 백엔드: plx-core/src/changes.rs ref_names, compare.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { shortSha } from "../blame/format";
import type { CommitInfo } from "../blame/types";
import { call } from "../ipc";
import { isComposing } from "../keys";
import type { Compare } from "../nav";
import { filterByLabel } from "../palette/match";
import { highlightRuns } from "../quickopen/highlight";
import { Runs } from "../quickopen/QuickOpen";
import { compareOf, defaultBase } from "./range";

/** plx-core/src/changes.rs RefName */
type RefName = { name: string; kind: "branch" | "remote" | "tag"; current: boolean };
/** 권하는 이름(`label`로 맞춘다) */
type Suggestion = RefName & { label: string };

const KIND_NAME: Record<RefName["kind"], string> = { branch: "브랜치", remote: "원격", tag: "태그" };
/** 권하는 이름을 이만큼만 그린다 */
const SHOWN = 50;
/** 친 리비전을 풀어 보기 전에 기다리는 시간 */
const RESOLVE_MS = 120;

type Field = "base" | "target";
/** 칸에 친 리비전을 푼 결과 */
type Resolved = { text: string; state: "empty" | "checking" | "ok" | "failed"; info?: CommitInfo; error?: string };

export function ComparePicker({
  repo,
  initial,
  onPick,
  onClose,
}: {
  /** 저장소 안의 아무 경로 */
  repo: string;
  /** 처음 칸에 넣을 값(없으면 본줄기 ↔ 지금 브랜치) */
  initial?: Partial<Compare>;
  onPick(compare: Compare): void;
  onClose(): void;
}) {
  const [names, setNames] = useState<Suggestion[]>([]);
  const [base, setBase] = useState(initial?.base ?? "");
  const [target, setTarget] = useState(initial?.target ?? "");
  const [mergeBase, setMergeBase] = useState(initial?.mergeBase ?? false);
  const [field, setField] = useState<Field>("base");
  /** 권하는 이름 가운데 고른 것(-1이면 고르지 않았다: Enter가 비교를 연다) */
  const [selected, setSelected] = useState(-1);
  const [resolved, setResolved] = useState<Record<Field, Resolved>>({
    base: { text: "", state: "empty" },
    target: { text: "", state: "empty" },
  });
  const baseRef = useRef<HTMLInputElement>(null);
  const targetRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    let disposed = false;
    call<RefName[]>("ref_names", { repo }).then(
      (list) => {
        if (disposed) return;
        setNames(list.map((r) => ({ ...r, label: r.name })));
        // 처음 값이 없으면 본줄기 ↔ 지금 브랜치
        if (initial) return;
        const current = list.find((r) => r.current)?.name ?? null;
        const main = defaultBase(
          list.map((r) => r.name),
          current,
        );
        if (main) {
          setBase((b) => b || main);
          if (current) setTarget((t) => t || current);
        }
      },
      () => {},
    );
    baseRef.current?.focus();
    return () => {
      disposed = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo]);

  // 친 리비전을 풀어 그 커밋을 칸 아래에 보인다(잠깐 멈추면).
  const resolve = (which: Field, text: string) => {
    const rev = text.trim();
    if (rev === "" || rev.includes("..")) {
      setResolved((r) => ({ ...r, [which]: { text, state: "empty" } }));
      return () => {};
    }
    setResolved((r) => ({ ...r, [which]: { text, state: "checking", info: r[which].info } }));
    let disposed = false;
    const timer = window.setTimeout(() => {
      call<CommitInfo>("commit_info", { repo, rev }).then(
        (info) => !disposed && setResolved((r) => ({ ...r, [which]: { text, state: "ok", info } })),
        (e: unknown) => !disposed && setResolved((r) => ({ ...r, [which]: { text, state: "failed", error: String(e) } })),
      );
    }, RESOLVE_MS);
    return () => {
      disposed = true;
      window.clearTimeout(timer);
    };
  };
  useEffect(() => resolve("base", base), [base, repo]);
  useEffect(() => resolve("target", target), [target, repo]);

  const query = field === "base" ? base : target;
  const matched = useMemo(() => filterByLabel(query.trim(), names).slice(0, SHOWN), [query, names]);
  useEffect(() => setSelected(-1), [query, field]);
  useLayoutEffect(() => {
    if (selected >= 0) listRef.current?.children[selected]?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  const range = target.trim() === "" && base.includes("..");
  const compare = compareOf(base, target, mergeBase);
  const failed = (["base", "target"] as const).find((which) => resolved[which].state === "failed");

  const fill = (name: string) => {
    if (field === "base") {
      setBase(name);
      targetRef.current?.focus();
    } else {
      setTarget(name);
    }
  };
  const submit = () => {
    if (failed) return;
    onPick(compare);
  };
  const swap = () => {
    if (target.trim() === "") return;
    setBase(target);
    setTarget(base);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    // 아래 화면의 키(j/k, q …)로 가지 않게 여기서 멈춘다.
    e.stopPropagation();
    if (isComposing(e.nativeEvent)) return;
    let handled = true;
    if (e.key === "ArrowDown" || (e.ctrlKey && (e.key === "n" || e.key === "j"))) {
      setSelected((i) => Math.min(matched.length - 1, i + 1));
    } else if (e.key === "ArrowUp" || (e.ctrlKey && (e.key === "p" || e.key === "k"))) {
      setSelected((i) => Math.max(-1, i - 1));
    } else if (e.key === "Enter") {
      const pick = matched[selected];
      if (pick) fill(pick.item.name);
      else submit();
    } else if (e.key === "Escape") {
      if (selected >= 0) setSelected(-1);
      else onClose();
    } else {
      handled = false;
    }
    if (handled) e.preventDefault();
  };

  const side = (which: Field) => {
    const r = resolved[which];
    if (which === "target" && target.trim() === "") return <span className="muted">작업트리 (커밋하지 않은 것까지)</span>;
    if (which === "base" && range) {
      return (
        <span className="muted">
          범위: {compare.base}
          {compare.mergeBase ? "..." : ".."}
          {compare.target}
        </span>
      );
    }
    if (r.state === "empty") return <span className="muted">HEAD</span>;
    if (r.state === "failed") return <span className="error-text">{r.error}</span>;
    if (!r.info) return <span className="muted">찾는 중…</span>;
    return (
      <span className={r.state === "checking" ? "muted" : undefined}>
        <span className="mono">{shortSha(r.info.sha)}</span> {r.info.summary}
      </span>
    );
  };

  return (
    <div className="overlay top" role="dialog" aria-label="두 리비전 비교" onClick={onClose}>
      <div className="help picker quick-open compare-picker" onClick={(e) => e.stopPropagation()}>
        <h2>두 리비전 비교</h2>
        <div className="compare-fields">
          <label className="compare-field">
            <span className="compare-label">기준</span>
            <input
              ref={baseRef}
              value={base}
              placeholder="브랜치·태그·sha (비우면 HEAD, a...b도 된다)"
              aria-label="기준 리비전"
              spellCheck={false}
              autoComplete="off"
              onFocus={() => setField("base")}
              onChange={(e) => setBase(e.target.value)}
              onKeyDown={onKeyDown}
            />
            <span className="compare-resolved">{side("base")}</span>
          </label>
          <button
            type="button"
            className="icon compare-swap"
            title="기준과 대상 바꾸기"
            aria-label="기준과 대상 바꾸기"
            disabled={target.trim() === ""}
            onClick={swap}
          >
            ⇅
          </button>
          <label className="compare-field">
            <span className="compare-label">대상</span>
            <input
              ref={targetRef}
              value={target}
              placeholder="브랜치·태그·sha (비우면 작업트리)"
              aria-label="대상 리비전"
              spellCheck={false}
              autoComplete="off"
              disabled={range}
              onFocus={() => setField("target")}
              onChange={(e) => setTarget(e.target.value)}
              onKeyDown={onKeyDown}
            />
            <span className="compare-resolved">{side("target")}</span>
          </label>
        </div>
        <div className="compare-options">
          <span className="segmented" role="group" aria-label="비교 방법">
            <button
              type="button"
              className={!mergeBase ? "toggle primary" : "toggle"}
              aria-pressed={!mergeBase}
              disabled={range}
              title="기준 커밋과 대상을 그대로 견준다 (git diff a b)"
              onClick={() => setMergeBase(false)}
            >
              두 점 a ↔ b
            </button>
            <button
              type="button"
              className={mergeBase ? "toggle primary" : "toggle"}
              aria-pressed={mergeBase}
              disabled={range}
              title="두 쪽이 갈라진 지점과 대상을 견준다: 대상 쪽에서 바꾼 것만 (git diff a...b)"
              onClick={() => setMergeBase(true)}
            >
              갈라진 지점부터 a...b
            </button>
          </span>
          <button type="button" className="primary" disabled={!!failed} onClick={submit}>
            비교
          </button>
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
              onClick={() => fill(item.name)}
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
        {names.length > 0 && matched.length === 0 && <p className="muted">맞는 이름이 없다. 친 리비전 그대로 견준다.</p>}
        <p className="muted">↑↓ 이름 고르기 · Enter 넣기(고르지 않았으면 비교) · Tab 다음 칸 · Esc 닫기</p>
      </div>
    </div>
  );
}
