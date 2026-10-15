// 커밋 그래프: 시작점(HEAD, 로컬 브랜치와 그 upstream)에서 닿는 커밋을 `git log --topo-order` 순으로 보인다.
// 행은 화면이 보는 범위만 묶음으로 백엔드에서 읽고(가상 스크롤), 레인은 캔버스 하나에 그린다.
// 키: j/k 이동, Enter 커밋 상세, y sha 복사, R 원격 브랜치 모두 ↔ 브랜치만, q 뒤로, ? 도움말.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Help } from "../Help";
import { copyText } from "../clipboard";
import { formatCount, formatDate, relativeTime, shortSha } from "../blame/format";
import { Acceleration } from "../history/Acceleration";
import { scrollToRow, visibleRange } from "../history/list";
import { lastName, type Navigation } from "../nav";
import { afterPaint, nowMs, record } from "../perf";
import { drawRows, graphWidth, readPalette, ROW_HEIGHT, type Palette, type Visible } from "./draw";
import { CHUNK, RowCache, rowWidth } from "./rows";
import { openGraph, type GraphHandle } from "./session";
import type { GraphMeta, GraphRow, GraphScope, GraphStats, Label } from "./types";

type Props = {
  /** 저장소 안의 아무 경로(작업트리 또는 plx 호출의 cwd) */
  repo: string;
  /** 처음 고를 커밋(리비전). 없으면 첫 행. */
  focus?: string | null;
  /** 탐색 스택 맨 위(보이는 화면)인가 */
  active: boolean;
  depth: number;
  /** 콜드 스타트의 첫 요청. 창이 보일 때까지 페인트가 없어 예산과 비교하지 않는다. */
  cold?: boolean;
  /** 첫 화면(첫 행들 또는 오류)을 DOM에 넣었다. */
  onShown?(committedMs: number): void;
  onBack(): void;
  onNavigate(navigation: Navigation): void;
};

const SCOPES: { scope: GraphScope; name: string; title: string }[] = [
  { scope: "head", name: "HEAD", title: "HEAD에서 닿는 커밋만" },
  {
    scope: "branches",
    name: "브랜치",
    title: "HEAD, 로컬 브랜치, 로컬 브랜치의 upstream",
  },
  { scope: "all", name: "원격 포함", title: "원격 브랜치까지 모두 (R)" },
];
const SCOPE_KEY = "plx.graph.scope";

function readScope(): GraphScope {
  try {
    const value = localStorage.getItem(SCOPE_KEY);
    return value === "head" || value === "all" ? value : "branches";
  } catch {
    return "branches";
  }
}

function writeScope(scope: GraphScope) {
  try {
    localStorage.setItem(SCOPE_KEY, scope);
  } catch {
    // 저장하지 못해도 이번 실행에는 반영된다.
  }
}

/** 범위를 바꾸면 그래프를 새로 연다(고른 커밋은 새 그래프에서 다시 찾는다). */
export function GraphView(props: Props) {
  const [scope, setScope] = useState<GraphScope>(readScope);
  const [focus, setFocus] = useState(props.focus ?? null);
  const first = useRef(scope);
  return (
    <GraphPane
      key={scope}
      {...props}
      // 첫 화면 측정은 처음 연 그래프만 한다.
      onShown={scope === first.current ? props.onShown : undefined}
      focus={focus}
      scope={scope}
      onScope={(next, selectedSha) => {
        if (next === scope) return;
        setFocus(selectedSha);
        setScope(next);
        writeScope(next);
      }}
    />
  );
}

type PaneProps = Props & {
  scope: GraphScope;
  onScope(scope: GraphScope, selectedSha: string | null): void;
};

type Progress =
  | { kind: "loading"; commits: number }
  | { kind: "layout" }
  | { kind: "done"; stats: GraphStats }
  | { kind: "failed"; message: string };

const TOAST_MS = 1800;
/** 성능 예산(docs/MILESTONES.md): 참조 저장소 전체 이력에서 첫 화면 ≤ 150ms */
const FIRST_SCREEN_BUDGET_MS = 150;
/** 보이는 행 앞뒤로 미리 읽는 행 수 */
const OVERSCAN = 60;
/** 한 행에 보이는 이름표 수(나머지는 +N) */
const MAX_LABELS = 3;

