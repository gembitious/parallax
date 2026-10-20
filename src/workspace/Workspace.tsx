// 작업 공간: 왼쪽 파일 트리, 가운데 탭(파일 blame), 아래 패널(히스토리·라인 히스토리·그래프·커밋 상세), 맨 아래 상태
// 막대. 무엇을 어디에 여는지는 state.ts(순수 함수)가 정하고 여기서는 그리기·키·마우스만 한다. 화면(BlameView 등)은
// 숨긴 채 살려 둬 다시 보이면 커서·스크롤이 그대로다.
import {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactElement,
} from "react";
import { flushSync } from "react-dom";
import { BlameView, requestGotoLine, type EditorStatus } from "../blame/BlameView";
import { shortSha } from "../blame/format";
import { toggleBlameGutter, useBlameGutter } from "../blame/gutterSetting";
import { CommitView } from "../commit/CommitView";
import { openInEditor } from "../editor";
import { GraphView } from "../graph/GraphView";
import { Help } from "../Help";
import { HistoryView } from "../history/HistoryView";
import { toggleHud } from "../Hud";
import { call } from "../ipc";
import { isComposing } from "../keys";
import { openContextMenu, type MenuEntry } from "../menu/Menu";
import { setTarget, usePublish } from "../menu/registry";
import { backDirectionAt, lastName, titleOf, type Navigation, type StackEntry } from "../nav";
import { commonCommands, usePalette, type Command } from "../palette/Palette";
import { isPaletteKey } from "../palette/keys";
import { getPerfInfo } from "../perf";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { isQuickOpenKey } from "../quickopen/pick";
import { OPEN_REPO_KEYS } from "../start/keys";
import { openRepoDialog } from "../start/open";
import { toast } from "../Toasts";
import { watchRepo } from "../watch";
import { FileTree } from "./FileTree";
import { IS_MAC, MOD, workspaceKey, type WorkspaceKey } from "./keys";
import { absolutePath, copyPath } from "./paths";
import { canGoBack, targetPane, topOf, type Pane, type Place, type Tab, type Workspace, type WorkspaceAction } from "./state";
import { Toolbar } from "./Toolbar";

/** src-tauri/src/git.rs RepoState */
type RepoState = { repo: string; name: string; branch: string | null; head: string | null };

type Props = {
  state: Workspace;
  dispatch(action: WorkspaceAction): void;
  /** 화면 하나를 다시 연다(저장소가 바뀌었다). 라이선스 창이 떠 있으면 App이 닫을 때까지 미룬다. */
  reload(action: Extract<WorkspaceAction, { type: "reload" }>): void;
  /** 화면이 다른 화면으로 가자고 했다. 라이선스 창이 떠 있으면 App이 버린다. */
  navigate(navigation: Navigation, from: Place): void;
  /** plx 요청으로 연 화면이 첫 화면을 DOM에 넣었다 */
  onShown(request: number, committedMs: number): void;
  hideWindow(): void;
};

const SHIFT_MOD = IS_MAC ? "⌘⇧" : "Ctrl+Shift+";
const TREE_MIN_PX = 140;
const PANEL_MIN_RATIO = 0.15;
const PANEL_MAX_RATIO = 0.85;

/** 이 컴퓨터에 남기는 화면 설정(트리 너비 등). 저장하지 못해도 이번 실행에는 반영된다. */
function usePersisted<T extends boolean | number>(key: string, initial: T): [T, (value: T) => void, () => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const saved = localStorage.getItem(key);
      if (saved === null) return initial;
      const parsed: unknown = JSON.parse(saved);
      return typeof parsed === typeof initial ? (parsed as T) : initial;
    } catch {
      return initial;
    }
  });
  const latest = useRef(value);
  latest.current = value;
  const save = () => {
    try {
      localStorage.setItem(key, JSON.stringify(latest.current));
    } catch {
      // 다음 실행에는 처음 값이다.
    }
  };
  return [value, setValue, save];
}

/** 끌어서 크기를 바꾸는 막대. `onMove`는 포인터 자리(clientX·clientY)를 받는다. */
function Splitter({
  vertical,
  label,
  onMove,
  onDone,
}: {
  vertical: boolean;
  label: string;
  onMove(x: number, y: number): void;
  onDone(): void;
}) {
  const down = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const target = e.currentTarget;
    target.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => onMove(ev.clientX, ev.clientY);
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", up);
      onDone();
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", up);
  };
  return (
    <div
      className={`splitter ${vertical ? "vertical" : "horizontal"}`}
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      aria-label={label}
      title={label}
      onPointerDown={down}
    />
  );
}

