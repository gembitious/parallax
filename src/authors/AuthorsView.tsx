// 작성자 보기(M18): 저장소(또는 파일·폴더)를 바꾼 사람마다 커밋 수, 몫, 처음·마지막 커밋을 보인다. 많이 쓴 사람부터
// (`t`로 최근에 쓴 사람부터). 사람을 고르면 그 사람이 쓴 커밋을 그래프에서 찾는다(그래프 검색의 `author:`). 읽기만 한다.
// 키: j/k 이동, Enter·G 그 사람의 커밋을 그래프에서, y `이름 <메일>` 복사, s HEAD ↔ 모든 브랜치, t 커밋 수 ↔ 최근,
//     / 거르기, q 뒤로, ? 도움말.
// 백엔드: plx-core/src/authors.rs, src-tauri/src/git.rs authors_list. HEAD·참조가 바뀌면 다시 센다(같은 시작점은 캐시).
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { formatCount, relativeTime } from "../blame/format";
import { copyText } from "../clipboard";
import { authorQuery } from "../graph/search";
import { Help } from "../Help";
import { toggleHud } from "../Hud";
import { call } from "../ipc";
import { isComposing, routeKey } from "../keys";
import { copyItem, openContextMenu, selectedText, type MenuEntry } from "../menu/Menu";
import { usePublish, type ViewSlot } from "../menu/registry";
import { lastName, type AuthorScope, type Navigation } from "../nav";
import { commonCommands, usePalette, type Command } from "../palette/Palette";
import { getPerfInfo, nowMs, record } from "../perf";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { drawWindow, insideDrawn, type DrawnRange } from "../scrollWindow";
import { toast } from "../Toasts";
import { refsMoved, watchRepo } from "../watch";
import {
  ident,
  keepAuthor,
  localDay,
  matchesQuery,
  ORDER_LABEL,
  ordered,
  otherOrder,
  otherScope,
  SCOPE_LABEL,
  sharePercent,
  type Author,
  type AuthorOrder,
  type Authors,
} from "./types";

type Props = {
  /** 저장소 안의 아무 경로 */
  repo: string;
  /** 센 파일·폴더(저장소 기준). 없으면 저장소 전체 */
  path: string | null;
  visible: boolean;
  active: boolean;
  slot: ViewSlot | null;
  depth: number;
  onShown?(committedMs: number): void;
  onBack(): void;
  onNavigate(navigation: Navigation): void;
};

type Load = { kind: "loading" } | { kind: "done"; list: Authors } | { kind: "failed"; message: string };

const ROW_HEIGHT = 24;
const SCOPES: AuthorScope[] = ["head", "all"];
const ORDERS: AuthorOrder[] = ["commits", "recent"];

