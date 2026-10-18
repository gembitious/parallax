// Quick Open(`f`, Ctrl·⌘+P): 저장소의 추적 파일을 퍼지 검색해 고르면 그 파일의 blame으로(Shift+Enter면 파일
// 히스토리로) 간다. `파일:줄`이면 그 줄로. 글자마다 백엔드가 찾는다(plx-core/src/files.rs, 점수는 fuzzy.rs).
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { formatCount } from "../blame/format";
import { CANCELLED } from "../graph/session";
import type { Navigation } from "../nav";
import { afterPaint, nowMs, phasesBetween, record } from "../perf";
import { splitPath, type Run } from "./highlight";
import { quickOpenNavigation, type QuickOpenPick } from "./pick";
import type { FileMatch, FileSearch } from "./types";

/** 한 번에 받는 결과 수(화면에는 20여 개가 보인다. 많이 그릴수록 글자마다 페인트가 늘어난다) */
const LIMIT = 50;
/** 글자 하나를 친 뒤 결과가 그려지기까지(docs/MILESTONES.md 성능 예산) */
const BUDGET_MS = 50;
const PAGE = 10;

/**
 * 화면에 Quick Open을 붙인다. `open()`으로 열고 `overlay`를 화면 끝에 그린다. 고르면 `onNavigate`로 가고,
 * 그냥 닫으면 `onClosed`(화면에 포커스를 돌려준다)를 부른다. `close()`는 화면이 숨을 때 닫는다(포커스는 그대로).
 */
export function useQuickOpen({
  repo,
  current,
  onNavigate,
  onClosed,
}: {
  /** 저장소 안의 아무 경로. 아직 모르면 null(열지 않는다) */
  repo: string | null;
  /** 지금 화면의 파일(저장소 기준) */
  current: string | null;
  onNavigate(navigation: Navigation): void;
  onClosed?(): void;
}): { open(): void; close(): void; isOpen: boolean; overlay: ReactNode } {
  const [isOpen, setOpen] = useState(false);
  const overlay =
    isOpen && repo ? (
      <QuickOpen
        repo={repo}
        current={current}
        onClose={() => {
          setOpen(false);
          onClosed?.();
        }}
        onPick={(pick) => {
          setOpen(false);
          onNavigate(quickOpenNavigation(pick));
        }}
      />
    ) : null;
  return { open: () => setOpen(true), close: () => setOpen(false), isOpen, overlay };
}

type Shown = { search: FileSearch; seq: number };
type Measure = { query: string; started: number; received: number; search: FileSearch };