/**
 * 화면 요소를 화면(key)마다 기억해 두고 넘길 값이 그대로면 같은 요소를 돌려준다. React는 같은 요소를 다시 그리지
 * 않으므로 요청·탭 이동마다 다른 탭과 숨은 화면들까지 다시 그리지 않는다. 콜백은 ref로 지금 상태를 읽으므로
 * 처음 만든 것을 그대로 써도 된다.
 */
function useElementCache() {
  const cache = useRef(new Map<number, { deps: readonly unknown[]; element: ReactElement }>());
  const used = new Set<number>();
  const get = (key: number, deps: readonly unknown[], make: () => ReactElement): ReactElement => {
    used.add(key);
    const hit = cache.current.get(key);
    if (hit && hit.deps.length === deps.length && hit.deps.every((dep, i) => Object.is(dep, deps[i]))) return hit.element;
    const element = make();
    cache.current.set(key, { deps, element });
    return element;
  };
  /** 이번 렌더에서 쓰지 않은 화면(닫았다)을 잊는다 */
  const prune = () => {
    for (const key of cache.current.keys()) if (!used.has(key)) cache.current.delete(key);
  };
  return { get, prune };
}

/** 저장소를 아직 모르면(콜드 스타트) 요청의 대상으로 찾는다. */
function seedPath(state: Workspace): string | null {
  if (state.repo !== null) return state.repo;
  const request = state.shown?.request;
  if (!request) return null;
  const view = request.request;
  return view.kind === "blame" || view.kind === "log" || view.kind === "lineLog" ? view.path : request.cwd;
}

type CursorStatus = { key: number; status: EditorStatus };

/**
 * 보이는 탭의 커서 자리(줄·열·인코딩·줄 끝). 커서를 옮길 때마다 바뀌므로 상태 막대만 다시 그린다(작업 공간과
 * 화면들은 그대로다).
 */
class CursorStore {
  private current: CursorStatus | null = null;
  private listeners = new Set<() => void>();

  set = (key: number, status: EditorStatus) => {
    this.current = { key, status };
    for (const listener of this.listeners) listener();
  };

  get = () => this.current;

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
}

function StatusBar({
  cursors,
  activeKey,
  repoState,
  onGraph,
}: {
  cursors: CursorStore;
  activeKey: number | null;
  repoState: RepoState | null;
  /** 브랜치를 눌렀다: 저장소 그래프 */
  onGraph(): void;
}) {
  const cursor = useSyncExternalStore(cursors.subscribe, cursors.get);
  const showGutter = useBlameGutter();
  const status = cursor && cursor.key === activeKey ? cursor.status : null;
  return (
    <footer className="status-bar">
      {repoState && (
        <button
          type="button"
          className="status-item"
          onClick={onGraph}
          title={`${repoState.repo}\n${repoState.head ?? "커밋 없음"}\n누르면 커밋 그래프`}
        >
          {repoState.branch ? `⎇ ${repoState.branch}` : repoState.head ? `HEAD ${shortSha(repoState.head)} (detached)` : "커밋 없음"}
        </button>
      )}
      <span className="status-spacer" />
      {status && (
        <>
          <button type="button" className="status-item" onClick={requestGotoLine} title="줄로 이동 (Ctrl+G)">
            줄 {status.line}, 열 {status.col}
          </button>
          {status.encoding && <span className="status-item">{status.encoding}</span>}
          <span className="status-item">{status.crlf ? "CRLF" : "LF"}</span>
        </>
      )}
      <button
        type="button"
        className="status-item"
        aria-pressed={showGutter}
        onClick={toggleBlameGutter}
        title={showGutter ? "blame 거터 숨기기: 커서 줄 끝에만 blame 한 줄 (b)" : "blame 거터 보이기: 줄마다 커밋 (b)"}
      >
        blame 거터 {showGutter ? "켬" : "끔"}
      </button>
    </footer>
  );
}

