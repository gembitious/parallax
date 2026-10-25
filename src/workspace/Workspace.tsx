// 작업 공간: 왼쪽 파일 트리, 가운데 에디터 그룹(좌우로 나눈 칸, 그룹마다 탭), 아래 패널(화면 스택), 맨 아래 상태 막대.
// 무엇을 어디에 여는지는 state.ts(순수 함수)가 정하고 여기서는 그리기·키·마우스만 한다. 화면(BlameView 등)은 화면마다의
// 자리(hosts.ts)에 portal로 그린다: 탭을 다른 그룹·패널로 옮겨도 화면을 새로 만들지 않고 자리만 옮긴다. 숨긴 화면도 살려
// 둬 다시 보이면 커서·스크롤이 그대로다.
import {
  Fragment,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type MouseEvent as ReactMouseEvent,
  type MutableRefObject,
  type PointerEvent as ReactPointerEvent,
  type ReactElement,
} from "react";
import { createPortal, flushSync } from "react-dom";
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
import { setTarget, usePublish, type ViewSlot } from "../menu/registry";
import { WORKTREE_CHANGES, backDirectionAt, lastName, type Navigation, type StackEntry } from "../nav";
import { commonCommands, usePalette, type Command, type CommandId } from "../palette/Palette";
import { isPaletteKey } from "../palette/keys";
import { getPerfInfo } from "../perf";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { isQuickOpenKey } from "../quickopen/pick";
import { OPEN_REPO_KEYS } from "../start/keys";
import { openRepoDialog } from "../start/open";
import { toast } from "../Toasts";
import { watchRepo } from "../watch";
import { DRAG_THRESHOLD, dropTargetAt, type DragSource, type DropTarget } from "./drag";
import { FileTree } from "./FileTree";
import { ViewHosts, type Placement } from "./hosts";
import { IS_MAC, MOD, workspaceKey, type WorkspaceKey } from "./keys";
import { absolutePath, copyPath } from "./paths";
import {
  MAX_GROUPS,
  activeGroupOf,
  activeTabOf,
  allTabs,
  besideTarget,
  canGoBack,
  focusedEntry,
  groupOfTab,
  targetPane,
  topOf,
  type Group,
  type Pane,
  type Place,
  type Tab,
  type Workspace,
  type WorkspaceAction,
} from "./state";
import { TabBar } from "./TabBar";
import { tabLabel } from "./tabs";
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
/** 그룹 사이의 막대를 끌어도 그룹이 이보다 좁아지지 않는다(px) */
const GROUP_MIN_PX = 160;

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
  onDone?(): void;
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
      onDone?.();
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

/**
 * blame 화면마다의 커서 자리(줄·열·인코딩·줄 끝). 커서를 옮길 때마다 바뀌므로 상태 막대만 다시 그린다(작업 공간과
 * 화면들은 그대로다).
 */
class CursorStore {
  private statuses = new Map<number, EditorStatus>();
  private listeners = new Set<() => void>();

  set = (key: number, status: EditorStatus) => {
    this.statuses.set(key, status);
    for (const listener of this.listeners) listener();
  };

  get = (key: number | null): EditorStatus | null => (key === null ? null : (this.statuses.get(key) ?? null));

  /** 닫은 화면의 것을 잊는다 */
  keep = (keys: ReadonlySet<number>) => {
    for (const key of this.statuses.keys()) if (!keys.has(key)) this.statuses.delete(key);
  };

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
}

/** 끄는 동안 보이는 것: 포인터를 따라오는 이름표, 놓을 곳(칸을 덮는 면, 탭 막대에 끼울 자리의 세로줄) */
type DragShow = {
  label: string;
  x: number;
  y: number;
  zone: { line: boolean; left: number; top: number; width: number; height: number } | null;
};

/** 끄는 자리. 포인터가 움직일 때마다 바뀌므로 끌기 층만 다시 그린다. */
class DragStore {
  private current: DragShow | null = null;
  private listeners = new Set<() => void>();