function GraphPane({
  repo,
  focus = null,
  scope,
  onScope,
  active,
  depth,
  cold = false,
  onShown,
  onBack,
  onNavigate,
}: PaneProps) {
  const [meta, setMeta] = useState<GraphMeta | null>(null);
  /** 전체 행 수. 첫 화면이 오기 전에는 null. */
  const [total, setTotal] = useState<number | null>(null);
  /** 레이아웃을 정한 행 수(그만큼 읽을 수 있다) */
  const [available, setAvailable] = useState(0);
  /**
   * 지금까지 화면에 보인 가장 넓은 행의 레인 수. 그래프 칸은 이만큼만 넓히고 좁히지 않는다(스크롤할 때 글자가
   * 좌우로 흔들리지 않게). 이력 전체의 최대 너비를 쓰면 git/git은 처음부터 칸이 가장 넓어진다.
   */
  const [lanes, setLanes] = useState(1);
  const [progress, setProgress] = useState<Progress>({
    kind: "loading",
    commits: 0,
  });
  const [selected, setSelected] = useState(0);
  const [viewport, setViewport] = useState({ top: 0, height: 0 });
  /** 행 묶음이 도착할 때마다 올려 다시 그린다. */
  const [, setVersion] = useState(0);
  const [palette, setPalette] = useState<Palette | null>(null);
  const [help, setHelp] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const cache = useRef(new RowCache());
  const handle = useRef<GraphHandle | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  /** 숨겼다 다시 보일 때 되돌릴 스크롤 위치 */
  const savedTop = useRef(0);
  const toastTimer = useRef(0);
  const nowAtOpen = useRef(Date.now());
  const timing = useRef({
    t0: nowMs(),
    metaMs: NaN,
    pageMs: NaN,
    shown: false,
    painted: null as Promise<number | null> | null,
    reported: false,
  });
  const latest = useRef({ active, meta, selected, help, total });
  latest.current = { active, meta, selected, help, total };

  const showToast = (message: string) => {
    setToast(message);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), TOAST_MS);
  };

  // 그래프를 연다. 화면을 떠나면(끝난 뒤에도) 닫아 백엔드의 레이아웃을 버린다.
  useEffect(() => {
    const graph = openGraph(repo, scope, (event) => {
      switch (event.kind) {
        case "meta": {
          const { kind: _kind, ...m } = event;
          timing.current.metaMs = nowMs();
          setMeta(m);
          break;
        }
        case "loading":
          setProgress({ kind: "loading", commits: event.commits });
          break;
        case "page":
          timing.current.pageMs = nowMs();
          cache.current.seed(event.rows);
          setAvailable(event.rows.length);
          setTotal(event.total);
          setProgress((p) => (p.kind === "done" ? p : { kind: "layout" }));
          break;
        case "progress":
          setAvailable(event.available);
          break;
        case "done":
          setAvailable(event.stats.rows);
          setProgress({ kind: "done", stats: event.stats });
          break;
        case "error":
          setProgress({ kind: "failed", message: event.message });
          break;
        case "cancelled":
          break;
      }
    });
    handle.current = graph;
    return () => {
      graph.close();
      window.clearTimeout(toastTimer.current);
    };
    // 범위마다 새 컴포넌트다(GraphView가 key를 준다).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 첫 화면을 그린 시각
  useLayoutEffect(() => {
    const t = timing.current;
    if (t.shown || (total === null && progress.kind !== "failed")) return;
    t.shown = true;
    onShown?.(nowMs());
    t.painted = afterPaint();
  }, [total, progress]);

  // 레이아웃이 끝나면 성능 HUD에 남긴다: 첫 화면을 그린 때까지, 예산 150ms.
  useEffect(() => {
    const t = timing.current;
    if (progress.kind !== "done" || t.reported || !t.painted) return;
    t.reported = true;
    const { stats } = progress;
    void t.painted.then((painted) => {
      const end = painted ?? nowMs();
      const phases = [
        { label: "참조", ms: t.metaMs - t.t0 },
        { label: "첫 화면 보냄", ms: t.pageMs - t.t0 },
        { label: "표시", ms: end - t.t0 },
        { label: "레이아웃 끝", ms: stats.totalMs },
      ].filter((p) => Number.isFinite(p.ms));
      const sources = [
        stats.fromGraph > 0 && `commit-graph ${formatCount(stats.fromGraph)}`,
        stats.fromTable > 0 && `커밋 표 ${formatCount(stats.fromTable)}`,
        stats.fromObjects > 0 && `객체 ${formatCount(stats.fromObjects)}`,
      ].filter(Boolean);
      record({
        kind: "graph",
        label: `그래프 ${lastName(latest.current.meta?.repo ?? repo)}${cold ? " (콜드 스타트)" : ""}`,
        totalMs: end - t.t0,
        budgetMs: cold ? undefined : FIRST_SCREEN_BUDGET_MS,
        phases,
        detail: `커밋 ${stats.rows} · 레인 ${stats.width} · DAG ${stats.loadMs.toFixed(1)}ms (${sources.join(", ") || "없음"})`,
        ok: painted !== null,
      });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [progress]);

  // 처음 고를 커밋: 준 커밋, 없으면 HEAD(맨 위가 아닐 수 있다: 다른 브랜치가 더 최근이거나 detached HEAD)
  const pageArrived = total !== null;
  useEffect(() => {
    if (!pageArrived) return;
    handle.current?.find(focus ?? "HEAD").then(
      (row) => {
        if (row !== null) select(row, true);
        else if (focus) showToast(`이 그래프에 없는 커밋이다: ${shortSha(focus)}`);
      },
      (e: unknown) => focus && showToast(String(e)),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageArrived]);

  // 보이는 범위(와 앞뒤 여유)의 행을 읽는다.
  useEffect(() => {
    if (total === null) return;
    const { start, end } = visibleRange(viewport.top, viewport.height, ROW_HEIGHT, total, OVERSCAN);
    for (const index of cache.current.take(start, end, available)) {
      handle.current?.rows(index * CHUNK, (index + 1) * CHUNK).then(
        (window) => {
          cache.current.put(window);
          setVersion((v) => v + 1);
        },
        () => cache.current.failed(index),
      );
    }
  }, [viewport, available, total]);

  // 목록 크기를 따라간다.
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const update = () => setViewport({ top: list.scrollTop, height: list.clientHeight });
    update();
    const observer = new ResizeObserver(update);
    observer.observe(list);
    return () => observer.disconnect();
  }, []);

  // 레인 색(테마가 바뀌면 다시 읽는다)
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const update = () => setPalette(readPalette(list));
    update();
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    const list = listRef.current;
    if (active && list) {
      if (list.scrollTop !== savedTop.current) list.scrollTop = savedTop.current;
      setViewport({ top: list.scrollTop, height: list.clientHeight });
      list.focus({ preventScroll: true });
    } else {
      setHelp(false);
    }
  }, [active]);

  /** `index`를 고르고 보이게 스크롤한다(`center`면 화면 밖일 때 가운데로). */
  const select = (index: number, center = false) => {
    const n = latest.current.total ?? 0;
    if (n === 0) return;
    const next = Math.max(0, Math.min(n - 1, index));
    setSelected(next);
    const list = listRef.current;
    if (!list) return;
    const nearest = scrollToRow(next, list.scrollTop, list.clientHeight, ROW_HEIGHT);
    const top =
      center && nearest !== list.scrollTop
        ? Math.max(0, next * ROW_HEIGHT - (list.clientHeight - ROW_HEIGHT) / 2)
        : nearest;
    if (top !== list.scrollTop) list.scrollTop = top;
  };

  const pageRows = () => Math.max(1, Math.floor((listRef.current?.clientHeight ?? ROW_HEIGHT) / ROW_HEIGHT) - 1);

  const current = (index = latest.current.selected) => cache.current.get(index);

  const openCommit = (index?: number) => {
    const m = latest.current.meta;
    const row = current(index);
    if (!m || !row) return;
    onNavigate({ kind: "commit", repo: m.repo, rev: row.sha, path: null });
  };

  const copySha = () => {
    const row = current();
    if (!row) return;
    copyText(row.sha).then(
      () => showToast(`복사했다: ${shortSha(row.sha)}`),
      (e: unknown) => showToast(String(e)),
    );
  };

  const changeScope = (next: GraphScope) => onScope(next, current()?.sha ?? null);

  // 키: 화면이 맨 위일 때만.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!latest.current.active || e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, textarea")) return;
      // 머리줄 버튼에 초점이 있으면 Enter·Space는 버튼이 받는다.
      if (target?.closest("button") && (e.key === "Enter" || e.key === " ")) return;
      const { selected: at, help: helpOpen } = latest.current;
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
        case " ":
          select(at + pageRows());
          break;
        case "PageUp":
          select(at - pageRows());
          break;
        case "Home":
          select(0);
          break;
        case "End":
          select((latest.current.total ?? 1) - 1);
          break;
        case "Enter":
          openCommit();
          break;
        case "y":
          copySha();
          break;
        case "R":
          changeScope(scope === "all" ? "branches" : "all");
          break;
        case "q":
          onBack();
          break;
        case "?":
          setHelp(true);
          break;
        case "Escape":
          handled = helpOpen;
          setHelp(false);
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

  const count = total ?? 0;
  const { start, end } = visibleRange(viewport.top, viewport.height, ROW_HEIGHT, count);
  const visible: Visible[] = [];
  let visibleLanes = 1;
  for (let i = start; i < end; i++) {
    const row = cache.current.get(i);
    if (row) {
      visible.push({ index: i, row });
      visibleLanes = Math.max(visibleLanes, rowWidth(row));
    }
  }
  const width = graphWidth(Math.max(lanes, visibleLanes));
  useLayoutEffect(() => {
    if (visibleLanes > lanes) setLanes(visibleLanes);
  });

  // 레인은 보이는 행만 캔버스에 그린다(스크롤·행 도착·테마 변경마다).
  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !palette) return;
    const dpr = window.devicePixelRatio || 1;
    const height = viewport.height;
    const w = Math.round(width * dpr);
    const h = Math.round(height * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    drawRows(ctx, visible, viewport.top, width, palette, meta?.head ?? null);
  });

  const chosen = current();
  const hidden =
    meta && scope === "branches" && meta.hiddenRemotes > 0
      ? `원격 브랜치 ${formatCount(meta.hiddenRemotes)}개 숨김`
      : meta && scope === "head" && meta.hiddenBranches > 0
        ? `브랜치 ${formatCount(meta.hiddenBranches)}개 숨김`
        : null;

  return (
    <section className="graph">
      <header className="blame-header">
        <span className="badge">그래프</span>
        <span className="file-path mono" title={meta?.repo ?? repo}>
          <span className="file-name">{lastName(meta?.repo ?? repo)}</span>
        </span>
        {meta?.branch ? (
          <span className="badge mono" title="HEAD가 가리키는 브랜치">
            {meta.branch}
          </span>
        ) : (
          meta?.head && (
            <span className="badge warn mono" title="브랜치가 아닌 커밋을 체크아웃했다">
              HEAD {shortSha(meta.head)}
            </span>
          )
        )}
        <span className="segmented" role="group" aria-label="보일 브랜치">
          {SCOPES.map((s) => (
            <button
              key={s.scope}
              type="button"
              className={s.scope === scope ? "toggle primary" : "toggle"}
              aria-pressed={s.scope === scope}
              title={s.title}
              onClick={() => changeScope(s.scope)}
            >
              {s.name}
            </button>
          ))}
        </span>
        {hidden && (
          <span className="badge" title={scope === "branches" ? "R로 원격 브랜치까지 모두 보인다" : undefined}>
            {hidden}
          </span>
        )}
        {depth > 1 && (
          <span className="badge" title="q로 앞 화면으로 돌아간다">
            뒤로 q · {depth - 1}
          </span>
        )}
        {meta && <Acceleration repo={meta.repo} onDone={showToast} />}
        <span className="blame-status">
          {progress.kind === "loading" &&
            (progress.commits > 0 ? `커밋 ${formatCount(progress.commits)}개 읽는 중…` : "여는 중…")}
          {(progress.kind === "layout" || progress.kind === "done") && `커밋 ${formatCount(count)}개`}
          {progress.kind === "failed" && <span className="error-text">실패</span>}
        </span>
        <button type="button" className="icon" onClick={() => setHelp(true)} aria-label="단축키 (?)" title="단축키 (?)">
          ?
        </button>
      </header>
      <div className="graph-body">
        <div
          className="graph-list"
          ref={listRef}
          tabIndex={0}
          role="listbox"
          aria-label="커밋 그래프"
          aria-activedescendant={chosen ? `graph-row-${selected}` : undefined}
          onScroll={(e) => {
            const list = e.currentTarget;
            if (latest.current.active) savedTop.current = list.scrollTop;
            setViewport({ top: list.scrollTop, height: list.clientHeight });
          }}
        >
          <div className="graph-rows" style={{ height: count * ROW_HEIGHT }}>
            {Array.from({ length: end - start }, (_, offset) => {
              const i = start + offset;
              const row = cache.current.get(i);
              return (
                <div
                  key={i}
                  id={`graph-row-${i}`}
                  role="option"
                  aria-selected={i === selected}
                  className={i === selected ? "graph-row selected" : "graph-row"}
                  style={{
                    top: i * ROW_HEIGHT,
                    height: ROW_HEIGHT,
                    paddingLeft: width,
                  }}
                  onClick={() => select(i)}
                  onDoubleClick={() => {
                    select(i);
                    openCommit(i);
                  }}
                >
                  {row ? <Row row={row} nowMs={nowAtOpen.current} /> : <span className="muted">…</span>}
                </div>
              );
            })}
          </div>
          {progress.kind !== "failed" && total === 0 && (
            <div className="blame-message">
              <p>커밋이 없다.</p>
            </div>
          )}
          {progress.kind === "failed" && (
            <div className="blame-message error">
              <p>{progress.message}</p>
            </div>
          )}
        </div>
        <canvas className="graph-lanes" ref={canvasRef} aria-hidden="true" />
      </div>
      {chosen && (
        <footer className="history-detail">
          <span className="mono" title="y로 복사">
            {chosen.sha}
          </span>
          <span>
            {chosen.author} <span className="muted">&lt;{chosen.authorMail}&gt;</span>
          </span>
          <span className="muted">{formatDate(chosen.authorTime, chosen.authorTzOffset)}</span>
          {chosen.parents.length > 1 && <span className="badge">merge · 부모 {chosen.parents.length}</span>}
          {chosen.parents.length === 0 && <span className="badge">루트</span>}
          {chosen.labels.length > MAX_LABELS && (
            <span className="muted">{chosen.labels.map((l) => l.name).join(", ")}</span>
          )}
        </footer>
      )}
      {help && <Help view="graph" onClose={() => setHelp(false)} />}
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}