function TabBar({
  tabs,
  active,
  onActivate,
  onPin,
  onClose,
  onMenu,
}: {
  tabs: Tab[];
  active: number | null;
  onActivate(tab: number): void;
  onPin(tab: number): void;
  onClose(tab: number): void;
  /** 우클릭: 탭 메뉴 */
  onMenu(tab: Tab, e: ReactMouseEvent): void;
}) {
  const activeRef = useRef<HTMLDivElement>(null);
  // 아무것도 돌려주지 않는다: 새 Chromium(Windows CI의 WebView2)은 scrollIntoView가 Promise를 돌려줘, 그것을 정리
  // 함수로 받은 React가 탭이 바뀔 때 부르다 실패하고 화면을 모두 내렸다.
  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active]);
  return (
    <div className="tab-bar" role="tablist" aria-label="열린 파일">
      {tabs.map((tab) => {
        const { path, rev } = topOf(tab).target;
        const name = lastName(path);
        const classes = ["tab"];
        if (tab.key === active) classes.push("active");
        if (tab.preview) classes.push("preview");
        return (
          <div
            key={tab.key}
            ref={tab.key === active ? activeRef : undefined}
            role="tab"
            aria-selected={tab.key === active}
            className={classes.join(" ")}
            title={`${path}${rev ? ` @${shortSha(rev)}` : ""}${tab.preview ? "\n미리보기: 두 번 누르면 고정한다" : ""}`}
            onMouseDown={(e) => {
              // 가운데 단추로 닫는다
              if (e.button === 1) {
                e.preventDefault();
                onClose(tab.key);
              }
            }}
            onClick={() => onActivate(tab.key)}
            onDoubleClick={() => onPin(tab.key)}
            onContextMenu={(e) => onMenu(tab, e)}
          >
            <span className="tab-name">{name}</span>
            {rev && <span className="tab-rev">{shortSha(rev)}</span>}
            <button
              type="button"
              className="tab-close"
              aria-label={`${name} 닫기`}
              title={`닫기 (${MOD}W)`}
              onClick={(e) => {
                e.stopPropagation();
                onClose(tab.key);
              }}
            >
              ×
            </button>
          </div>
        );
      })}
    </div>
  );
}