  set = (next: DragShow | null) => {
    this.current = next;
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

/** 놓을 곳을 화면에 보일 자리(창 좌표) */
function zoneOf(target: DropTarget | null): DragShow["zone"] {
  if (!target) return null;
  const area = (el: Element | null, half?: "left" | "right") => {
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    const width = half ? rect.width / 2 : rect.width;
    return { line: false, left: half === "right" ? rect.left + width : rect.left, top: rect.top, width, height: rect.height };
  };
  if (target.kind === "panel") return area(document.querySelector(".panel"));
  if (target.kind === "split") return area(document.querySelector(`[data-views="${target.group}"]`), target.side);
  if (target.index === undefined) return area(document.querySelector(`[data-views="${target.group}"]`));
  const bar = document.querySelector(`[data-tabbar="${target.group}"]`);
  if (!bar) return null;
  const rect = bar.getBoundingClientRect();
  const tabs = bar.querySelectorAll("[data-tab]");
  const x =
    target.index < tabs.length
      ? tabs[target.index].getBoundingClientRect().left
      : tabs.length > 0
        ? tabs[tabs.length - 1].getBoundingClientRect().right
        : rect.left;
  return { line: true, left: Math.max(rect.left, Math.min(rect.right - 2, x - 1)), top: rect.top, width: 2, height: rect.height };
}

function DragLayer({ store }: { store: DragStore }) {
  const drag = useSyncExternalStore(store.subscribe, store.get);
  if (!drag) return null;
  const { zone } = drag;
  return (
    <div className="drag-layer" aria-hidden="true">
      {zone && (
        <div
          className={`drop-zone${zone.line ? " line" : ""}`}
          style={{ left: zone.left, top: zone.top, width: zone.width, height: zone.height }}
        />
      )}
      <div className="drag-label" style={{ left: drag.x + 14, top: drag.y + 12 }}>
        {drag.label}
      </div>
    </div>
  );
}

/**
 * 화면 자리를 칸에 놓는다. 칸(그룹·패널의 요소)이 DOM에 붙은 뒤, 화면들의 layout effect보다 먼저 돌아야 한다: React는
 * layout effect를 자식부터, 형제는 앞에서부터 부르므로 칸들 뒤, portal들 앞에 둔다.
 */
function HostPlacer({
  hosts,
  placements,
  containers,
  cursors,
}: {
  hosts: ViewHosts;
  placements: Placement[];
  containers: MutableRefObject<Map<string, HTMLElement>>;
  cursors: CursorStore;
}) {
  useLayoutEffect(() => {
    hosts.place(placements, containers.current);
    cursors.keep(new Set(placements.map((placement) => placement.key)));
  });
  return null;
}

function StatusBar({
  cursors,
  statusKey,
  repoState,
  onGraph,
  onGotoLine,
}: {
  cursors: CursorStore;
  /** 커서 자리를 보이는 blame 화면 */
  statusKey: number | null;
  repoState: RepoState | null;
  /** 브랜치를 눌렀다: 저장소 그래프 */
  onGraph(): void;
  /** 줄·열을 눌렀다: 그 화면에서 줄로 이동 */
  onGotoLine(): void;
}) {
  const status = useSyncExternalStore(cursors.subscribe, () => cursors.get(statusKey));
  const showGutter = useBlameGutter();
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
          <button type="button" className="status-item" onClick={onGotoLine} title="줄로 이동 (Ctrl+G)">
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

/** 그룹 너비 몫의 합. flex-grow의 합이 1보다 작으면 남는 자리를 다 채우지 않아 몫을 합으로 나눠 준다. */
function total(groups: readonly Group[]): number {
  return groups.reduce((sum, group) => sum + group.weight, 0);
}

/** 눌러도 포커스를 가져가지 않는다(Chromium은 누른 단추에 포커스를 줘, 다음 Enter가 그 단추를 또 누른다). */
const keepFocus = (e: ReactMouseEvent) => e.preventDefault();

/** 끌 때 놓은 뒤 오는 click(탭 누르기·머리줄의 앞 화면)을 버린다. click은 pointerup과 같은 작업 안에 온다. */
function swallowClick(): void {
  const stop = (e: MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
  };
  window.addEventListener("click", stop, true);
  window.setTimeout(() => window.removeEventListener("click", stop, true), 0);
}

export function WorkspaceView({ state, dispatch, reload, navigate, onShown, hideWindow }: Props) {
  const [treeOpen, setTreeOpen, saveTreeOpen] = usePersisted<boolean>("plx.tree.open", true);
  const [treeWidth, setTreeWidth, saveTreeWidth] = usePersisted<number>("plx.tree.width", 260);
  const [panelRatio, setPanelRatio, savePanelRatio] = usePersisted<number>("plx.panel.ratio", 0.4);
  const [repoState, setRepoState] = useState<RepoState | null>(null);
  const [cursors] = useState(() => new CursorStore());
  const [drags] = useState(() => new DragStore());
  const [hosts] = useState(() => new ViewHosts());
  /** 화면 자리를 넣는 칸: `group:<그룹 key>`, `panel` */
  const containers = useRef(new Map<string, HTMLElement>());
  const containerRefs = useRef(new Map<string, (el: HTMLDivElement | null) => void>());
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
  const editorRef = useRef<HTMLDivElement>(null);

  const activeGroup = activeGroupOf(state);
  const activeTab = activeTabOf(state);
  const activeTop = activeTab ? topOf(activeTab) : null;
  const activeFile =
    activeTop?.view === "blame" && activeTop.target.rev === null && activeTop.target.repo !== null ? activeTop.target.path : null;
  const editorShown = !(state.panelOpen && state.panelMax);
  const panelShown = state.panelOpen && state.panel.length > 0;
  /** 키를 받는 화면이 없다(트리, 탭이 없는 에디터): 작업 공간이 Quick Open·팔레트·도움말 키를 받는다 */
  const noView = state.focus === "tree" || (state.focus === "editor" && !activeTab) || (state.focus === "panel" && !panelShown);

  const containerRef = (name: string) => {
    let ref = containerRefs.current.get(name);
    if (!ref) {
      ref = (el: HTMLDivElement | null) => {
        if (el) containers.current.set(name, el);
        else containers.current.delete(name);
      };
      containerRefs.current.set(name, ref);
    }
    return ref;
  };

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
  /** 작업 공간(트리·빈 화면·메뉴)에서 여는 화면의 자리: 키를 받는 그룹의 보이는 탭 */
  const fromActive = (): Place => ({ pane: "editor", tab: activeTabOf(latest.current.state)?.key ?? -1 });
  const openGraph = () => {
    const { repo } = latest.current.state;
    if (repo) dispatch({ type: "navigate", navigation: { kind: "graph", repo, rev: null }, from: { pane: "editor", tab: -1 } });
  };
  /** 작업트리 변경(HEAD ↔ 작업트리, 아래 패널). `path`면 그 파일을 골라 둔다. */
  const openChanges = (path: string | null = null) => {
    const { repo } = latest.current.state;
    if (!repo) return;
    dispatch({
      type: "navigate",
      navigation: { kind: "commit", repo, rev: WORKTREE_CHANGES.base, path, compare: WORKTREE_CHANGES },
      from: { pane: "editor", tab: -1 },
    });
  };
  /** Mod+W·메뉴 막대: 키를 받는 그룹의 보이는 탭을 닫는다. 탭이 없으면 창을 숨긴다(패널이 에디터를 가렸으면 아무것도 하지 않는다). */
  const closeTabOrHide = () => {
    const s = latest.current.state;
    const tab = activeTabOf(s);
    if (tab === null) hideWindow();
    else if (!(s.panelOpen && s.panelMax)) dispatch({ type: "close", tab: tab.key });
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
    dispatch({ type: "focus", focus: activeTabOf(s) === null && s.panelOpen && s.panel.length > 0 ? "panel" : "editor" });
  };
  /** 탭의 화면을 옆 새 그룹에 하나 더 연다. blame은 커서 줄로. */
  const splitTab = (tab: number, side: "left" | "right") => {
    const found = allTabs(latest.current.state).find((t) => t.key === tab);
    if (!found) return;
    const top = topOf(found);
    dispatch({ type: "split", tab, side, line: top.view === "blame" ? cursors.get(top.key)?.line : undefined });
  };
  const splitActive = () => {
    const tab = activeTabOf(latest.current.state);
    if (tab) splitTab(tab.key, "right");
  };
  /** 탭을 옆 그룹으로(없으면 그쪽에 나눠) 옮긴다 */
  const moveBeside = (tab: number, side: "left" | "right") => {
    const to = besideTarget(latest.current.state, tab, side);
    if (to) dispatch({ type: "moveTab", tab, to });
  };

  const quickOpen = useQuickOpen({
    repo: state.repo,
    current: activeFile,
    onNavigate: (navigation) => navigate(navigation, { pane: "editor", tab: activeTab?.key ?? -1 }),
    onClosed: refocus,
  });

  // 작업 공간 명령: 어느 화면의 팔레트에도 넣고(commonCommands가 읽는다), 메뉴 막대의 보기·파일 메뉴에 둔다.
  const leftTo = activeTab ? besideTarget(state, activeTab.key, "left") : null;
  const rightTo = activeTab ? besideTarget(state, activeTab.key, "right") : null;
  const workspaceCommands: Command[] = [
    {
      id: "changes",
      label: "작업트리 변경",
      keys: "D",
      detail: "커밋하지 않은 것 (아래 패널)",
      app: true,
      run: () => openChanges(),
    },
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
      ? [
          {
            id: "split" as const,
            label: "에디터 나누기",
            keys: `${MOD}\\`,
            accelerator: "CmdOrCtrl+\\",
            detail: "보이는 탭을 오른쪽에 하나 더 연다",
            app: true,
            run: splitActive,
          },
          ...(leftTo
            ? [{ id: "tabLeft" as const, label: "탭을 왼쪽 그룹으로", app: true, run: () => moveBeside(activeTab.key, "left") }]
            : []),
          ...(rightTo
            ? [{ id: "tabRight" as const, label: "탭을 오른쪽 그룹으로", app: true, run: () => moveBeside(activeTab.key, "right") }]
            : []),
          {
            id: "tabToPanel" as const,
            label: "탭을 아래 패널로",
            app: true,
            run: () => dispatch({ type: "tabToPanel", tab: activeTab.key }),
          },
        ]
      : []),
    ...(panelShown
      ? [
          {
            id: "panelToEditor" as const,
            label: "패널 화면을 탭으로",
            detail: "아래 패널의 화면을 에디터의 탭으로 옮긴다",
            app: true,
            run: () => dispatch({ type: "panelToEditor" }),
          },
        ]
      : []),
    ...(state.groups.length > 1
      ? state.groups.map(
          (_, i): Command => ({
            id: `group${i + 1}` as CommandId,
            label: `에디터 그룹 ${i + 1}로`,
            keys: `${MOD}${i + 1}`,
            accelerator: `CmdOrCtrl+${i + 1}`,
            app: true,
            run: () => dispatch({ type: "focusGroupAt", index: i }),
          }),
        )
      : []),
    ...(activeTab
      ? [{ id: "closeTab" as const, label: "탭 닫기", keys: `${MOD}W`, accelerator: "CmdOrCtrl+W", app: true, run: closeTabOrHide }]
      : []),
    ...(activeGroup.tabs.length > 1
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

  // 작업 공간 키(트리·패널·탭·그룹)는 어느 칸에서든 화면보다 먼저 받는다. 떠 있는 창(팔레트·Quick Open의 Ctrl+J 등,
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
      splitEditor: splitActive,
      group1: () => dispatch({ type: "focusGroupAt", index: 0 }),
      group2: () => dispatch({ type: "focusGroupAt", index: 1 }),
      group3: () => dispatch({ type: "focusGroupAt", index: 2 }),
      group4: () => dispatch({ type: "focusGroupAt", index: 3 }),
    };
    const onCapture = (e: KeyboardEvent) => {
      if (e.defaultPrevented || isComposing(e) || (e.target as Element | null)?.closest?.(".overlay, .menu")) return;
      // D(작업트리 변경)는 어느 화면에서든 받는다. 입력란 안과 떠 있는 창(도움말, 팔레트 등)에서는 아니다.
      if (
        e.key === "D" &&
        !(e.ctrlKey || e.metaKey || e.altKey) &&
        !latest.current.help &&
        !latest.current.overlay &&
        !document.querySelector(".overlay") &&
        !(e.target as Element | null)?.closest?.("input, textarea")
      ) {
        e.preventDefault();
        e.stopPropagation();
        openChanges();
        return;
      }
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

  // 누르거나 포커스가 들어온 칸이 키를 받는다(그룹, 패널). 화면은 portal이라 React 이벤트는 작업 공간의 칸을 거치지 않는다:
  // DOM에서 받는다.
  useEffect(() => {
    const center = centerRef.current;
    if (!center) return;
    const follow = (e: Event) => {
      const el = e.target instanceof Element ? e.target : null;
      const s = latest.current.state;
      const group = el?.closest<HTMLElement>("[data-group]");
      if (group) {
        const key = Number(group.dataset.group);
        if (s.activeGroup !== key || s.focus !== "editor") dispatch({ type: "focusGroup", group: key });
      } else if (el?.closest(".panel") && s.focus !== "panel") {
        dispatch({ type: "focus", focus: "panel" });
      }
    };
    center.addEventListener("mousedown", follow, true);
    center.addEventListener("focusin", follow, true);
    return () => {
      center.removeEventListener("mousedown", follow, true);
      center.removeEventListener("focusin", follow, true);
    };
    // dispatch는 바뀌지 않는다.
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

  // 화면들: 화면마다 자리(host)에 portal로 그리고, 자리를 그룹·패널 칸에 놓는다(HostPlacer).
  const views = useElementCache();
  const placements: Placement[] = [];
  const portals: { key: number; element: ReactElement }[] = [];
  const addView = (
    entry: StackEntry,
    container: string,
    place: Place,
    stack: readonly StackEntry[],
    index: number,
    shown: { visible: boolean; active: boolean; slot: ViewSlot | null },
  ) => {
    const { visible, active, slot } = shown;
    placements.push({ key: entry.key, container, visible, active });
    const depth = index + 1;
    const backDirection = backDirectionAt(stack, index);
    const cold = coldFor(entry);
    const at = place.pane === "panel" ? -1 : place.tab;
    const element = views.get(entry.key, [entry, visible, active, slot, depth, backDirection, cold, at], () => {
      const common = {
        visible,
        active,
        slot,
        depth,
        onShown: shownFor(entry),
        onBack: () => back(place),
        onNavigate: (navigation: Navigation) => navigate(navigation, place),
      };
      const view =
        entry.view === "blame" ? (
          <BlameView
            {...common}
            target={entry.target}
            line={entry.line}
            backDirection={backDirection}
            search={entry.search}
            cold={cold}
            onReload={(line, search) => reload({ type: "reload", key: entry.key, line, search })}
            onLocated={(repo, path) => dispatch({ type: "located", key: entry.key, repo, path })}
            onStatus={(next) => cursors.set(entry.key, next)}
          />
        ) : entry.view === "history" ? (
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
          <CommitView
            {...common}
            repo={entry.repo}
            rev={entry.rev}
            path={entry.path}
            only={entry.only}
            compare={entry.compare}
          />
        );
      return createPortal(view, hosts.get(entry.key), String(entry.key));
    });
    portals.push({ key: entry.key, element });
  };
  for (const group of state.groups) {
    const keyGroup = group.key === state.activeGroup;
    for (const tab of group.tabs) {
      const place: Place = { pane: "editor", tab: tab.key };
      tab.stack.forEach((entry, i) => {
        const visible = editorShown && tab.key === group.active && i === tab.stack.length - 1;
        addView(entry, `group:${group.key}`, place, tab.stack, i, {
          visible,
          active: visible && keyGroup && state.focus === "editor",
          slot: visible && keyGroup ? "editor" : null,
        });
      });
    }
  }
  state.panel.forEach((entry, i) => {
    const visible = panelShown && i === state.panel.length - 1;
    addView(entry, "panel", { pane: "panel" }, state.panel, i, {
      visible,
      active: visible && state.focus === "panel",
      slot: visible ? "panel" : null,
    });
  });
  views.prune();
  // 화면 순서는 key 순서로 둔다: 탭을 옮겨도 portal의 순서가 바뀌지 않는다.
  portals.sort((a, b) => a.key - b.key);

  const repoName = repoState?.name ?? (state.repo ? lastName(state.repo) : "");

  // 도구 막대의 뒤로: 명령을 보내는 칸의 화면에서 `q`와 같다(맨 앞 화면이면 꺼 둔다. `q`는 창을 숨긴다).
  const targetPlace: Place | null =
    target === "editor" && activeTab ? { pane: "editor", tab: activeTab.key } : target === "panel" ? { pane: "panel" } : null;

  // 상태 막대의 줄·열: 키를 받는 blame 화면, 아니면 키를 받는 그룹에 보이는 blame 화면
  const focused = focusedEntry(state);
  const statusEntry = focused?.view === "blame" ? focused : activeTop?.view === "blame" ? activeTop : null;
  const statusPane: Pane | null =
    statusEntry === null ? null : state.panel.some((entry) => entry.key === statusEntry.key) ? "panel" : "editor";
  const gotoLine = () => {
    // 그 화면이 키를 받게 한 뒤 연다(줄 이동 칸은 키를 받는 코드 뷰가 연다).
    if (statusPane) focusSlot(statusPane);
    requestGotoLine();
  };

  /** 파일 히스토리(트리·탭 메뉴). 디렉터리면 그 아래를 바꾼 커밋이다. */
  const openHistory = (repo: string, path: string, rev: string | null, from: Place) =>
    navigate({ kind: "history", target: { repo, path, rev } }, from);

  // 끌어 놓기: 탭(다른 자리·그룹·그룹의 옆·패널)과 패널의 화면 스택(그룹·그룹의 옆). 움직여야 끌기다(그 전에 놓으면 누르기).
  const drop = (source: DragSource, to: DropTarget) => {
    if (source.kind === "tab") {
      if (to.kind === "panel") dispatch({ type: "tabToPanel", tab: source.tab });
      else dispatch({ type: "moveTab", tab: source.tab, to });
    } else if (to.kind !== "panel") {
      dispatch({ type: "panelToEditor", to });
    }
  };
  /** 더 나눌 수 있다(탭이 그룹에 혼자면 옮기면서 그 그룹이 빠진다) */
  const canSplit = (source: DragSource) => {
    const s = latest.current.state;
    if (s.groups.length < MAX_GROUPS) return true;
    return source.kind === "tab" && groupOfTab(s, source.tab)?.tabs.length === 1;
  };
  const startDrag = (source: DragSource, label: string, e: ReactPointerEvent<HTMLElement>) => {
    const el = e.currentTarget;
    const { pointerId } = e;
    const x0 = e.clientX;
    const y0 = e.clientY;
    let dragging = false;
    let to: DropTarget | null = null;
    const finish = () => {
      window.removeEventListener("pointermove", move, true);
      window.removeEventListener("pointerup", up, true);
      window.removeEventListener("pointercancel", cancel, true);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("selectstart", noSelect, true);
      document.body.classList.remove("dragging");
      drags.set(null);
      if (el.hasPointerCapture(pointerId)) el.releasePointerCapture(pointerId);
    };
    const move = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      if (!dragging) {
        if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < DRAG_THRESHOLD) return;
        dragging = true;
        // 창 밖으로 나가도 놓는 것을 받는다. 그동안 다른 요소는 포인터를 받지 않는다(blame 거터의 카드가 뜨지 않는다).
        if (el.isConnected) el.setPointerCapture(pointerId);
        document.body.classList.add("dragging");
        window.getSelection()?.removeAllRanges();
      }
      to = dropTargetAt(ev.clientX, ev.clientY, source, canSplit(source));
      drags.set({ label, x: ev.clientX, y: ev.clientY, zone: zoneOf(to) });
    };
    const up = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId) return;
      finish();
      if (!dragging) return;
      swallowClick();
      if (to) drop(source, to);
    };
    const cancel = (ev: PointerEvent) => {
      if (ev.pointerId === pointerId) finish();
    };
    // Esc: 끌기를 그만둔다(놓아도 그대로다).
    const key = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      ev.preventDefault();
      ev.stopPropagation();
      finish();
    };
    // 누른 채 움직이면 글자를 고르지 않는다. WebKit은 user-select: none인 탭에서 누른 것도 옆 글자(코드 뷰, 목록)까지
    // 고르기를 시작하고, 고르는 동안 코드 뷰를 스크롤한다. pointerdown을 막으면 mousedown이 오지 않아 이렇게 막는다.
    const noSelect = (ev: Event) => ev.preventDefault();
    window.addEventListener("pointermove", move, true);
    window.addEventListener("pointerup", up, true);
    window.addEventListener("pointercancel", cancel, true);
    window.addEventListener("keydown", key, true);
    window.addEventListener("selectstart", noSelect, true);
  };

  /** 그룹 사이의 막대(`boundary`번 그룹의 왼쪽)를 `x`로 옮긴다. 양옆 두 그룹의 몫만 바뀐다. */
  const resizeAt = (boundary: number, x: number) => {
    const area = editorRef.current?.getBoundingClientRect();
    const weights = latest.current.state.groups.map((group) => group.weight);
    if (!area || area.width <= 0 || boundary < 1 || boundary >= weights.length) return;
    const total = weights.reduce((sum, w) => sum + w, 0);
    const before = weights.slice(0, boundary - 1).reduce((sum, w) => sum + w, 0);
    const pair = weights[boundary - 1] + weights[boundary];
    const left = area.left + (area.width * before) / total;
    const width = (area.width * pair) / total;
    const min = Math.min(GROUP_MIN_PX, width / 2);
    const at = Math.max(min, Math.min(width - min, x - left));
    weights[boundary - 1] = (pair * at) / width;
    weights[boundary] = pair - weights[boundary - 1];
    dispatch({ type: "resizeGroups", weights });
  };

  /** 탭 우클릭 메뉴 */
  const tabMenu = (tab: Tab, e: ReactMouseEvent) => {
    const s = latest.current.state;
    const group = groupOfTab(s, tab.key);
    if (!group) return;
    const top = topOf(tab);
    const place: Place = { pane: "editor", tab: tab.key };
    const left = besideTarget(s, tab.key, "left");
    const right = besideTarget(s, tab.key, "right");
    const visible = group.key === s.activeGroup && tab.key === group.active;
    const entries: MenuEntry[] = [
      { label: "닫기", keys: visible ? `${MOD}W` : undefined, run: () => dispatch({ type: "close", tab: tab.key }) },
      { label: "다른 탭 닫기", disabled: group.tabs.length < 2, run: () => dispatch({ type: "closeOthers", tab: tab.key }) },
      {
        label: "모든 탭 닫기",
        detail: s.groups.length > 1 ? "이 그룹" : undefined,
        run: () => dispatch({ type: "closeAll", group: group.key }),
      },
      ...(tab.preview
        ? [{ label: "탭 고정", detail: "미리보기를 보통 탭으로 (두 번 누르기)", run: () => dispatch({ type: "pin", tab: tab.key }) }]
        : []),
      "separator",
      {
        label: "오른쪽에 나누기",
        keys: visible ? `${MOD}\\` : undefined,
        detail: "이 탭을 옆에 하나 더",
        disabled: s.groups.length >= MAX_GROUPS,
        run: () => splitTab(tab.key, "right"),
      },
      { label: "왼쪽 그룹으로 옮기기", disabled: left === null, run: () => moveBeside(tab.key, "left") },
      { label: "오른쪽 그룹으로 옮기기", disabled: right === null, run: () => moveBeside(tab.key, "right") },
      { label: "아래 패널로 옮기기", run: () => dispatch({ type: "tabToPanel", tab: tab.key }) },
    ];
    if (top.view === "blame" || top.view === "history") {
      const { repo, path, rev } = top.target;
      // 보이는 blame이면 커서 줄을 에디터로 연다.
      const line = top.view === "blame" ? (cursors.get(top.key)?.line ?? null) : null;
      entries.push(
        "separator",
        ...(top.view === "blame"
          ? [
              {
                label: "파일 히스토리",
                keys: "H",
                disabled: repo === null,
                run: () => repo !== null && openHistory(repo, path, rev, place),
              },
              ...(rev === null
                ? [{ label: "작업트리 변경", detail: "이 파일을 골라서", disabled: repo === null, run: () => openChanges(path) }]
                : []),
            ]
          : []),
        {
          label: "에디터로 열기",
          keys: "o",
          disabled: repo === null || path === ".",
          run: () => repo !== null && void openInEditor({ repo, path, rev, line }).then((message) => toast(message)),
        },
        "separator",
        { label: "경로 복사", detail: "저장소 기준", disabled: repo === null, run: () => copyPath(path) },
        { label: "전체 경로 복사", disabled: repo === null, run: () => repo !== null && copyPath(absolutePath(repo, path)) },
      );
    }
    openContextMenu(e, entries, `${tabLabel(top).name} 탭`);
  };

  /** 파일 트리 머리·빈 자리 우클릭: 저장소의 일. 행은 행의 메뉴를 연다(이미 열었으면 넘긴다). */
  const repoMenu = (e: ReactMouseEvent) => {
    const { repo } = latest.current.state;
    if (e.nativeEvent.defaultPrevented || !repo) return;
    openContextMenu(
      e,
      [
        { label: "파일 찾기", keys: `f · ${MOD}P`, run: quickOpen.open },
        { label: "커밋 그래프", keys: "G", run: openGraph },
        { label: "작업트리 변경", keys: "D", detail: "커밋하지 않은 것", run: () => openChanges() },
        { label: "폴더 히스토리", detail: "저장소 전체", run: () => openHistory(repo, ".", null, fromActive()) },
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
    const group = activeGroupOf(s);
    openContextMenu(
      e,
      [
        { label: "뒤로", keys: "q", disabled: !canGoBack(s, { pane: "panel" }), run: () => back({ pane: "panel" }) },
        "separator",
        { label: "탭으로 옮기기", detail: "에디터의 탭으로 (머리줄을 끌어 놓아도 된다)", run: () => dispatch({ type: "panelToEditor" }) },
        {
          label: "오른쪽에 나눠 탭으로",
          disabled: s.groups.length >= MAX_GROUPS || group.tabs.length === 0,
          run: () => dispatch({ type: "panelToEditor", to: { kind: "split", group: group.key, side: "right" } }),
        },
        "separator",
        { label: s.panelMax ? "패널 되돌리기" : "패널 최대화", run: () => dispatch({ type: "maximizePanel" }) },
        { label: "패널 닫기", keys: `${MOD}J`, run: () => dispatch({ type: "togglePanel" }) },
      ],
      "패널",
    );
  };

  const panelTop = state.panel.length > 0 ? state.panel[state.panel.length - 1] : null;

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
                onOpenSide={(path, missing) =>
                  dispatch({ type: "open", repo: state.repo!, path, missing, preview: false, side: true })
                }
                onHistory={(path) => openHistory(state.repo!, path, null, fromActive())}
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
          <div className="editor-area" ref={editorRef} hidden={!editorShown}>
            {state.groups.map((group, i, groups) => {
              const classes = ["editor-group"];
              if (state.focus === "editor" && group.key === state.activeGroup) classes.push("focused");
              if (state.groups.length > 1 && group.key === state.activeGroup) classes.push("current");
              return (
                <Fragment key={group.key}>
                  {i > 0 && <Splitter vertical label="에디터 그룹 너비" onMove={(x) => resizeAt(i, x)} />}
                  <div className={classes.join(" ")} data-group={group.key} style={{ flex: `${group.weight / total(groups)} 1 0px` }}>
                    {group.tabs.length > 0 && (
                      <TabBar
                        group={group}
                        onActivate={(tab) => dispatch({ type: "activate", tab })}
                        onPin={(tab) => dispatch({ type: "pin", tab })}
                        onClose={(tab) => dispatch({ type: "close", tab })}
                        onMenu={tabMenu}
                        onDragStart={(tab, e) => startDrag({ kind: "tab", tab: tab.key }, tabLabel(topOf(tab)).name, e)}
                      />
                    )}
                    <div className="editor-views" data-views={group.key}>
                      <div className="view-hosts" ref={containerRef(`group:${group.key}`)} />
                      {group.tabs.length === 0 && (
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
                            <p className="muted">
                              트리에서 파일을 누르면 미리보기로, 두 번 누르면 탭으로 연다. 탭을 끌어 옆에 놓으면 나눠 본다.
                            </p>
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                </Fragment>
              );
            })}
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
          {panelTop && (
            <section
              className={`panel${state.focus === "panel" ? " focused" : ""}${editorShown ? "" : " max"}`}
              hidden={!panelShown}
              style={editorShown ? { height: `${Math.round(panelRatio * 1000) / 10}%` } : undefined}
              aria-label="아래 패널"
            >
              <div
                className="panel-header"
                title="끌어서 에디터의 그룹이나 그 옆에 놓으면 탭으로 옮긴다"
                onContextMenu={panelMenu}
                onPointerDown={(e) => {
                  if (e.button === 0 && !(e.target as Element).closest(".panel-button")) {
                    startDrag({ kind: "panel" }, tabLabel(panelTop).name, e);
                  }
                }}
              >
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
                        {tabLabel(entry).name}
                      </button>
                    </span>
                  ))}
                </nav>
                <button
                  type="button"
                  className="panel-button text"
                  title="에디터의 탭으로 옮기기 (머리줄을 끌어 그룹이나 그 옆에 놓아도 된다)"
                  onMouseDown={keepFocus}
                  onClick={() => dispatch({ type: "panelToEditor" })}
                >
                  탭으로
                </button>
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
              <div className="panel-views" ref={containerRef("panel")} />
            </section>
          )}
        </div>
      </div>
      <StatusBar
        cursors={cursors}
        statusKey={statusEntry?.key ?? null}
        repoState={repoState}
        onGraph={openGraph}
        onGotoLine={gotoLine}
      />
      <HostPlacer hosts={hosts} placements={placements} containers={containers} cursors={cursors} />
      {portals.map((portal) => portal.element)}
      {help && <Help view="workspace" onClose={closeHelp} />}
      {quickOpen.overlay}
      {palette.overlay}
      <DragLayer store={drags} />
    </div>
  );
}