function Row({ row, nowMs }: { row: GraphRow; nowMs: number }) {
  return (
    <>
      <Labels labels={row.labels} />
      <span className="history-summary">{row.summary}</span>
      <span className="history-author" title={row.authorMail}>
        {row.author}
      </span>
      <span className="history-time" title={formatDate(row.authorTime, row.authorTzOffset)}>
        {relativeTime(row.authorTime, nowMs)}
      </span>
      <span className="history-sha mono">{shortSha(row.sha)}</span>
    </>
  );
}

const LABEL_TITLE: Record<Label["kind"], string> = {
  head: "HEAD (브랜치가 아니다)",
  branch: "브랜치",
  remote: "원격 브랜치",
  tag: "태그",
};

function Labels({ labels }: { labels: Label[] }) {
  if (labels.length === 0) return null;
  const shown = labels.slice(0, MAX_LABELS);
  const rest = labels.slice(MAX_LABELS);
  return (
    <>
      {shown.map((label) => (
        <span
          key={`${label.kind}:${label.name}`}
          className={`label label-${label.kind}${label.current ? " current" : ""}`}
          title={`${LABEL_TITLE[label.kind]}${label.current ? " · HEAD" : ""}: ${label.name}`}
        >
          {label.name}
        </span>
      ))}
      {rest.length > 0 && (
        <span className="label label-more" title={rest.map((l) => l.name).join("\n")}>
          +{rest.length}
        </span>
      )}
    </>
  );
}
