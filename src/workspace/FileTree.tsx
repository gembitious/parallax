// 파일 트리(왼쪽): 저장소의 추적 파일을 폴더별로 보인다(인덱스, 백엔드 files_dir. Quick Open과 같은 목록). 파일을
// 누르면 미리보기 탭, 두 번 누르거나 Enter면 보통 탭, Mod+Enter면 옆(오른쪽) 에디터 그룹에 연다. 우클릭은 열기·옆에
// 열기·히스토리·에디터로 열기·경로 복사.
// 보이는 탭의 파일은 펼쳐서 고른다. 저장소가 바뀌거나(HEAD·참조) 창으로 돌아오면 펼친 폴더를 다시 읽는다(git add로
// 늘어난 파일).
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { openInEditor } from "../editor";
import { call } from "../ipc";
import { isComposing } from "../keys";
import { openContextMenu, type MenuEntry } from "../menu/Menu";
import { drawWindow, insideDrawn, type DrawnRange } from "../scrollWindow";
import { toast } from "../Toasts";
import { watchRepo } from "../watch";
import { MOD } from "./keys";
import { absolutePath, copyPath } from "./paths";
import { ancestorsOf, sortListing, treeKey, treeRows, type DirListing, type TreeMove, type TreeRow } from "./tree";

/** 행 높이(px). styles.css의 .tree-row와 같다. */
const ROW_HEIGHT = 22;
const INDENT_PX = 12;

type Props = {
  /** 작업트리 경로 */
  repo: string;
  /** 키를 받는 칸이다 */
  focused: boolean;
  /** 바뀌면 (키를 받는 칸일 때) 포커스를 다시 가져온다: 그 위에 뜬 창(Quick Open 등)이 닫혔다 */
  focusTick: number;
  /** 보이는 탭의 작업트리 파일(저장소 기준). 펼쳐서 고른다. */
  activeFile: string | null;
  onOpen(path: string, missing: boolean, preview: boolean): void;
  /** Mod+Enter·우클릭 메뉴: 옆(오른쪽) 에디터 그룹에 연다 */
  onOpenSide(path: string, missing: boolean): void;
  /** 우클릭 메뉴: 파일(폴더면 그 아래) 히스토리 */
  onHistory(path: string): void;
  /** Esc·q: 에디터(또는 패널)로 돌아간다 */
  onLeave(): void;
};