export function WorkspaceView({ state, dispatch, reload, navigate, onShown, hideWindow }: Props) {
  const [treeOpen, setTreeOpen, saveTreeOpen] = usePersisted<boolean>("plx.tree.open", true);
  const [treeWidth, setTreeWidth, saveTreeWidth] = usePersisted<number>("plx.tree.width", 260);
  const [panelRatio, setPanelRatio, savePanelRatio] = usePersisted<number>("plx.panel.ratio", 0.4);
  const [repoState, setRepoState] = useState<RepoState | null>(null);
  const [cursors] = useState(() => new CursorStore());
  const [help, setHelp] = useState(false);
  // 작업 공간의 창(Quick Open·팔레트·도움말)을 닫으면 키를 받던 칸(트리)에 포커스를 돌려준다.
  const [focusTick, setFocusTick] = useState(0);
  const refocus = () => setFocusTick((tick) => tick + 1);
  const closeHelp = () => {
    setHelp(false);
    refocus();
  };
  const showGutter = useBlameGutter();
  const mainRef = useRef<HTMLDivElement>(null);
  const centerRef = useRef<HTMLDivElement>(null);

  const activeTab = state.tabs.find((tab) => tab.key === state.active) ?? null;
  const activeEntry = activeTab ? topOf(activeTab) : null;
  const activeFile =
    activeEntry && activeEntry.target.rev === null && activeEntry.target.repo !== null ? activeEntry.target.path : null;
  const editorShown = !(state.panelOpen && state.panelMax);
  const panelShown = state.panelOpen && state.panel.length > 0;
  /** 키를 받는 화면이 없다(트리, 탭이 없는 에디터): 작업 공간이 Quick Open·팔레트·도움말 키를 받는다 */
  const noView = state.focus === "tree" || (state.focus === "editor" && !activeTab) || (state.focus === "panel" && !panelShown);

  const focusPane = (focus: Pane | "tree") => {
    if (latest.current.state.focus !== focus) dispatch({ type: "focus", focus });
  };
  /** 메뉴·도구 막대가 명령을 실행하기 전에: 그 칸에 포커스를 주고 화면이 키를 받는 화면이 된 뒤에 돌아온다(registry runIn). */
  const focusSlot = (pane: Pane) => {
    if (latest.current.state.focus !== pane) flushSync(() => dispatch({ type: "focus", focus: pane }));
  };
  const back = (from: Place) => {
    if (canGoBack(latest.current.state, from)) dispatch({ type: "back", from });
    else hideWindow();
  };
  const openGraph = () => {
    const { repo } = latest.current.state;
    if (repo) dispatch({ type: "navigate", navigation: { kind: "graph", repo, rev: null }, from: { pane: "editor", tab: -1 } });
  };
  /** Mod+W·메뉴 막대: 보이는 탭을 닫는다. 탭이 없으면 창을 숨긴다(패널이 에디터를 가렸으면 아무것도 하지 않는다). */
  const closeTabOrHide = () => {
    const s = latest.current.state;
    if (s.active === null) hideWindow();
    else if (!(s.panelOpen && s.panelMax)) dispatch({ type: "close", tab: s.active });
  };
  const toggleTree = () => {
    const open = !latest.current.treeOpen;
    setTreeOpen(open);
    latest.current.treeOpen = open;
    saveTreeOpen();
    if (!open && latest.current.state.focus === "tree") leaveTree();
  };
  const focusTree = () => {
    if (!latest.current.treeOpen) {
      setTreeOpen(true);
      latest.current.treeOpen = true;
      saveTreeOpen();
    }
    dispatch({ type: "focus", focus: "tree" });
  };
  /** 트리에서 나간다: 보이는 탭, 없으면 패널, 둘 다 없으면 빈 에디터 */
  const leaveTree = () => {
    const s = latest.current.state;
    dispatch({ type: "focus", focus: s.active === null && s.panelOpen && s.panel.length > 0 ? "panel" : "editor" });
  };

  const quickOpen = useQuickOpen({
    repo: state.repo,
    current: activeFile,
    onNavigate: (navigation) => navigate(navigation, { pane: "editor", tab: state.active ?? -1 }),
    onClosed: refocus,
  });

  // 작업 공간 명령: 어느 화면의 팔레트에도 넣고(commonCommands가 읽는다), 메뉴 막대의 보기·파일 메뉴에 둔다.
  const workspaceCommands: Command[] = [
    {
      id: "tree",
      label: treeOpen ? "파일 트리 숨기기" : "파일 트리 보이기",
      keys: `${MOD}B`,
      accelerator: "CmdOrCtrl+B",
      app: true,
      run: toggleTree,
    },
    {
      id: "treeFocus",
      label: "파일 트리로",
      keys: `${SHIFT_MOD}E`,
      accelerator: "CmdOrCtrl+Shift+E",
      detail: "키로 파일을 고른다",
      app: true,
      run: focusTree,
    },
    {
      id: "panel",
      label: state.panelOpen ? "아래 패널 닫기" : "아래 패널 열기",
      keys: `${MOD}J`,
      accelerator: "CmdOrCtrl+J",
      detail: "히스토리·그래프·커밋 상세",
      app: true,
      run: () => dispatch({ type: "togglePanel" }),
    },
    ...(state.panelOpen
      ? [
          {
            id: "panelMax" as const,
            label: state.panelMax ? "아래 패널 되돌리기" : "아래 패널 최대화",
            app: true,
            run: () => dispatch({ type: "maximizePanel" }),
          },
        ]
      : []),
    ...(activeTab
      ? [{ id: "closeTab" as const, label: "탭 닫기", keys: `${MOD}W`, accelerator: "CmdOrCtrl+W", app: true, run: closeTabOrHide }]
      : []),
    ...(state.tabs.length > 1
      ? [
          { id: "nextTab" as const, label: "다음 탭", keys: "Ctrl+Tab", app: true, run: () => dispatch({ type: "cycle", delta: 1 }) },
          {
            id: "previousTab" as const,
            label: "이전 탭",
            keys: "Ctrl+Shift+Tab",
            app: true,
            run: () => dispatch({ type: "cycle", delta: -1 }),
          },
        ]
      : []),
  ];

  const palette = usePalette([
    { label: "커밋 그래프 (아래 패널)", keys: "G", run: openGraph },
    { label: showGutter ? "blame 거터 숨기기" : "blame 거터 보이기", keys: "b", run: toggleBlameGutter },
    ...commonCommands({
      files: quickOpen.open,
      worktrees: () => {},
      help: () => setHelp(true),
      back: () => (activeTab ? back({ pane: "editor", tab: activeTab.key }) : hideWindow()),
      hud: getPerfInfo().enabled ? toggleHud : undefined,
    }).filter((command) => command.keys !== "W"),
  ], refocus);

  usePublish("workspace", true, "작업 공간", workspaceCommands, {
    palette: palette.open,
    help: () => setHelp(true),
    files: quickOpen.open,
    graph: openGraph,
    closeTab: closeTabOrHide,
  });
  // 도구 막대·메뉴 막대가 명령을 보내는 칸(키를 받는 칸, 트리면 보이는 화면). 화면들이 명령을 낸 뒤에 정한다.
  const target = targetPane(state);
  useEffect(() => setTarget(target, focusSlot));
  useEffect(() => () => setTarget(null, () => {}), []);

  const overlay = help || quickOpen.isOpen || palette.isOpen;
  const latest = useRef({ state, treeOpen, noView, help, overlay, quickOpen, palette });
  latest.current = { state, treeOpen, noView, help, overlay, quickOpen, palette };

  // 작업 공간 키(트리·패널·탭)는 어느 칸에서든 화면보다 먼저 받는다. 떠 있는 창(팔레트·Quick Open의 Ctrl+J 등,
  // 라이선스)과 메뉴의 키는 그 창의 것이다. 키를 받는 화면이 없으면(트리, 탭이 없는 에디터) Quick Open·팔레트·도움말·
  // 그래프·q도 여기서 받는다.
  useEffect(() => {
    const run: Record<WorkspaceKey, () => void> = {
      toggleTree,
      focusTree,
      togglePanel: () => dispatch({ type: "togglePanel" }),
      nextTab: () => dispatch({ type: "cycle", delta: 1 }),
      previousTab: () => dispatch({ type: "cycle", delta: -1 }),
      closeTab: closeTabOrHide,
    };
    const onCapture = (e: KeyboardEvent) => {
      if (e.defaultPrevented || isComposing(e) || (e.target as Element | null)?.closest?.(".overlay, .menu")) return;
      const key = workspaceKey(e);
      if (!key) return;
      e.preventDefault();
      e.stopPropagation();
      run[key]();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || isComposing(e)) return;
      if (latest.current.help) {
        if (e.key === "Escape" || e.key === "q" || e.key === "?") {
          e.preventDefault();
          closeHelp();
        }
        return;
      }
      const { noView: on, overlay: open } = latest.current;
      if (!on || open) return;
      if (isPaletteKey(e)) latest.current.palette.open();
      else if (isQuickOpenKey(e)) latest.current.quickOpen.open();
      else if (e.ctrlKey || e.metaKey || e.altKey || (e.target as Element | null)?.closest?.("input, textarea")) return;
      else if (e.key === "?") setHelp(true);
      else if (e.key === "G") openGraph();
      else if (e.key === "b") toggleBlameGutter();
      else if (e.key === "q" || e.key === "Escape") {
        if (latest.current.state.focus === "tree") leaveTree();
        else if (e.key === "q") hideWindow();
        else return;
      } else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onCapture, true);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onCapture, true);
      window.removeEventListener("keydown", onKey);
    };
    // 처리기는 ref만 읽는다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 머리줄·상태 막대의 저장소 이름과 브랜치. HEAD·참조가 바뀌면 다시 읽는다. 콜드 스타트는 여기서 저장소를 안다.
  const seed = seedPath(state);
  useEffect(() => {
    if (!seed) return;
    let disposed = false;
    const read = () =>
      call<RepoState>("repo_state", { path: seed }).then(
        (found) => {
          if (disposed) return;
          setRepoState(found);
          dispatch({ type: "repo", repo: found.repo });
        },
        (e: unknown) => console.error("저장소 상태를 읽지 못했다", e),
      );
    void read();
    const stop = watchRepo(seed, () => void read());
    return () => {
      disposed = true;
      stop();
    };
    // dispatch는 바뀌지 않는다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [seed]);

  const shownFor = (entry: StackEntry) =>
    entry.request === undefined ? undefined : (ms: number) => onShown(entry.request!, ms);
  const coldFor = (entry: StackEntry) =>
    entry.request !== undefined && entry.request === state.shown?.request.id && state.shown.request.cold;

  const views = useElementCache();
  const editorViews = state.tabs.flatMap((tab) =>
    tab.stack.map((entry, i) => {
      const visible = editorShown && tab.key === state.active && i === tab.stack.length - 1;
      const active = visible && state.focus === "editor";
      const backDirection = backDirectionAt(tab.stack, i);
      const cold = coldFor(entry);
      const place: Place = { pane: "editor", tab: tab.key };
      return views.get(entry.key, [entry, visible, active, i, backDirection, cold], () => (
        <div key={entry.key} className="stack-entry" hidden={!visible}>
          <BlameView
            target={entry.target}
            line={entry.line}
            visible={visible}
            active={active}
            depth={i + 1}
            backDirection={backDirection}
            search={entry.search}
            cold={cold}
            onShown={shownFor(entry)}
            onBack={() => back(place)}
            onNavigate={(navigation) => navigate(navigation, place)}
            onReload={(line, search) => reload({ type: "reload", key: entry.key, line, search })}
            onLocated={(repo, path) => dispatch({ type: "located", key: entry.key, repo, path })}
            onStatus={(next) => cursors.set(entry.key, next)}
          />
        </div>
      ));
    }),
  );

  const panelViews = state.panel.map((entry, i) => {
    const visible = panelShown && i === state.panel.length - 1;
    const active = visible && state.focus === "panel";
    const cold = coldFor(entry);
    const place: Place = { pane: "panel" };
    return views.get(entry.key, [entry, visible, active, i, cold], () => {
      const common = {
        visible,
        active,
        depth: i + 1,
        onShown: shownFor(entry),
        onBack: () => back(place),
        onNavigate: (navigation: Navigation) => navigate(navigation, place),
      };
      return (
        <div key={entry.key} className="stack-entry" hidden={!visible}>
          {entry.view === "history" ? (
            <HistoryView
              {...common}
              target={entry.target}
              lines={entry.lines}
              query={entry.query}
              cold={cold}
              onReload={(lines, query) => reload({ type: "reload", key: entry.key, lines, query })}
              onLocated={(repo, path) => dispatch({ type: "located", key: entry.key, repo, path })}
            />
          ) : entry.view === "graph" ? (
            <GraphView {...common} repo={entry.repo} focus={entry.focus} cold={cold} />
          ) : (
            <CommitView {...common} repo={entry.repo} rev={entry.rev} path={entry.path} only={entry.only} />
          )}
        </div>
      );
    });
  });
  views.prune();

  const repoName = repoState?.name ?? (state.repo ? lastName(state.repo) : "");

  // 도구 막대의 뒤로: 명령을 보내는 칸의 화면에서 `q`와 같다(맨 앞 화면이면 꺼 둔다. `q`는 창을 숨긴다).
  const targetPlace: Place | null =
    target === "editor" && activeTab ? { pane: "editor", tab: activeTab.key } : target === "panel" ? { pane: "panel" } : null;

  /** 파일 히스토리(트리·탭 메뉴). 디렉터리면 그 아래를 바꾼 커밋이다. */
  const openHistory = (repo: string, path: string, rev: string | null, from: Place) =>
    navigate({ kind: "history", target: { repo, path, rev } }, from);

  /** 탭 우클릭 메뉴 */
  const tabMenu = (tab: Tab, e: ReactMouseEvent) => {
    const top = topOf(tab);
    const { repo, path, rev } = top.target;
    const place: Place = { pane: "editor", tab: tab.key };
    // 보이는 탭이면 커서 줄을 에디터로 연다.
    const cursor = cursors.get();
    const line = cursor && cursor.key === top.key ? cursor.status.line : null;
    const entries: MenuEntry[] = [
      { label: "닫기", keys: tab.key === state.active ? `${MOD}W` : undefined, run: () => dispatch({ type: "close", tab: tab.key }) },
      { label: "다른 탭 닫기", disabled: state.tabs.length < 2, run: () => dispatch({ type: "closeOthers", tab: tab.key }) },
      { label: "모든 탭 닫기", run: () => dispatch({ type: "closeAll" }) },
      ...(tab.preview ? [{ label: "탭 고정", detail: "미리보기를 보통 탭으로 (두 번 누르기)", run: () => dispatch({ type: "pin", tab: tab.key }) }] : []),
      "separator",
      {
        label: "파일 히스토리",
        keys: "H",
        disabled: repo === null,
        run: () => repo !== null && openHistory(repo, path, rev, place),
      },
      {
        label: "에디터로 열기",
        keys: "o",
        disabled: repo === null,
        run: () => repo !== null && void openInEditor({ repo, path, rev, line }).then((message) => toast(message)),
      },
      "separator",
      { label: "경로 복사", detail: "저장소 기준", disabled: repo === null, run: () => copyPath(path) },
      { label: "전체 경로 복사", disabled: repo === null, run: () => repo !== null && copyPath(absolutePath(repo, path)) },
    ];
    openContextMenu(e, entries, `${lastName(path)} 탭`);
  };

  /** 파일 트리 머리·빈 자리 우클릭: 저장소의 일. 행은 행의 메뉴를 연다(이미 열었으면 넘긴다). */
  const repoMenu = (e: ReactMouseEvent) => {
    const { repo } = latest.current.state;
    if (e.nativeEvent.defaultPrevented || !repo) return;
    const from: Place = { pane: "editor", tab: latest.current.state.active ?? -1 };
    openContextMenu(
      e,
      [
        { label: "파일 찾기", keys: `f · ${MOD}P`, run: quickOpen.open },
        { label: "커밋 그래프", keys: "G", run: openGraph },
        { label: "폴더 히스토리", detail: "저장소 전체", run: () => openHistory(repo, ".", null, from) },
        "separator",
        {
          label: "에디터로 열기",
          detail: "작업트리 폴더",
          run: () => void openInEditor({ repo, path: null, rev: null, line: null }).then((message) => toast(message)),
        },
        { label: "전체 경로 복사", run: () => copyPath(repo) },
        "separator",
        { label: "파일 트리 숨기기", keys: `${MOD}B`, run: toggleTree },
        { label: "다른 저장소 열기…", keys: OPEN_REPO_KEYS, run: openRepoDialog },
      ],
      repoName,
    );
  };

  /** 패널 머리 우클릭: 패널의 일 */
  const panelMenu = (e: ReactMouseEvent) => {
    const s = latest.current.state;
    openContextMenu(
      e,
      [
        { label: "뒤로", keys: "q", disabled: !canGoBack(s, { pane: "panel" }), run: () => back({ pane: "panel" }) },
        "separator",
        { label: s.panelMax ? "패널 되돌리기" : "패널 최대화", run: () => dispatch({ type: "maximizePanel" }) },
        { label: "패널 닫기", keys: `${MOD}J`, run: () => dispatch({ type: "togglePanel" }) },
      ],
      "패널",
    );
  };

  return (
    <div className="workspace">
      <Toolbar canBack={targetPlace !== null && canGoBack(state, targetPlace)} onBack={() => targetPlace && back(targetPlace)} />
      <div className="workspace-main" ref={mainRef}>
        {treeOpen && state.repo && (
          <>
            <aside
              className={`sidebar${state.focus === "tree" ? " focused" : ""}`}
              style={{ width: treeWidth }}
              aria-label="파일 트리"
              onMouseDownCapture={() => focusPane("tree")}
              onFocusCapture={() => focusPane("tree")}
              onContextMenu={repoMenu}
            >
              <div className="sidebar-header" title={state.repo}>
                {repoName}
              </div>
              <FileTree
                key={state.repo}
                repo={state.repo}
                focused={state.focus === "tree"}
                focusTick={focusTick}
                activeFile={activeFile}
                onOpen={(path, missing, preview) => dispatch({ type: "open", repo: state.repo!, path, missing, preview })}
                onHistory={(path) => openHistory(state.repo!, path, null, { pane: "editor", tab: state.active ?? -1 })}
                onLeave={leaveTree}
              />
            </aside>
            <Splitter
              vertical
              label="파일 트리 너비"
              onMove={(x) => {
                const main = mainRef.current?.getBoundingClientRect();
                if (main) setTreeWidth(Math.round(Math.max(TREE_MIN_PX, Math.min(main.width * 0.6, x - main.left))));
              }}
              onDone={saveTreeWidth}
            />
          </>
        )}
        <div className="workspace-center" ref={centerRef}>
          <div
            className={`editor-area${state.focus === "editor" ? " focused" : ""}`}
            hidden={!editorShown}
            onMouseDownCapture={() => focusPane("editor")}
            onFocusCapture={() => focusPane("editor")}
          >
            {state.tabs.length > 0 && (
              <TabBar
                tabs={state.tabs}
                active={state.active}
                onActivate={(tab) => dispatch({ type: "activate", tab })}
                onPin={(tab) => dispatch({ type: "pin", tab })}
                onClose={(tab) => dispatch({ type: "close", tab })}
                onMenu={tabMenu}
              />
            )}
            <div className="editor-views">
              {editorViews}
              {!activeTab && (
                <div className="editor-empty" tabIndex={-1}>
                  <div className="editor-empty-panel">
                    <p className="editor-empty-title">{repoName}</p>
                    <ul className="editor-empty-actions">
                      <li>
                        <button type="button" className="link" onClick={quickOpen.open}>
                          파일 찾기
                        </button>
                        <kbd>f</kbd> <kbd>{MOD}P</kbd>
                      </li>
                      <li>
                        <button type="button" className="link" onClick={openGraph}>
                          커밋 그래프
                        </button>
                        <kbd>G</kbd>
                      </li>
                      <li>
                        <button type="button" className="link" onClick={toggleTree}>
                          파일 트리 {treeOpen ? "숨기기" : "보이기"}
                        </button>
                        <kbd>{MOD}B</kbd>
                      </li>
                      <li>
                        <button type="button" className="link" onClick={palette.open}>
                          명령 팔레트
                        </button>
                        <kbd>:</kbd> <kbd>{SHIFT_MOD}P</kbd>
                      </li>
                      <li>
                        <button type="button" className="link" onClick={openRepoDialog}>
                          다른 저장소 열기…
                        </button>
                        <kbd>{OPEN_REPO_KEYS}</kbd>
                      </li>
                    </ul>
                    <p className="muted">트리에서 파일을 누르면 미리보기로, 두 번 누르면 탭으로 연다.</p>
                  </div>
                </div>
              )}
            </div>
          </div>
          {state.panel.length > 0 && editorShown && panelShown && (
            <Splitter
              vertical={false}
              label="아래 패널 높이"
              onMove={(_, y) => {
                const center = centerRef.current?.getBoundingClientRect();
                if (!center || center.height <= 0) return;
                const ratio = (center.bottom - y) / center.height;
                setPanelRatio(Math.max(PANEL_MIN_RATIO, Math.min(PANEL_MAX_RATIO, ratio)));
              }}
              onDone={savePanelRatio}
            />
          )}
          {state.panel.length > 0 && (
            <section
              className={`panel${state.focus === "panel" ? " focused" : ""}${editorShown ? "" : " max"}`}
              hidden={!panelShown}
              style={editorShown ? { height: `${Math.round(panelRatio * 1000) / 10}%` } : undefined}
              aria-label="아래 패널"
              onMouseDownCapture={() => focusPane("panel")}
              onFocusCapture={() => focusPane("panel")}
            >
              <div className="panel-header" onContextMenu={panelMenu}>
                <nav className="panel-crumbs" aria-label="패널 화면">
                  {state.panel.map((entry, i) => (
                    <span key={entry.key} className="panel-crumb">
                      {i > 0 && <span className="panel-crumb-sep">›</span>}
                      <button
                        type="button"
                        className={i === state.panel.length - 1 ? "current" : ""}
                        onClick={() => dispatch({ type: "panelTo", key: entry.key })}
                        title={i === state.panel.length - 1 ? undefined : "이 화면으로 돌아간다"}
                      >
                        {titleOf(entry)}
                      </button>
                    </span>
                  ))}
                </nav>
                <button
                  type="button"
                  className="panel-button"
                  aria-label={state.panelMax ? "패널 되돌리기" : "패널 최대화"}
                  title={state.panelMax ? "패널 되돌리기" : "패널 최대화"}
                  onClick={() => dispatch({ type: "maximizePanel" })}
                >
                  {state.panelMax ? "▾" : "▴"}
                </button>
                <button
                  type="button"
                  className="panel-button"
                  aria-label="패널 닫기"
                  title={`패널 닫기 (${MOD}J)`}
                  onClick={() => dispatch({ type: "togglePanel" })}
                >
                  ×
                </button>
              </div>
              <div className="panel-views">{panelViews}</div>
            </section>
          )}
        </div>
      </div>
      <StatusBar cursors={cursors} activeKey={activeEntry?.key ?? null} repoState={repoState} onGraph={openGraph} />
      {help && <Help view="workspace" onClose={closeHelp} />}
      {quickOpen.overlay}
      {palette.overlay}
    </div>
  );
}