export function AuthorsView({ repo, path, visible, active, slot, depth, onShown, onBack, onNavigate }: Props) {
  const [scope, setScope] = useState<AuthorScope>("head");
  const [order, setOrder] = useState<AuthorOrder>("commits");
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [selected, setSelected] = useState(0);
  const [query, setQuery] = useState("");
  const [filtering, setFiltering] = useState(false);
  const [help, setHelp] = useState(false);
  const [viewport, setViewport] = useState({ top: 0, height: 0 });
  /** 숨어 있는 동안 저장소가 바뀌었다(보이면 다시 센다) */
  const [stale, setStale] = useState(false);
  const [version, setVersion] = useState(0);
  const drawnRange = useRef<DrawnRange>({ start: 0, end: 0, height: 0 });
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const shown = useRef(false);
  const savedTop = useRef(0);
  const nowAt = useRef(Date.now());

  const list = load.kind === "done" ? load.list : null;
  const sorted = useMemo(() => (list ? ordered(list.authors, order) : []), [list, order]);
  const rows = useMemo(() => sorted.filter((author) => matchesQuery(author, query)), [sorted, query]);
  /** 막대의 길이 기준(가장 많이 쓴 사람) */
  const most = useMemo(() => (list ? Math.max(1, ...list.authors.map((a) => a.commits)) : 1), [list]);

  // 세기: 범위를 바꾸거나 저장소가 바뀌면 다시 센다. 고른 사람은 이름·메일로 이어 간다.
  useEffect(() => {
    let stale = false;
    const started = nowMs();
    const before = latest.current.rows[latest.current.selected];
    const index = latest.current.selected;
    setLoad((l) => (l.kind === "done" && l.list.scope === scope ? l : { kind: "loading" }));
    call<Authors>("authors_list", { repo, scope, path }).then(
      (next) => {
        if (stale) return;
        nowAt.current = Date.now();
        setLoad({ kind: "done", list: next });
        const { order: o, query: q } = latest.current;
        setSelected(keepAuthor(before, ordered(next.authors, o).filter((a) => matchesQuery(a, q)), index));
        record({
          kind: "authors",
          label: `작성자 ${path ?? lastName(repo)}${next.cached ? " (캐시)" : ""}`,
          totalMs: nowMs() - started,
          phases: [],
          ok: true,
        });
      },
      (e: unknown) => !stale && setLoad({ kind: "failed", message: String(e) }),
    );
    return () => {
      stale = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo, path, scope, version]);

  // HEAD·참조가 바뀌면(커밋, 체크아웃, fetch) 다시 센다. 숨어 있으면 보일 때.
  useEffect(
    () =>
      watchRepo(repo, (change) => {
        if (!refsMoved(change)) return;
        if (latest.current.visible) setVersion((v) => v + 1);
        else setStale(true);
      }),
    [repo],
  );
  useEffect(() => {
    if (visible && stale) {
      setStale(false);
      setVersion((v) => v + 1);
    }
  }, [visible, stale]);

  useEffect(() => {
    if (!shown.current && load.kind !== "loading") {
      shown.current = true;
      onShown?.(nowMs());
    }
  });

  useEffect(() => {
    if (active) (filtering ? inputRef.current : listRef.current)?.focus({ preventScroll: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  useLayoutEffect(() => {
    if (visible && listRef.current) listRef.current.scrollTop = savedTop.current;
  }, [visible]);

  const current = (index = latest.current.selected): Author | null => latest.current.rows[index] ?? null;
  const repoOf = () => (latest.current.load.kind === "done" ? latest.current.load.list.repo : repo);

  const select = (index: number) => {
    const count = latest.current.rows.length;
    if (count === 0) return;
    const next = Math.max(0, Math.min(count - 1, index));
    setSelected(next);
    const el = listRef.current;
    if (!el) return;
    const top = next * ROW_HEIGHT;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + ROW_HEIGHT > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW_HEIGHT - el.clientHeight;
  };

  /** 그 사람이 쓴 커밋(이 경로를 바꾼 것만)을 그래프에서 찾는다 */
  const openGraph = (index?: number) => {
    const author = current(index);
    if (author) onNavigate({ kind: "graph", repo: repoOf(), rev: null, search: authorQuery(author.name, author.email, path) });
  };
  const copy = (text: string) =>
    copyText(text).then(
      () => toast(`복사했다: ${text}`),
      (e: unknown) => toast(String(e)),
    );
  const copyIdent = (index?: number) => {
    const author = current(index);
    if (author) void copy(ident(author));
  };
  const copyEmail = (index?: number) => {
    const author = current(index);
    if (author) void copy(author.email);
  };
  const resetScroll = () => {
    setSelected(0);
    savedTop.current = 0;
    if (listRef.current) listRef.current.scrollTop = 0;
  };
  const switchScope = (next?: AuthorScope) => {
    const to = next ?? otherScope(latest.current.scope);
    if (to === latest.current.scope) return;
    setScope(to);
    resetScroll();
  };
  const switchOrder = (next?: AuthorOrder) => {
    const to = next ?? otherOrder(latest.current.order);
    if (to === latest.current.order) return;
    setOrder(to);
    resetScroll();
  };
  const openHistory = () => onNavigate({ kind: "history", target: { repo: repoOf(), path: path ?? ".", rev: null } });
  const openFilter = () => {
    setFiltering(true);
    window.setTimeout(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
  };
  const closeFilter = (clear: boolean) => {
    if (clear) {
      const author = current();
      setQuery("");
      if (author) setSelected(Math.max(0, latest.current.sorted.indexOf(author)));
    }
    setFiltering(false);
    listRef.current?.focus({ preventScroll: true });
  };

  const quickOpen = useQuickOpen({
    repo,
    current: null,
    onNavigate,
    onClosed: () => listRef.current?.focus({ preventScroll: true }),
  });

  const commands: Command[] = [
    { id: "graph", label: "이 사람의 커밋을 그래프에서", keys: "Enter · G", detail: "author: 검색", run: () => openGraph() },
    { label: "이름 <메일> 복사", keys: "y", run: () => copyIdent() },
    { label: "메일 복사", run: () => copyEmail() },
    { label: scope === "head" ? "모든 브랜치에서 세기" : "HEAD에서 세기", keys: "s", run: () => switchScope() },
    { label: order === "commits" ? "최근에 쓴 사람부터" : "커밋이 많은 사람부터", keys: "t", run: () => switchOrder() },
    { label: path ? "이 경로의 히스토리" : "폴더 히스토리", detail: path ?? "저장소 전체", run: openHistory },
    { label: "거르기", keys: "/", run: openFilter },
    ...commonCommands({
      files: quickOpen.open,
      worktrees: () => {},
      help: () => setHelp(true),
      back: onBack,
      hud: getPerfInfo().enabled ? toggleHud : undefined,
    }).filter((command) => command.keys !== "W"),
  ];
  const palette = usePalette(commands, () => listRef.current?.focus({ preventScroll: true }));
  usePublish(slot, visible, "작성자", commands, {
    palette: palette.open,
    help: () => setHelp(true),
    files: quickOpen.open,
  });

  const latest = useRef({ rows, sorted, selected, load, scope, order, query, visible, active, help, palette: palette.open, openFiles: quickOpen.open });
  latest.current = { rows, sorted, selected, load, scope, order, query, visible, active, help, palette: palette.open, openFiles: quickOpen.open };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { active: on, openFiles, palette: openPalette, help: helpOpen, rows: all, selected: at } = latest.current;
      if (!on) return;
      if (routeKey(e, { files: openFiles, palette: openPalette }) !== "view") return;
      if (isComposing(e)) return;
      const page = Math.max(1, Math.floor((listRef.current?.clientHeight ?? ROW_HEIGHT) / ROW_HEIGHT) - 1);
      let handled = true;
      switch (e.key) {
        case "j":
        case "ArrowDown":
          select(at + 1);
          break;
        case "k":
        case "ArrowUp":
          select(at - 1);
          break;
        case "PageDown":
          select(at + page);
          break;
        case "PageUp":
          select(at - page);
          break;
        case "Home":
          select(0);
          break;
        case "End":
          select(all.length - 1);
          break;
        case "Enter":
        case "G":
          openGraph();
          break;
        case "y":
          copyIdent();
          break;
        case "s":
          switchScope();
          break;
        case "t":
          switchOrder();
          break;
        case "/":
          openFilter();
          break;
        case "q":
          onBack();
          break;
        case "?":
          setHelp(true);
          break;
        case "Escape":
          if (helpOpen) setHelp(false);
          else if (latest.current.query) closeFilter(true);
          else handled = false;
          break;
        default:
          handled = false;
      }
      if (handled) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const rowMenu = (index: number): MenuEntry[] => {
    const author = current(index);
    if (!author) return [];
    return [
      { header: ident(author) },
      { label: "이 사람의 커밋을 그래프에서", keys: "Enter", detail: path ? `${path}를 바꾼 것` : undefined, run: () => openGraph(index) },
      "separator",
      { label: "이름 <메일> 복사", keys: "y", run: () => copyIdent(index) },
      { label: "메일 복사", detail: author.email, run: () => copyEmail(index) },
    ];
  };

  const count = rows.length;
  const { start, end } = drawWindow(viewport.top, viewport.height, ROW_HEIGHT, count);
  useLayoutEffect(() => {
    drawnRange.current = { start, end, height: viewport.height };
  });
  useLayoutEffect(() => {
    const el = listRef.current;
    if (el) setViewport({ top: el.scrollTop, height: el.clientHeight });
  }, [count]);

  const chosen = rows[selected] ?? null;
  const total = list?.commits ?? 0;
  const status =
    load.kind === "loading"
      ? "세는 중…"
      : list
        ? `${query ? `${formatCount(rows.length)} / ` : ""}${formatCount(list.authors.length)}명 · 커밋 ${formatCount(total)}개`
        : null;

  return (
    <section className="reflog-view authors-view">
      <header className="blame-header" onContextMenu={(e) => openContextMenu(e, [copyItem(selectedText(e.target))], "작성자")}>
        <span className="segmented" role="group" aria-label="센 커밋">
          {SCOPES.map((value) => (
            <button
              key={value}
              type="button"
              className={scope === value ? "toggle primary" : "toggle"}
              aria-pressed={scope === value}
              title={value === "head" ? "HEAD에서 닿는 커밋 (s로 바꾼다)" : "모든 브랜치·원격 브랜치·태그에서 닿는 커밋 (s로 바꾼다)"}
              onClick={() => switchScope(value)}
            >
              {SCOPE_LABEL[value]}
            </button>
          ))}
        </span>
        <span className="segmented" role="group" aria-label="차례">
          {ORDERS.map((value) => (
            <button
              key={value}
              type="button"
              className={order === value ? "toggle primary" : "toggle"}
              aria-pressed={order === value}
              title={`${ORDER_LABEL[value]} 차례 (t로 바꾼다)`}
              onClick={() => switchOrder(value)}
            >
              {ORDER_LABEL[value]}
            </button>
          ))}
        </span>
        <span className="file-path mono" title={path ? `${repoOf()} · ${path}` : repoOf()}>
          <span className="file-name">{path ?? lastName(repoOf())}</span>
          {path && <span className="muted"> 을(를) 바꾼 커밋</span>}
        </span>
        {depth > 1 && (
          <span className="badge" title="q로 앞 화면으로 돌아간다">
            뒤로 q · {depth - 1}
          </span>
        )}
        <span className="blame-status" role="status" title="merge 커밋은 세지 않는다 · 이름·메일은 .mailmap을 따른다">
          {status}
        </span>
        <button type="button" className="icon" onClick={() => setHelp(true)} aria-label="단축키 (?)" title="단축키 (?)">
          ?
        </button>
      </header>
      {(filtering || query) && (
        <div className="history-search">
          <input
            ref={inputRef}
            type="search"
            value={query}
            placeholder="거르기: 이름·메일 (Enter 목록으로, Esc 지우기)"
            aria-label="거르기"
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => {
              setQuery(e.target.value);
              setSelected(0);
            }}
            onKeyDown={(e) => {
              if (isComposing(e.nativeEvent)) return;
              e.stopPropagation();
              if (e.key === "Enter" || e.key === "ArrowDown") {
                e.preventDefault();
                closeFilter(false);
              } else if (e.key === "Escape") {
                e.preventDefault();
                closeFilter(true);
              }
            }}
          />
        </div>
      )}
      <div
        className="history-list"
        ref={listRef}
        tabIndex={0}
        role="listbox"
        aria-label="작성자 목록"
        aria-activedescendant={chosen ? `authors-row-${selected}` : undefined}
        onScroll={(e) => {
          const el = e.currentTarget;
          if (visible) savedTop.current = el.scrollTop;
          if (insideDrawn(el.scrollTop, el.clientHeight, ROW_HEIGHT, count, drawnRange.current)) return;
          setViewport({ top: el.scrollTop, height: el.clientHeight });
        }}
      >
        <div className="history-rows" style={{ height: count * ROW_HEIGHT }}>
          {rows.slice(start, end).map((author, offset) => {
            const i = start + offset;
            return (
              <div
                key={`${author.name}\u0000${author.email}`}
                id={`authors-row-${i}`}
                role="option"
                aria-selected={i === selected}
                className={`history-row authors-row${i === selected ? " selected" : ""}`}
                style={{ top: i * ROW_HEIGHT, height: ROW_HEIGHT }}
                onClick={() => select(i)}
                onDoubleClick={() => {
                  select(i);
                  openGraph(i);
                }}
                onContextMenu={(e) => {
                  select(i);
                  openContextMenu(e, rowMenu(i), "작성자");
                }}
              >
                <Row author={author} total={total} most={most} nowMs={nowAt.current} />
              </div>
            );
          })}
        </div>
        {list && list.authors.length === 0 && (
          <div className="blame-message">
            <p className="muted">
              {path ? `${path}을(를) 바꾼 커밋이 없다(merge 빼고).` : "커밋이 없다(merge 빼고)."}
            </p>
          </div>
        )}
        {list && list.authors.length > 0 && rows.length === 0 && (
          <div className="blame-message">
            <p className="muted">맞는 사람이 없다.</p>
          </div>
        )}
        {load.kind === "failed" && (
          <div className="blame-message error">
            <p>{load.message}</p>
          </div>
        )}
      </div>
      {chosen && (
        <footer className="history-detail" onContextMenu={(e) => openContextMenu(e, rowMenu(selected), "작성자")}>
          <span title="y로 복사">{ident(chosen)}</span>
          <span>
            커밋 {formatCount(chosen.commits)}개 <span className="muted">({sharePercent(chosen.commits, total)})</span>
          </span>
          <span className="muted" title="처음 쓴 커밋 – 마지막으로 쓴 커밋">
            {localDay(chosen.first)} – {localDay(chosen.last)}
          </span>
        </footer>
      )}
      {help && <Help view="authors" onClose={() => setHelp(false)} />}
      {quickOpen.overlay}
      {palette.overlay}
    </section>
  );
}

function Row({ author, total, most, nowMs: now }: { author: Author; total: number; most: number; nowMs: number }) {
  return (
    <>
      <span className="authors-name" title={`${author.name} <${author.email}>`}>
        {author.name}
        <span className="muted mono authors-email"> {author.email}</span>
      </span>
      <span className="authors-bar" aria-hidden="true">
        <span style={{ width: `${Math.max(2, (author.commits / most) * 100)}%` }} />
      </span>
      <span className="authors-count mono" title={`전체 커밋의 ${sharePercent(author.commits, total)}`}>
        {formatCount(author.commits)}
      </span>
      <span className="history-time" title={`마지막 커밋 ${localDay(author.last)}`}>
        {relativeTime(author.last, now)}
      </span>
    </>
  );
}