export function FileTree({ repo, focused, focusTick, activeFile, onOpen, onOpenSide, onHistory, onLeave }: Props) {
  const [listings, setListings] = useState<ReadonlyMap<string, DirListing>>(() => new Map());
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());
  const [selected, setSelected] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [viewport, setViewport] = useState({ top: 0, height: 0 });
  const listRef = useRef<HTMLDivElement>(null);
  const drawnRange = useRef<DrawnRange>({ start: 0, end: 0, height: 0 });
  const rows = useMemo(() => treeRows(listings, expanded), [listings, expanded]);
  const latest = useRef({ rows, listings, expanded, selected });
  latest.current = { rows, listings, expanded, selected };

  // 폴더 목록을 읽어 넣는다. 저장소마다 새 컴포넌트다(작업 공간이 key를 준다).
  const load = (dirs: Iterable<string>) => {
    for (const dir of dirs) {
      call<DirListing>("files_dir", { repo, dir }).then(
        (listing) => {
          setListings((map) => new Map(map).set(dir, sortListing(listing)));
          if (dir === "") setFailed(null);
        },
        (e: unknown) => {
          if (dir === "") setFailed(String(e));
          else setListings((map) => (map.has(dir) ? new Map([...map].filter(([key]) => key !== dir)) : map));
        },
      );
    }
  };
  const refresh = () => load(["", ...latest.current.expanded]);

  useEffect(() => {
    load([""]);
    const stop = watchRepo(repo, refresh);
    window.addEventListener("focus", refresh);
    return () => {
      stop();
      window.removeEventListener("focus", refresh);
    };
    // 저장소마다 새 컴포넌트다. load·refresh는 ref만 읽는다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 보이는 탭의 파일을 펼쳐서 고른다(탐색기의 자동 표시).
  useEffect(() => {
    if (!activeFile) return;
    const dirs = ancestorsOf(activeFile);
    const missing = dirs.filter((dir) => !latest.current.expanded.has(dir));
    if (missing.length > 0) {
      setExpanded((prev) => new Set([...prev, ...missing]));
      load(missing);
    }
    setSelected(activeFile);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeFile]);

  // 키를 받는 칸이 되면(또는 위에 뜬 창이 닫히면) 목록에 포커스를 준다.
  useEffect(() => {
    if (focused) listRef.current?.focus({ preventScroll: true });
  }, [focused, focusTick]);

  // 목록 크기를 따라간다.
  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const update = () => setViewport({ top: list.scrollTop, height: list.clientHeight });
    update();
    const observer = new ResizeObserver(update);
    observer.observe(list);
    return () => observer.disconnect();
  }, []);

  // 고른 행이 보이게 한다(키로 옮겼거나 보이는 탭의 파일을 골랐다).
  const selectedIndex = selected === null ? -1 : rows.findIndex((row) => row.path === selected);
  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list || selectedIndex < 0) return;
    const top = selectedIndex * ROW_HEIGHT;
    if (top < list.scrollTop) list.scrollTop = top;
    else if (top + ROW_HEIGHT > list.scrollTop + list.clientHeight) list.scrollTop = top + ROW_HEIGHT - list.clientHeight;
    else return;
    setViewport({ top: list.scrollTop, height: list.clientHeight });
  }, [selectedIndex]);

  const toggle = (path: string) => {
    const open = latest.current.expanded.has(path);
    setExpanded((prev) => {
      const next = new Set(prev);
      if (open) next.delete(path);
      else next.add(path);
      return next;
    });
    // 펼칠 때마다 다시 읽는다(인덱스가 그대로면 백엔드가 목록을 다시 읽지 않는다).
    if (!open) load([path]);
  };

  const apply = (move: TreeMove) => {
    switch (move.kind) {
      case "select":
        setSelected(move.path);
        break;
      case "toggle":
        setSelected(move.path);
        toggle(move.path);
        break;
      case "open":
        setSelected(move.path);
        onOpen(move.path, move.missing, move.preview);
        break;
    }
  };

  /** 우클릭 메뉴 */
  const rowMenu = (row: TreeRow): MenuEntry[] => {
    const copies: MenuEntry[] = [
      { label: "경로 복사", detail: "저장소 기준", run: () => copyPath(row.path) },
      { label: "전체 경로 복사", run: () => copyPath(absolutePath(repo, row.path)) },
    ];
    if (row.dir) {
      return [
        { label: row.open ? "접기" : "펼치기", run: () => apply({ kind: "toggle", path: row.path }) },
        { label: "폴더 히스토리", detail: "이 아래를 바꾼 커밋", run: () => onHistory(row.path) },
        "separator",
        ...copies,
      ];
    }
    return [
      { label: "열기", keys: "Enter", run: () => apply({ kind: "open", path: row.path, missing: row.missing, preview: false }) },
      { label: "미리보기로 열기", run: () => apply({ kind: "open", path: row.path, missing: row.missing, preview: true }) },
      { label: "옆에 열기", keys: `${MOD}Enter`, detail: "오른쪽 에디터 그룹", run: () => onOpenSide(row.path, row.missing) },
      "separator",
      { label: "파일 히스토리", run: () => onHistory(row.path) },
      {
        label: "에디터로 열기",
        detail: row.missing ? "작업트리에서 지운 파일" : undefined,
        disabled: row.missing,
        run: () => void openInEditor({ repo, path: row.path, rev: null, line: null }).then((message) => toast(message)),
      },
      "separator",
      ...copies,
    ];
  };

  const { start, end } = drawWindow(viewport.top, viewport.height, ROW_HEIGHT, rows.length);
  drawnRange.current = { start, end, height: viewport.height };
  const page = Math.max(1, Math.floor(viewport.height / ROW_HEIGHT) - 1);

  return (
    <div
      className="tree-list"
      ref={listRef}
      tabIndex={0}
      role="tree"
      aria-label="파일"
      aria-activedescendant={selectedIndex >= 0 ? `tree-row-${selectedIndex}` : undefined}
      onScroll={(e) => {
        const list = e.currentTarget;
        if (insideDrawn(list.scrollTop, list.clientHeight, ROW_HEIGHT, latest.current.rows.length, drawnRange.current)) return;
        setViewport({ top: list.scrollTop, height: list.clientHeight });
      }}
      onKeyDown={(e) => {
        if (isComposing(e.nativeEvent) || e.altKey) return;
        if (e.ctrlKey || e.metaKey) {
          // Mod+Enter: 고른 파일을 옆 그룹에
          const row = latest.current.rows.find((r) => r.path === latest.current.selected);
          if (e.key !== "Enter" || e.shiftKey || !row || row.dir) return;
          e.preventDefault();
          onOpenSide(row.path, row.missing);
          return;
        }
        if (e.key === "Escape" || e.key === "q") {
          e.preventDefault();
          onLeave();
          return;
        }
        const move = treeKey(latest.current.rows, latest.current.selected, e.key, page);
        if (!move) return;
        e.preventDefault();
        apply(move);
      }}
    >
      <div className="tree-rows" style={{ height: rows.length * ROW_HEIGHT }}>
        {rows.slice(start, end).map((row, offset) => {
          const i = start + offset;
          const classes = ["tree-row"];
          if (row.dir) classes.push("dir");
          if (row.missing) classes.push("missing");
          if (row.path === selected) classes.push("selected");
          return (
            <div
              key={row.path}
              id={`tree-row-${i}`}
              role="treeitem"
              aria-level={row.depth + 1}
              aria-expanded={row.dir ? row.open : undefined}
              aria-selected={row.path === selected}
              className={classes.join(" ")}
              style={{ top: i * ROW_HEIGHT, height: ROW_HEIGHT, paddingLeft: 8 + row.depth * INDENT_PX }}
              title={row.missing ? `${row.path} (작업트리에서 지움: HEAD 버전을 연다)` : row.path}
              onClick={() =>
                apply(row.dir ? { kind: "toggle", path: row.path } : { kind: "open", path: row.path, missing: row.missing, preview: true })
              }
              onDoubleClick={() => {
                if (!row.dir) onOpen(row.path, row.missing, false);
              }}
              onContextMenu={(e) => {
                setSelected(row.path);
                openContextMenu(e, rowMenu(row), row.name);
              }}
            >
              <span className={`tree-twisty${row.dir ? (row.open ? " open" : " closed") : ""}`} aria-hidden="true" />
              <span className="tree-name">{row.name}</span>
              {row.loading && <span className="tree-note">읽는 중…</span>}
              {row.missing && <span className="tree-note">지움</span>}
            </div>
          );
        })}
      </div>
      {failed && (
        <div className="tree-message error">
          <p>{failed}</p>
        </div>
      )}
      {!failed && listings.has("") && rows.length === 0 && (
        <div className="tree-message">
          <p>추적하는 파일이 없다.</p>
        </div>
      )}
    </div>
  );
}