export function QuickOpen({
  repo,
  current,
  onPick,
  onClose,
}: {
  /** 저장소 안의 아무 경로 */
  repo: string;
  /** 지금 화면의 파일(저장소 기준). 빈 검색어 목록의 맨 앞(가장 최근)이면 그다음을 골라 둔다. */
  current: string | null;
  onPick(pick: QuickOpenPick): void;
  onClose(): void;
}) {
  const [query, setQuery] = useState("");
  const [shown, setShown] = useState<Shown | null>(null);
  const [selected, setSelected] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const seq = useRef(0);
  const measure = useRef<Measure | null>(null);
  /** 결과가 오기 전에 Enter를 누르면 그 결과가 오면 고른다(`파일:줄`의 줄이 앞 결과의 것이 되지 않게). */
  const pendingPick = useRef<boolean | null>(null);
  const latest = useRef({ current, onPick });
  latest.current = { current, onPick };

  useEffect(() => {
    inputRef.current?.focus();
    return () => {
      // 닫은 뒤(Esc·바깥 클릭) 온 결과로는 고르지 않는다: 결과 전에 친 Enter가 남아 있으면 닫은 화면이 이동한다.
      seq.current += 1;
      pendingPick.current = null;
    };
  }, []);

  useEffect(() => {
    const id = ++seq.current;
    const started = nowMs();
    invoke<FileSearch>("files_search", { repo, query, limit: LIMIT }).then(
      (search) => {
        if (id !== seq.current) return;
        measure.current = { query, started, received: nowMs(), search };
        const first =
          query.trim() === "" && search.items.length > 1 && search.items[0].path === latest.current.current ? 1 : 0;
        setShown({ search, seq: id });
        setSelected(first);
        setError(null);
        if (pendingPick.current !== null) {
          const history = pendingPick.current;
          pendingPick.current = null;
          const item = search.items[first];
          if (item) latest.current.onPick(pickOf(search, item, history));
        }
      },
      (e: unknown) => {
        if (id !== seq.current || String(e) === CANCELLED) return;
        pendingPick.current = null;
        setError(String(e));
      },
    );
  }, [repo, query]);

  // 결과를 그린 뒤 성능 HUD에 남긴다(글자마다).
  useLayoutEffect(() => {
    const m = measure.current;
    if (!m) return;
    measure.current = null;
    const committed = nowMs();
    afterPaint().then((painted) => {
      const end = painted ?? committed;
      const { search } = m;
      record({
        kind: "files",
        label: `파일 찾기 ${JSON.stringify(m.query)}`,
        budgetMs: BUDGET_MS,
        totalMs: end - m.started,
        phases: phasesBetween([
          ["검색", m.started],
          ["그리기", m.received],
          [painted === null ? "페인트(측정 못 함)" : "페인트", committed],
          ["", end],
        ]),
        detail:
          `${formatCount(search.matched)} / ${formatCount(search.files)}개 · 백엔드 ${search.searchMs.toFixed(1)}ms` +
          (search.loadMs !== null ? ` · 목록 읽기 ${search.loadMs.toFixed(1)}ms` : "") +
          (search.narrowed ? " · 앞 결과 안에서" : ""),
        ok: painted !== null,
      });
    });
  }, [shown]);

  useLayoutEffect(() => {
    listRef.current?.children[selected]?.scrollIntoView({ block: "nearest" });
  }, [selected, shown]);

  const items = shown?.search.items ?? [];
  const pick = (index: number, history: boolean) => {
    if (!shown || shown.seq !== seq.current) {
      // 친 글자의 결과가 아직 없다.
      pendingPick.current = history;
      return;
    }
    const item = shown.search.items[index];
    if (item) onPick(pickOf(shown.search, item, history));
  };
  const move = (delta: number) => setSelected((i) => Math.max(0, Math.min(items.length - 1, i + delta)));

  return (
    <div className="overlay top" role="dialog" aria-label="파일 찾기" onClick={onClose}>
      <div className="help picker quick-open" onClick={(e) => e.stopPropagation()}>
        <div className="quick-open-bar">
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="파일 이름·경로 조각·초성(ㅍㅇ), 파일:줄"
            aria-label="파일 찾기"
            aria-controls="quick-open-list"
            aria-activedescendant={items.length > 0 ? `quick-open-${selected}` : undefined}
            spellCheck={false}
            autoComplete="off"
            onKeyDown={(e) => {
              // 아래 화면의 키(j/k, q …)로 가지 않게 여기서 멈춘다.
              e.stopPropagation();
              // IME 조합 중의 Enter·화살표는 조합을 끝내는 키다.
              if (e.nativeEvent.isComposing || e.keyCode === 229) return;
              const mod = e.ctrlKey || e.metaKey;
              let handled = true;
              if (e.key === "ArrowDown" || (e.ctrlKey && (e.key === "n" || e.key === "j")) || (mod && e.key === "p"))
                move(1);
              else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "k")) move(-1);
              else if (e.key === "PageDown") move(PAGE);
              else if (e.key === "PageUp") move(-PAGE);
              else if (e.key === "Enter") pick(selected, e.shiftKey);
              else if (e.key === "Escape") onClose();
              else if (e.key !== "Tab") handled = false;
              if (handled) e.preventDefault();
            }}
          />
          <span className="search-status" aria-live="polite">
            {shown === null
              ? "읽는 중…"
              : `${formatCount(shown.search.matched)} / ${formatCount(shown.search.files)}` +
                (shown.search.line !== null ? ` · ${shown.search.line}번째 줄` : "")}
          </span>
        </div>
        {error && <p className="error-text">{error}</p>}
        <ul id="quick-open-list" className="picker-list quick-open-list" role="listbox" ref={listRef}>
          {items.map((item, i) => {
            const { dir, name } = splitPath(item.path, item.positions);
            return (
              <li
                key={item.path}
                id={`quick-open-${i}`}
                role="option"
                aria-selected={i === selected}
                className={`quick-open-item${i === selected ? " selected" : ""}${item.missing ? " missing" : ""}`}
                onMouseMove={() => i !== selected && setSelected(i)}
                onClick={(e) => pick(i, e.shiftKey)}
                title={item.missing ? `${item.path} (작업트리에 없다: Enter는 HEAD 버전 blame)` : item.path}
              >
                <span className="quick-open-name">
                  <Runs runs={name} />
                </span>
                {item.missing && <span className="badge warn">지움</span>}
                <span className="quick-open-dir mono">
                  <Runs runs={dir} />
                </span>
              </li>
            );
          })}
        </ul>
        {shown && items.length === 0 && (
          <p className="muted">{shown.search.files === 0 ? "추적하는 파일이 없다." : "맞는 파일이 없다."}</p>
        )}
        <p className="muted">↑↓ 고르기 · Enter blame · Shift+Enter 파일 히스토리 · 파일:줄 · Esc 닫기</p>
      </div>
    </div>
  );
}

function pickOf(search: FileSearch, item: FileMatch, history: boolean): QuickOpenPick {
  return { repo: search.repo, path: item.path, line: history ? null : search.line, history, missing: item.missing };
}

/** 맞은 글자를 칠한 덩어리들(명령 팔레트도 쓴다) */
export function Runs({ runs }: { runs: Run[] }) {
  return (
    <>
      {runs.map((run, i) =>
        run.hit ? (
          <mark key={i} className="quick-open-hit">
            {run.text}
          </mark>
        ) : (
          <span key={i}>{run.text}</span>
        ),
      )}
    </>
  );
}
