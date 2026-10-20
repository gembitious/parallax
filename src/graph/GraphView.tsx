// 커밋 그래프: 시작점(HEAD, 로컬 브랜치와 그 upstream)에서 닿는 커밋을 `git log --topo-order` 순으로 보인다.
// 행은 화면이 보는 범위만 묶음으로 백엔드에서 읽고(가상 스크롤), 레인은 캔버스 하나에 그린다.
// 키: j/k 이동, Enter 커밋 상세, y sha 복사, R 원격 브랜치 모두 ↔ 브랜치만, / 검색(메시지·작성자·sha, path:경로),
//     n/N 다음·이전 결과, q 뒤로, ? 도움말.
import { memo, useEffect, useLayoutEffect, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { Help } from "../Help";
import { copyText } from "../clipboard";
import { formatCount, formatDate, relativeTime, shortSha } from "../blame/format";
import { Acceleration } from "../history/Acceleration";
import { scrollToRow, visibleRange } from "../history/list";
import { lastName, type Navigation } from "../nav";
import { afterPaint, getPerfInfo, longFrameIn, nowMs, record } from "../perf";
import { drawRows, graphWidth, readPalette, ROW_HEIGHT, type Palette, type Visible } from "./draw";
import { reloadFocus, type Focus } from "./focus";
import { CHUNK, RowCache, rowWidth } from "./rows";
import { nextRow, parseQuery, rowMatches, type Query } from "./search";
import { CANCELLED, openGraph, type GraphHandle } from "./session";
import { watchRepo } from "../watch";
import { WorktreePicker } from "../worktree/WorktreePicker";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { commonCommands, usePalette, type Command } from "../palette/Palette";
import { isComposing, routeKey } from "../keys";
import { copyItem, openContextMenu, selectedText, type MenuEntry } from "../menu/Menu";
import { usePublish } from "../menu/registry";
import { openInEditor } from "../editor";
import { toggleHud } from "../Hud";
import { ScrollMeter } from "../scrollMeter";
import { drawWindow, insideDrawn, type DrawnRange } from "../scrollWindow";
import type { GraphMeta, GraphRow, GraphScope, GraphStats, Label } from "./types";

type Props = {
  /** 저장소 안의 아무 경로(작업트리 또는 plx 호출의 cwd) */
  repo: string;
  /** 처음 고를 커밋(리비전). 없으면 첫 행. */
  focus?: string | null;
  /** 보이는 화면인가(열린 패널의 스택 맨 위) */
  visible: boolean;
  /** 키를 받는 화면인가(보이고, 패널에 포커스가 있다) */
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
/** 그리는 범위 앞뒤로 미리 읽는 행 수 */
const OVERSCAN = 60;
/** 읽지 못한 행 묶음을 다시 읽기까지(ms) */
const RETRY_MS = 1000;
/** 한 행에 보이는 이름표 수(나머지는 +N) */
const MAX_LABELS = 3;
/**
 * 레인 캔버스의 문맥 설정. WebKit(macOS·Linux)에는 CPU 캔버스(willReadFrequently, 픽셀을 읽지는 않는다)를 청한다.
 * GPU 캔버스를 처음 쓸 때 준비하느라 앱이 뜬 뒤 첫 그래프가 늦었다(macOS CI 0.9~3.1초 → CPU 캔버스로 0.27~0.56초).
 * 레인은 선과 점뿐이고 그리는 범위가 바뀔 때만 다시 그려 CPU로 충분하다. Chromium(Windows WebView2)은 기본(GPU 캔버스)으로
 * 둔다: CPU 캔버스로 빨라지지 않았다(CI 첫 그래프의 그린 뒤 화면까지 CPU 77·111ms, GPU 97·98·101ms, bench/results.md).
 */
const CANVAS_SETTINGS: CanvasRenderingContext2DSettings | undefined = /\bChrome\//.test(navigator.userAgent)
  ? undefined
  : { willReadFrequently: true };

/** 마지막으로 한 검색(n/N이 되풀이한다). 경로 검색은 일치 행들을 한 번 구해 둔다. */
type Found = { kind: "text"; text: string } | { kind: "path"; path: string; rows: number[]; set: ReadonlySet<number> };

function GraphPane({
  repo,
  focus = null,
  scope,
  onScope,
  visible,
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
  /** 그리는 범위의 기준 스크롤 위치. 스크롤마다가 아니라 미리 그린 범위를 벗어날 때 바뀐다. */
  const [viewport, setViewport] = useState({ top: 0, height: 0 });
  /** 지금 그린 행 범위(스크롤 이벤트가 다시 그릴지 판단한다). 행과 레인 캔버스가 스크롤 내용 안에 있어 이 범위 안의
   * 스크롤은 브라우저가 옮기기만 한다(src/scrollWindow.ts). */
  const drawnRange = useRef<DrawnRange>({ start: 0, end: 0, height: 0 });
  /** 행 묶음이 도착할 때마다 올려 다시 그린다. */
  const [version, setVersion] = useState(0);
  /** 보이는 세션이 바뀔 때마다 올린다(행을 다시 읽는다). */
  const [generation, setGeneration] = useState(0);
  const [palette, setPalette] = useState<Palette | null>(null);
  const [help, setHelp] = useState(false);
  const [worktrees, setWorktrees] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [search, setSearch] = useState({ open: false, input: "" });
  const [found, setFound] = useState<Found | null>(null);
  const [searching, setSearching] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  /** 검색마다 올린다. 늦게 온 앞 검색의 결과는 버린다. */
  const searchSeq = useRef(0);
  const cache = useRef(new RowCache());
  /** 보이는 세션 */
  const handle = useRef<GraphHandle | null>(null);
  /** 다시 읽는 중인 세션(첫 화면이 오면 보이는 세션과 바꾼다) */
  const pending = useRef<GraphHandle | null>(null);
  const sessionSeq = useRef(0);
  /** 아직 이루지 못한 처음 요청(`G`로 연 커밋 등). 그사이 다시 읽으면 새 세션도 이것을 고른다. */
  const pendingFocus = useRef<Focus | null>(null);
  /** 고른 행의 sha(그 행의 묶음이 캐시에서 밀려나도 다시 읽을 때 같은 커밋을 고른다) */
  const selectedSha = useRef<{ index: number; sha: string | null }>({ index: -1, sha: null });
  const listRef = useRef<HTMLDivElement>(null);
  const quickOpen = useQuickOpen({
    repo: meta?.repo ?? repo,
    current: null,
    onNavigate,
    onClosed: () => listRef.current?.focus({ preventScroll: true }),
  });
  const canvasRef = useRef<HTMLCanvasElement>(null);
  /** 숨겼다 다시 보일 때 되돌릴 스크롤 위치 */
  const savedTop = useRef(0);
  const toastTimer = useRef(0);
  const nowAtOpen = useRef(Date.now());
  const timing = useRef({
    t0: nowMs(),
    metaMs: NaN,
    pageMs: NaN,
    /** 첫 화면의 레인을 캔버스에 그린 때와 그리는 데 든 시간 */
    lanesMs: NaN,
    lanesDrawMs: NaN,
    shown: false,
    painted: null as Promise<number | null> | null,
    reported: false,
  });
  const [accelRequest, setAccelRequest] = useState(0);
  // 명령 팔레트: 키로 하는 일 모두와 머리줄 버튼(범위, 가속 설정). 함수들은 아래에 있다(누를 때 부른다).
  const commands: Command[] = [
    { id: "commit", label: "커밋 상세", keys: "Enter", run: () => openCommit() },
    { label: "커밋 sha 복사", keys: "y", run: () => copySha() },
    ...SCOPES.map(
      (s): Command => ({
        label: `범위: ${s.name}`,
        keys: s.scope === "all" ? "R" : undefined,
        detail: s.title,
        run: () => changeScope(s.scope),
      }),
    ),
    { label: "검색 (메시지·작성자·sha, path:경로)", keys: "/", run: () => openSearch() },
    { label: "다음 검색 결과", keys: "n", run: () => runSearch(true) },
    { label: "이전 검색 결과", keys: "N", run: () => runSearch(false) },
    { label: "맨 위(최근) 커밋으로", keys: "Home", run: () => select(0) },
    { label: "맨 아래 커밋으로", keys: "End", run: () => select((latest.current.total ?? 1) - 1) },
    { label: "에디터로 열기", keys: "o", detail: "작업트리 폴더", run: () => openEditor() },
    { label: "가속 설정 (commit-graph·Bloom 필터)", detail: "그래프·히스토리를 빠르게", run: () => setAccelRequest((n) => n + 1) },
    ...commonCommands({
      files: quickOpen.open,
      worktrees: () => setWorktrees(true),
      help: () => setHelp(true),
      back: onBack,
      hud: getPerfInfo().enabled ? toggleHud : undefined,
    }),
  ];
  const commandPalette = usePalette(commands, () => listRef.current?.focus({ preventScroll: true }));
  // 도구 막대·메뉴 막대(보이는 동안)
  usePublish("panel", visible, "그래프", commands, {
    palette: commandPalette.open,
    help: () => setHelp(true),
    files: quickOpen.open,
  });
  const latest = useRef({
    visible,
    active,
    meta,
    selected,
    help,
    total,
    found,
    openFiles: quickOpen.open,
    palette: commandPalette.open,
  });
  // 스크롤 프레임 측정(성능 HUD가 켜져 있을 때만)
  const scrollMeter = useRef<ScrollMeter | null>(null);
  scrollMeter.current ??= new ScrollMeter(
    () => `그래프 ${lastName(latest.current.meta?.repo ?? repo)}`,
    ROW_HEIGHT,
  );
  useEffect(() => () => scrollMeter.current?.dispose(), []);
  latest.current = {
    visible,
    active,
    meta,
    selected,
    help,
    total,
    found,
    openFiles: quickOpen.open,
    palette: commandPalette.open,
  };
  {
    const row = cache.current.get(selected);
    if (selectedSha.current.index !== selected) selectedSha.current = { index: selected, sha: row?.sha ?? null };
    else if (row) selectedSha.current.sha = row.sha;
  }

  const showToast = (message: string) => {
    setToast(message);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), TOAST_MS);
  };

  /**
   * 그래프 세션을 연다. 처음에는 바로 보이고, 다시 읽을 때(저장소가 바뀜)는 새 세션의 첫 화면이 올 때까지 앞 세션을
   * 그대로 보이다가 바꿔 끼운다(깜빡이지 않게). 더 새 세션이 열리면 앞 세션의 이벤트는 버린다.
   */
  const openSession = (focusOn: Focus, reload: boolean) => {
    const seq = ++sessionSeq.current;
    const startedMs = nowMs();
    let nextMeta: GraphMeta | null = null;
    pending.current?.close();
    pending.current = null;
    const graph = openGraph(repo, scope, (event) => {
      const showing = handle.current === graph;
      // 더 새 세션이 열렸으면 이 세션의 이벤트는 버린다. 단 보이는 세션은 바꿔 끼울 때까지 그린다(다시 읽기가
      // 실패해도 보이는 그래프가 멈추지 않게).
      if (seq !== sessionSeq.current && !showing) return;
      switch (event.kind) {
        case "meta": {
          const { kind: _kind, ...m } = event;
          if (showing) {
            timing.current.metaMs = nowMs();
            setMeta(m);
          } else {
            nextMeta = m;
          }
          break;
        }
        case "loading":
          if (showing) setProgress({ kind: "loading", commits: event.commits });
          break;
        case "page":
          if (showing) {
            timing.current.pageMs = nowMs();
          } else {
            handle.current?.close();
            handle.current = graph;
            pending.current = null;
            cache.current = new RowCache();
            if (nextMeta) setMeta(nextMeta);
            refreshSearch(graph);
            const total = event.total;
            const receivedMs = nowMs();
            // 바뀐 저장소를 다시 그린 데까지(외부 커밋 → 화면 반영, 예산 1초의 우리 몫)
            void afterPaint().then((painted) => {
              const end = painted ?? nowMs();
              record({
                kind: "graph",
                label: `그래프 다시 읽음 ${lastName(nextMeta?.repo ?? repo)}`,
                totalMs: end - startedMs,
                phases: [
                  { label: "첫 화면 받음", ms: receivedMs - startedMs },
                  { label: "표시", ms: end - startedMs },
                ],
                detail: `커밋 ${total}`,
                ok: painted !== null,
              });
            });
          }
          cache.current.seed(event.rows);
          setAvailable(event.rows.length);
          setTotal(event.total);
          setProgress({ kind: "layout" });
          setGeneration(seq);
          focusRow(graph, focusOn);
          break;
        case "progress":
          if (showing) setAvailable(event.available);
          break;
        case "done":
          if (showing) {
            setAvailable(event.stats.rows);
            setProgress({ kind: "done", stats: event.stats });
          }
          break;
        case "error":
          if (showing) setProgress({ kind: "failed", message: event.message });
          else showToast(`그래프를 다시 읽지 못했다: ${event.message}`);
          break;
        case "cancelled":
          break;
      }
    });
    if (reload) pending.current = graph;
    else handle.current = graph;
  };

  /** 새 세션의 첫 화면에서 고를 행 */
  const focusRow = (graph: GraphHandle, focusOn: Focus) => {
    if (focusOn.kind === "top") {
      pendingFocus.current = null;
      setSelected(0);
      if (listRef.current) listRef.current.scrollTop = 0;
      return;
    }
    graph.find(focusOn.kind === "rev" ? focusOn.rev : "HEAD").then(
      (row) => {
        // 보이는 세션의 답이면 쓴다(더 새 세션을 읽는 중이어도: 바꿔 끼우면 그 세션이 같은 요청을 이룬다).
        if (handle.current !== graph) return;
        if (pendingFocus.current === focusOn) pendingFocus.current = null;
        if (row === null) {
          if (focusOn.kind === "rev" && focusOn.offset === undefined) {
            showToast(`이 그래프에 없는 커밋이다: ${shortSha(focusOn.rev)}`);
          }
          return;
        }
        const list = listRef.current;
        if (focusOn.kind === "rev" && focusOn.offset !== undefined && list) {
          // 다시 읽은 그래프: 고른 커밋을 화면의 같은 자리에 둔다(위에 새 커밋이 생겨 행 번호가 밀려도).
          setSelected(row);
          list.scrollTop = Math.max(0, row * ROW_HEIGHT - focusOn.offset);
        } else {
          select(row, true);
        }
      },
      (e: unknown) => focusOn.kind === "rev" && focusOn.offset === undefined && showToast(String(e)),
    );
  };

  /**
   * 저장소가 바뀌었다: 맨 위를 보고 있었으면 맨 위(새 커밋)를, 아니면 고른 커밋을 같은 자리에 두고 다시 읽는다.
   * 처음 요청한 커밋을 아직 고르지 못했으면(그래프를 읽는 중에 커밋이 생겼다) 그 요청을 이어 간다.
   */
  const reload = () => {
    const { selected: at } = latest.current;
    const top = listRef.current?.scrollTop ?? 0;
    // 고른 행의 묶음이 캐시에서 밀려났으면 기억한 sha, 그것도 없으면 보이는 첫 행(늘 캐시에 있다)을 같은 자리에 둔다.
    const remembered = selectedSha.current.index === at ? selectedSha.current.sha : null;
    const focusOn = reloadFocus({
      pending: pendingFocus.current,
      scrollTop: top,
      rowHeight: ROW_HEIGHT,
      selected: at,
      selectedSha: cache.current.get(at)?.sha ?? remembered,
      firstVisibleSha: cache.current.get(Math.floor(top / ROW_HEIGHT))?.sha ?? null,
    });
    openSession(focusOn, true);
  };

  // 그래프를 연다. 화면을 떠나면(끝난 뒤에도) 닫아 백엔드의 레이아웃을 버린다.
  useEffect(() => {
    const initial: Focus = focus ? { kind: "rev", rev: focus } : { kind: "head" };
    pendingFocus.current = initial;
    openSession(initial, false);
    return () => {
      sessionSeq.current += 1;
      handle.current?.close();
      handle.current = null;
      pending.current?.close();
      window.clearTimeout(toastTimer.current);
    };
    // 범위마다 새 컴포넌트다(GraphView가 key를 준다).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 저장소의 HEAD·참조가 바뀌면(다른 터미널·워크트리의 커밋 등) 다시 읽는다. 숨어 있으면(다른 화면 아래, 닫은 패널:
  // 스크롤 위치를 읽을 수 없다) 표시만 해 두고 다시 보일 때 읽는다.
  const stale = useRef(false);
  useEffect(() => {
    return watchRepo(repo, () => {
      if (latest.current.visible) reload();
      else stale.current = true;
    });
    // reload는 ref만 읽는다. 범위마다 새 컴포넌트다.
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
        { label: "레인 그림", ms: t.lanesMs - t.t0 },
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
        detail: [
          `커밋 ${stats.rows} · 레인 ${stats.width} · DAG ${stats.loadMs.toFixed(1)}ms (${sources.join(", ") || "없음"})`,
          Number.isFinite(t.lanesDrawMs) && `캔버스 ${t.lanesDrawMs.toFixed(1)}ms`,
          longFrameIn(t.t0, end),
        ]
          .filter(Boolean)
          .join(" · "),
        ok: painted !== null,
      });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [progress]);

  // 그리는 범위(와 앞뒤 여유)의 행을 읽는다.
  useEffect(() => {
    if (total === null) return;
    const { start, end } = drawWindow(viewport.top, viewport.height, ROW_HEIGHT, total);
    const from = Math.max(0, start - OVERSCAN);
    // 응답이 오기 전에 세션을 바꿔 끼웠으면(다시 읽기) 그 응답은 앞 세션의 행이라 버린다.
    const rows = cache.current;
    for (const index of rows.take(from, Math.min(total, end + OVERSCAN), available)) {
      handle.current?.rows(index * CHUNK, (index + 1) * CHUNK).then(
        (window) => {
          if (cache.current !== rows) return;
          rows.put(window);
          // 덜 받은 묶음(레이아웃 진행 중에 요청)이면 다음 실행이 나머지를 다시 읽는다(version이 바뀐다).
          setVersion((v) => v + 1);
        },
        () => {
          if (cache.current !== rows) return;
          rows.failed(index);
          // 실패한 묶음은 잠시 뒤 다시 읽는다(곧바로 되풀이하지 않게).
          window.setTimeout(() => setVersion((v) => v + 1), RETRY_MS);
        },
      );
    }
  }, [viewport, available, total, generation, version]);

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

  // 다시 보이면 스크롤을 되돌리고(숨은 동안에는 읽을 수 없다) 그사이 바뀐 저장소를 읽는다.
  useEffect(() => {
    const list = listRef.current;
    if (!visible || !list) return;
    if (list.scrollTop !== savedTop.current) list.scrollTop = savedTop.current;
    setViewport({ top: list.scrollTop, height: list.clientHeight });
    if (stale.current) {
      stale.current = false;
      reload();
    }
    // reload는 ref만 읽는다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);

  // 키를 받는 화면이 되면 목록에 포커스를 준다. 키를 놓으면(숨거나 다른 칸으로) 떠 있는 창을 닫는다(돌아왔을 때
  // 포커스 없이 남지 않게).
  useEffect(() => {
    if (active) {
      listRef.current?.focus({ preventScroll: true });
    } else {
      setHelp(false);
      setWorktrees(false);
      quickOpen.close();
      commandPalette.close();
    }
    // close는 상태만 바꾼다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
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

  const openEditor = () => openInEditor({ repo: meta?.repo ?? repo, path: null, rev: null, line: null }).then(showToast);

  /**
   * 세션을 바꿔 끼웠다: 하던 검색은 버리고(앞 세션의 행 번호다), 경로 일치는 새 세션에서 다시 구한다(위에 커밋이
   * 생기면 행 번호가 밀린다). 메시지 검색은 행마다 다시 맞춰 보므로 그대로 둔다.
   */
  const refreshSearch = (graph: GraphHandle) => {
    const seq = ++searchSeq.current;
    setSearching(false);
    const last = latest.current.found;
    if (last?.kind !== "path") return;
    graph.searchPath(last.path).then(
      (matched) => {
        if (seq === searchSeq.current) setFound({ kind: "path", path: last.path, rows: matched, set: new Set(matched) });
      },
      () => {
        if (seq === searchSeq.current) setFound(null);
      },
    );
  };

  /**
   * 고른 행 다음(`forward`가 아니면 앞)의 일치로 간다. `input`을 주면 그 입력으로 찾고, 없으면 마지막 검색을 되풀이한다.
   * 메시지·작성자·sha는 백엔드가 행 순서대로 커밋을 풀어 보고, 경로는 일치 행을 한 번 구해 그 사이를 오간다.
   */
  const runSearch = (forward: boolean, input?: string) => {
    const graph = handle.current;
    const { found: last, selected: from, total: rows } = latest.current;
    if (!graph || rows === null) return;
    const query: Query | null =
      input !== undefined
        ? parseQuery(input)
        : last?.kind === "path"
          ? { kind: "path", path: last.path }
          : last
            ? { kind: "text", text: last.text }
            : null;
    if (!query) return;
    const seq = ++searchSeq.current;
    const failed = (e: unknown) => {
      if (seq === searchSeq.current && String(e) !== CANCELLED) showToast(String(e));
    };
    const finished = () => {
      if (seq === searchSeq.current) setSearching(false);
    };
    if (query.kind === "path") {
      if (last?.kind === "path" && last.path === query.path) {
        const row = nextRow(last.rows, from, forward);
        if (row === null) showToast(`이 경로를 바꾼 커밋이 없다: ${query.path}`);
        else select(row, true);
        return;
      }
      setSearching(true);
      graph
        .searchPath(query.path)
        .then((matched) => {
          if (seq !== searchSeq.current) return;
          setFound({ kind: "path", path: query.path, rows: matched, set: new Set(matched) });
          const row = nextRow(matched, latest.current.selected, forward);
          if (row === null) showToast(`이 경로를 바꾼 커밋이 없다: ${query.path}`);
          else select(row, true);
        }, failed)
        .finally(finished);
    } else {
      setFound({ kind: "text", text: query.text });
      setSearching(true);
      graph
        .search(query.text, from, forward)
        .then((row) => {
          if (seq !== searchSeq.current) return;
          if (row === null) showToast(`찾지 못했다: ${query.text}`);
          else select(row, true);
        }, failed)
        .finally(finished);
    }
  };

  useEffect(() => {
    if (search.open) searchRef.current?.focus();
    else if (latest.current.active) listRef.current?.focus({ preventScroll: true });
  }, [search.open]);

  /** 행 우클릭: 그 커밋을 고르고 그 커밋의 일을 보인다. 이름표(브랜치·태그)는 이름을 복사한다. */
  const rowMenu = (index: number, row: GraphRow | undefined): MenuEntry[] => {
    const copy = (text: string, what: string) => () =>
      copyText(text).then(
        () => showToast(`복사했다: ${what}`),
        (e: unknown) => showToast(String(e)),
      );
    const names = (row?.labels ?? []).filter((label) => label.kind !== "head");
    return [
      { label: "커밋 상세", keys: "Enter", disabled: !row, run: () => openCommit(index) },
      "separator",
      { label: "커밋 sha 복사", keys: "y", disabled: !row, run: copySha },
      { label: "요약 복사", disabled: !row, run: () => row && copy(row.summary, row.summary)() },
      ...names.slice(0, 4).map((label) => ({ label: "이름 복사", detail: label.name, run: copy(label.name, label.name) })),
      "separator",
      { label: "에디터로 열기", keys: "o", detail: "작업트리 폴더", run: () => void openEditor() },
    ];
  };

  /** 머리줄 우클릭: 그래프의 일(보일 브랜치, 검색, 가속 설정) */
  const headerMenu = (e: ReactMouseEvent) => {
    openContextMenu(
      e,
      [
        copyItem(selectedText(e.target)),
        "separator",
        ...SCOPES.map(
          (s): MenuEntry => ({
            label: `범위: ${s.name}${s.scope === scope ? " (지금)" : ""}`,
            keys: s.scope === "all" ? "R" : undefined,
            detail: s.title,
            run: () => changeScope(s.scope),
          }),
        ),
        "separator",
        { label: "검색 (메시지·작성자·sha, path:경로)", keys: "/", run: openSearch },
        { label: "가속 설정 (commit-graph·Bloom 필터)", detail: "그래프·히스토리를 빠르게", run: () => setAccelRequest((n) => n + 1) },
        { label: "에디터로 열기", keys: "o", detail: "작업트리 폴더", run: () => void openEditor() },
      ],
      "그래프",
    );
  };

  /** 검색 칸을 연다. 이미 열려 있으면(목록을 눌러 포커스가 떠났다) 그리로 포커스를 옮긴다. */
  const openSearch = () => {
    setSearch((s) => ({ ...s, open: true }));
    searchRef.current?.focus();
  };

  // 키: 화면이 맨 위일 때만.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { active: on, openFiles, palette: openPalette } = latest.current;
      if (!on || routeKey(e, { files: openFiles, palette: openPalette }) !== "view") return;
      // 머리줄 버튼에 초점이 있으면 Enter·Space는 버튼이 받는다.
      if ((e.target as HTMLElement | null)?.closest("button") && (e.key === "Enter" || e.key === " ")) return;
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
        case "/":
          openSearch();
          break;
        case "n":
          runSearch(true);
          break;
        case "N":
          runSearch(false);
          break;
        case "R":
          changeScope(scope === "all" ? "branches" : "all");
          break;
        case "q":
          onBack();
          break;
        case "W":
          setWorktrees(true);
          break;
        case "o":
          openEditor();
          break;
        case "?":
          setHelp(true);
          break;
        case "Escape":
          // 도움말을 닫고, 없으면 검색 강조를 지운다.
          if (helpOpen) setHelp(false);
          else if (latest.current.found) setFound(null);
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

  const count = total ?? 0;
  const { start, end } = drawWindow(viewport.top, viewport.height, ROW_HEIGHT, count);
  const drawnRows: Visible[] = [];
  let visibleLanes = 1;
  for (let i = start; i < end; i++) {
    const row = cache.current.get(i);
    if (row) {
      drawnRows.push({ index: i, row });
      visibleLanes = Math.max(visibleLanes, rowWidth(row));
    }
  }
  const width = graphWidth(Math.max(lanes, visibleLanes));
  useLayoutEffect(() => {
    if (visibleLanes > lanes) setLanes(visibleLanes);
  });

  // 레인은 그리는 범위의 행만 캔버스에 그린다. 캔버스는 스크롤 내용 안(범위의 첫 행 자리)에 있어 범위 안의 스크롤은
  // 다시 그리지 않는다. 범위·행 도착·너비·HEAD·테마가 바뀔 때만 다시 그린다.
  const drawnLanes = useRef<{ key: string; palette: Palette | null }>({ key: "", palette: null });
  useLayoutEffect(() => {
    drawnRange.current = { start, end, height: viewport.height };
    const canvas = canvasRef.current;
    const key = `${start}:${end}:${width}:${meta?.head ?? ""}:${version}:${generation}`;
    let canvasMs: number | null = null;
    if (canvas && palette && (drawnLanes.current.key !== key || drawnLanes.current.palette !== palette)) {
      const t0 = nowMs();
      drawnLanes.current = { key, palette };
      const dpr = window.devicePixelRatio || 1;
      const height = (end - start) * ROW_HEIGHT;
      const w = Math.round(width * dpr);
      const h = Math.round(height * dpr);
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
        canvas.style.width = `${width}px`;
        canvas.style.height = `${height}px`;
      }
      const ctx = canvas.getContext("2d", CANVAS_SETTINGS);
      if (ctx) {
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, width, height);
        drawRows(ctx, drawnRows, start * ROW_HEIGHT, width, palette, meta?.head ?? null);
      }
      const drawnAt = nowMs();
      canvasMs = drawnAt - t0;
      // 첫 화면의 레인(성능 HUD): '표시'가 그리기에서 늘었는지, 그린 뒤 페인트를 기다린 것인지 나눠 본다.
      const t = timing.current;
      if (t.shown && Number.isNaN(t.lanesMs)) {
        t.lanesMs = drawnAt;
        t.lanesDrawMs = canvasMs;
      }
    }
    // 스크롤 측정: 지금 보이는 행 가운데 아직 오지 않은 것이 있었나
    const list = listRef.current;
    if (list) {
      const shown = visibleRange(list.scrollTop, list.clientHeight, ROW_HEIGHT, count, 0);
      let blank = false;
      for (let i = shown.start; i < shown.end && !blank; i++) blank = cache.current.get(i) === undefined;
      scrollMeter.current?.drawn(blank, canvasMs);
    }
  });

  const chosen = current();
  const matchText = found?.kind === "text" ? found.text : null;
  const rowClass = (i: number, row: GraphRow | undefined) => {
    const classes = ["graph-row"];
    if (i === selected) classes.push("selected");
    if (found?.kind === "path" ? found.set.has(i) : matchText !== null && row && rowMatches(row, matchText)) {
      classes.push("match");
    }
    return classes.join(" ");
  };
  const hidden =
    meta && scope === "branches" && meta.hiddenRemotes > 0
      ? `원격 브랜치 ${formatCount(meta.hiddenRemotes)}개 숨김`
      : meta && scope === "head" && meta.hiddenBranches > 0
        ? `브랜치 ${formatCount(meta.hiddenBranches)}개 숨김`
        : null;

  return (
    <section className="graph">
      <header className="blame-header" onContextMenu={headerMenu}>
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
        {meta && <Acceleration repo={meta.repo} request={accelRequest} active={active} onDone={showToast} />}
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
      {search.open && (
        <div className="history-search graph-search">
          <input
            ref={searchRef}
            type="search"
            value={search.input}
            placeholder="메시지·작성자·sha, 경로는 path:경로 (Enter 다음, Shift+Enter 이전, Esc 닫기)"
            aria-label="그래프 검색"
            onChange={(e) => setSearch({ open: true, input: e.target.value })}
            onKeyDown={(e) => {
              // IME 조합 중의 Enter·Esc는 조합을 끝내는 키다(Quick Open·팔레트와 같다).
              if (isComposing(e.nativeEvent)) return;
              if (e.key === "Enter") {
                e.preventDefault();
                runSearch(!e.shiftKey, search.input);
              } else if (e.key === "Escape") {
                e.preventDefault();
                setSearch((s) => ({ ...s, open: false }));
              }
            }}
          />
          <span className="search-status" role="status">
            {searching ? "찾는 중…" : found?.kind === "path" ? `경로 일치 ${formatCount(found.rows.length)}개` : null}
          </span>
        </div>
      )}
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
            if (latest.current.visible) savedTop.current = list.scrollTop;
            const meter = scrollMeter.current;
            meter?.scrolled(list.scrollTop);
            // 미리 그린 범위 안이면 브라우저가 옮기기만 한다(다시 그리지 않는다).
            const count = latest.current.total ?? 0;
            if (insideDrawn(list.scrollTop, list.clientHeight, ROW_HEIGHT, count, drawnRange.current)) {
              meter?.drawn(false);
              return;
            }
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
                  className={rowClass(i, row)}
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
                  onContextMenu={(e) => {
                    select(i);
                    openContextMenu(e, rowMenu(i, row), "커밋");
                  }}
                >
                  {row ? <Row row={row} nowMs={nowAtOpen.current} /> : <span className="muted">…</span>}
                </div>
              );
            })}
            <canvas className="graph-lanes" ref={canvasRef} aria-hidden="true" style={{ top: start * ROW_HEIGHT }} />
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
      {worktrees && (
        <WorktreePicker
          repo={meta?.repo ?? repo}
          onClose={() => setWorktrees(false)}
          onPick={(worktree) => {
            setWorktrees(false);
            if (!worktree.current) onNavigate({ kind: "graph", repo: worktree.path, rev: null, worktree: true });
          }}
        />
      )}
      {quickOpen.overlay}
      {commandPalette.overlay}
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}

/** 행 글(이름표·요약·작성자·시각·sha). 범위를 다시 그릴 때 이미 그린 행은 건너뛴다. */
const Row = memo(function Row({ row, nowMs }: { row: GraphRow; nowMs: number }) {
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
});

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
          key={`${label.kind}:${label.name}:${label.worktree ?? ""}`}
          className={`label label-${label.kind}${label.current ? " current" : ""}${label.worktree ? " label-worktree" : ""}`}
          title={`${LABEL_TITLE[label.kind]}${label.current ? " · HEAD" : ""}: ${label.name}${
            label.worktree ? ` (워크트리 ${label.worktree}에서 체크아웃)` : ""
          }`}
        >
          {label.name}
          {label.worktree && <span className="label-wt">{label.worktree}</span>}
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
