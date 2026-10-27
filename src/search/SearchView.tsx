// 저장소 검색(M7-3): `git grep`을 흘려보내 파일마다 맞은 줄을 보인다. 작업트리(추적하는 파일) 또는 리비전 하나.
// 치는 대로(잠깐 멈추면) 다시 찾고, 맞은 줄을 고르면(Enter·두 번 누르기) 그 줄의 blame 탭을 연다.
// 키(목록): j/k 맞은 줄, J/K(또는 n/N) 다음·이전 파일, Enter blame, Space·←/→ 파일 접기·펴기, H 파일 히스토리,
//     o 에디터로 열기, y 경로:줄 복사, / 검색 칸으로, q 뒤로, ? 도움말. 검색 칸: Enter·↓ 목록으로, Esc 목록으로.
// 백엔드: plx-core/src/grep.rs, src-tauri/src/git.rs grep_open.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { formatCount, shortSha } from "../blame/format";
import { Help } from "../Help";
import { toggleHud } from "../Hud";
import { isComposing, routeKey } from "../keys";
import { copyItem, openContextMenu, selectedText, type MenuEntry } from "../menu/Menu";
import { usePublish, type ViewSlot } from "../menu/registry";
import { lastName, type Navigation } from "../nav";
import { commonCommands, usePalette, type Command } from "../palette/Palette";
import { getPerfInfo, nowMs, record } from "../perf";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { drawWindow, insideDrawn, type DrawnRange } from "../scrollWindow";
import { openStream, type StreamHandle } from "../stream";
import { openInEditor } from "../editor";
import { toast } from "../Toasts";
import { absolutePath, copyPath } from "../workspace/paths";
import { appendFiles, buildRows, findRow, nextFileRow, nextLineRow, splitPaths, splitRanges, type Row } from "./rows";
import type { GrepEvent, GrepFile, GrepQuery } from "./types";

type Props = {
  /** 저장소 안의 아무 경로 */
  repo: string;
  /** 찾을 리비전(없으면 작업트리) */
  rev: string | null;
  /** 처음 찾을 글 */
  query?: string;
  visible: boolean;
  active: boolean;
  slot: ViewSlot | null;
  depth: number;
  onShown?(committedMs: number): void;
  onBack(): void;
  onNavigate(navigation: Navigation): void;
};

type Options = { caseSensitive: boolean; word: boolean; regex: boolean };

type Progress =
  | { kind: "idle" }
  | { kind: "searching"; started: number }
  | { kind: "done"; matches: number; files: number; truncated: boolean; totalMs: number }
  | { kind: "failed"; message: string };

const ROW_HEIGHT = 22;
/** 치다가 멈춘 뒤 찾기까지 */
const DEBOUNCE_MS = 250;
const OPTIONS_KEY = "plx.search.options";

function readOptions(): Options {
  try {
    const saved = JSON.parse(localStorage.getItem(OPTIONS_KEY) ?? "null") as Partial<Options> | null;
    return { caseSensitive: !!saved?.caseSensitive, word: !!saved?.word, regex: !!saved?.regex };
  } catch {
    return { caseSensitive: false, word: false, regex: false };
  }
}

function saveOptions(options: Options) {
  try {
    localStorage.setItem(OPTIONS_KEY, JSON.stringify(options));
  } catch {
    // 남기지 못해도 이번 화면에는 그대로다.
  }
}

export function SearchView({ repo, rev, query = "", visible, active, slot, depth, onShown, onBack, onNavigate }: Props) {
  const [pattern, setPattern] = useState(query);
  const [paths, setPaths] = useState("");
  const [revText, setRevText] = useState(rev ?? "");
  const [options, setOptions] = useState<Options>(readOptions);
  const [files, setFiles] = useState<GrepFile[]>([]);
  /** 찾은 저장소(작업트리)와 푼 리비전 */
  const [meta, setMeta] = useState<{ repo: string; rev: string | null } | null>(null);
  const [progress, setProgress] = useState<Progress>({ kind: "idle" });
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [selected, setSelected] = useState(0);
  const [help, setHelp] = useState(false);
  const [viewport, setViewport] = useState({ top: 0, height: 0 });
  const drawnRange = useRef<DrawnRange>({ start: 0, end: 0, height: 0 });
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const stream = useRef<StreamHandle | null>(null);
  const shown = useRef(false);
  const savedTop = useRef(0);

  const rows = useMemo(() => buildRows(files, collapsed), [files, collapsed]);
  const quickOpen = useQuickOpen({
    repo,
    current: null,
    onNavigate,
    onClosed: () => listRef.current?.focus({ preventScroll: true }),
  });

  // 찾기: 글·경로·리비전·선택이 바뀌면 잠깐 기다렸다 앞 검색을 멈추고 새로 찾는다.
  const grepQuery: GrepQuery = {
    pattern,
    rev: revText.trim() || null,
    paths: splitPaths(paths),
    ...options,
  };
  const queryKey = JSON.stringify(grepQuery);
  useEffect(() => {
    stream.current?.cancel();
    stream.current = null;
    if (pattern === "") {
      setFiles([]);
      setProgress({ kind: "idle" });
      return;
    }
    const timer = window.setTimeout(() => {
      const started = nowMs();
      setProgress({ kind: "searching", started });
      let first = true;
      let pending: GrepFile[] = [];
      const handle = openStream<GrepEvent>("grep_open", "grep_cancel", { repo, query: grepQuery }, (event) => {
        if (stream.current !== handle) return;
        switch (event.kind) {
          case "meta":
            setMeta({ repo: event.repo, rev: event.rev });
            break;
          case "files":
            pending = appendFiles(first ? [] : pending, event.files);
            if (first) {
              first = false;
              setSelected(0);
              setCollapsed(new Set());
            }
            setFiles(pending);
            break;
          case "done":
            if (first) setFiles([]);
            setProgress({ kind: "done", matches: event.matches, files: event.files, truncated: event.truncated, totalMs: event.totalMs });
            record({
              kind: "search",
              label: `검색 ${lastName(repo)} "${pattern}"`,
              totalMs: nowMs() - started,
              phases: [
                { label: "첫 결과", ms: event.firstMs },
                { label: "git grep 끝", ms: event.totalMs },
              ],
              ok: true,
            });
            break;
          case "error":
            if (first) setFiles([]);
            setProgress({ kind: "failed", message: event.message });
            break;
          case "cancelled":
            break;
        }
      });
      stream.current = handle;
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queryKey, repo]);
  useEffect(() => () => stream.current?.cancel(), []);

  useEffect(() => {
    if (!shown.current) {
      shown.current = true;
      onShown?.(nowMs());
    }
  });

  // 처음 열면 검색 칸에 포커스(바로 친다)
  useEffect(() => {
    if (active) (pattern === "" ? inputRef.current : (listRef.current ?? inputRef.current))?.focus({ preventScroll: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  // 숨었다 다시 보이면 스크롤 자리를 되돌린다.
  useLayoutEffect(() => {
    if (visible && listRef.current) listRef.current.scrollTop = savedTop.current;
  }, [visible]);

  const current = (index = latest.current.selected) => {
    const row = latest.current.rows[index];
    if (!row) return null;
    const file = latest.current.files[row.file];
    return { row, file, line: row.kind === "line" ? file.lines[row.line] : null };
  };
  /** 결과의 리비전(작업트리면 null): 찾을 때 푼 커밋 */
  const resultRev = () => latest.current.meta?.rev ?? null;

  const select = (index: number) => {
    const count = latest.current.rows.length;
    if (count === 0) return;
    const next = Math.max(0, Math.min(count - 1, index));
    setSelected(next);
    const list = listRef.current;
    if (!list) return;
    const top = next * ROW_HEIGHT;
    if (top < list.scrollTop) list.scrollTop = top;
    else if (top + ROW_HEIGHT > list.scrollTop + list.clientHeight) list.scrollTop = top + ROW_HEIGHT - list.clientHeight;
  };

  const openBlame = (index?: number, side = false) => {
    const at = current(index);
    if (!at) return;
    if (!at.line) {
      toggleFile(index);
      return;
    }
    onNavigate({
      kind: "file",
      target: { repo: latest.current.meta?.repo ?? repo, path: at.file.path, rev: resultRev() },
      line: at.line.line,
      side,
    });
  };
  const openHistory = (index?: number) => {
    const at = current(index);
    if (!at) return;
    onNavigate({ kind: "history", target: { repo: latest.current.meta?.repo ?? repo, path: at.file.path, rev: resultRev() } });
  };
  const openEditor = (index?: number) => {
    const at = current(index);
    if (!at) return;
    void openInEditor({ repo, path: at.file.path, rev: resultRev(), line: at.line?.line ?? null }).then((m) => toast(m));
  };
  const copyLocation = (index?: number) => {
    const at = current(index);
    if (at) copyPath(at.line ? `${at.file.path}:${at.line.line}` : at.file.path);
  };
  const toggleFile = (index?: number, open?: boolean) => {
    const at = current(index);
    if (!at) return;
    const path = at.file.path;
    const set = new Set(latest.current.collapsed);
    const isCollapsed = set.has(path);
    if (open === undefined ? isCollapsed : open) set.delete(path);
    else set.add(path);
    if (set.has(path) === isCollapsed) return;
    // 접으면 그 파일 머리로 커서를 옮긴다.
    const nextRows = buildRows(latest.current.files, set);
    setCollapsed(set);
    setSelected(findRow(nextRows, latest.current.files, path, set.has(path) ? null : (at.line?.line ?? null)));
  };
  const setOption = (key: keyof Options) =>
    setOptions((o) => {
      const next = { ...o, [key]: !o[key] };
      saveOptions(next);
      return next;
    });
  const focusInput = () => {
    inputRef.current?.focus();
    inputRef.current?.select();
  };

  const optionCommands: Command[] = [
    { label: options.caseSensitive ? "대소문자 가리지 않기" : "대소문자 가리기", keys: "Alt+C", run: () => setOption("caseSensitive") },
    { label: options.word ? "낱말로만 찾지 않기" : "낱말로만 찾기", keys: "Alt+W", run: () => setOption("word") },
    { label: options.regex ? "글자 그대로 찾기" : "정규식으로 찾기", keys: "Alt+R", run: () => setOption("regex") },
  ];
  const commands: Command[] = [
    { id: "blame", label: "이 줄의 blame", keys: "Enter", run: () => openBlame() },
    { label: "옆에 blame 열기", keys: "Ctrl+Enter", run: () => openBlame(undefined, true) },
    { id: "history", label: "이 파일의 히스토리", keys: "H", run: () => openHistory() },
    { label: "다음 파일", keys: "J · n", run: () => select(nextFileRow(latest.current.rows, latest.current.selected, 1) ?? latest.current.selected) },
    { label: "이전 파일", keys: "K · N", run: () => select(nextFileRow(latest.current.rows, latest.current.selected, -1) ?? latest.current.selected) },
    { label: "파일 접기·펴기", keys: "Space", run: () => toggleFile() },
    { label: "모든 파일 접기", run: () => setCollapsed(new Set(latest.current.files.map((f) => f.path))) },
    { label: "모든 파일 펴기", run: () => setCollapsed(new Set()) },
    { label: "검색 칸으로", keys: "/", run: focusInput },
    ...optionCommands,
    { label: "경로:줄 복사", keys: "y", run: () => copyLocation() },
    { label: "에디터로 열기", keys: "o", detail: "그 줄(작업트리)", run: () => openEditor() },
    ...commonCommands({
      files: quickOpen.open,
      worktrees: () => {},
      help: () => setHelp(true),
      back: onBack,
      hud: getPerfInfo().enabled ? toggleHud : undefined,
    }).filter((command) => command.keys !== "W"),
  ];
  const palette = usePalette(commands, () => listRef.current?.focus({ preventScroll: true }));
  usePublish(slot, visible, "검색", commands, { palette: palette.open, help: () => setHelp(true), files: quickOpen.open });

  const latest = useRef({ rows, files, selected, collapsed, meta, active, help, palette: palette.open, openFiles: quickOpen.open });
  latest.current = { rows, files, selected, collapsed, meta, active, help, palette: palette.open, openFiles: quickOpen.open };

  // 목록의 키(화면이 키를 받을 때만, 검색 칸은 따로)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { active: on, openFiles, palette: openPalette, help: helpOpen, rows: list, selected: at } = latest.current;
      if (!on) return;
      // 선택 바꾸기(Alt+C·W·R)는 검색 칸 안에서도 받는다.
      if (e.altKey && !e.ctrlKey && !e.metaKey && !isComposing(e)) {
        const key = e.code === "KeyC" ? "caseSensitive" : e.code === "KeyW" ? "word" : e.code === "KeyR" ? "regex" : null;
        if (key) {
          e.preventDefault();
          setOption(key);
          return;
        }
      }
      // Ctrl·⌘+Enter: 옆 그룹에 blame(목록에서만)
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !(e.target as Element | null)?.closest?.("input")) {
        e.preventDefault();
        openBlame(undefined, true);
        return;
      }
      if (routeKey(e, { files: openFiles, palette: openPalette }) !== "view") return;
      if (isComposing(e)) return;
      let handled = true;
      switch (e.key) {
        case "j":
        case "ArrowDown":
          select(nextLineRow(list, at, 1) ?? at + 1);
          break;
        case "k":
        case "ArrowUp":
          select(nextLineRow(list, at, -1) ?? at - 1);
          break;
        case "J":
        case "n":
          select(nextFileRow(list, at, 1) ?? at);
          break;
        case "K":
        case "N":
          select(nextFileRow(list, at, -1) ?? at);
          break;
        case "PageDown":
          select(at + Math.max(1, Math.floor((listRef.current?.clientHeight ?? ROW_HEIGHT) / ROW_HEIGHT) - 1));
          break;
        case "PageUp":
          select(at - Math.max(1, Math.floor((listRef.current?.clientHeight ?? ROW_HEIGHT) / ROW_HEIGHT) - 1));
          break;
        case "Home":
          select(0);
          break;
        case "End":
          select(list.length - 1);
          break;
        case "Enter":
          openBlame();
          break;
        case " ":
          toggleFile();
          break;
        case "ArrowLeft":
        case "h":
          toggleFile(undefined, false);
          break;
        case "ArrowRight":
        case "l":
          toggleFile(undefined, true);
          break;
        case "H":
          openHistory();
          break;
        case "o":
          openEditor();
          break;
        case "y":
          copyLocation();
          break;
        case "/":
          focusInput();
          break;
        case "q":
          onBack();
          break;
        case "?":
          setHelp(true);
          break;
        case "Escape":
          if (helpOpen) setHelp(false);
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
    const at = current(index);
    if (!at) return [];
    const where = at.line ? `${at.file.path}:${at.line.line}` : at.file.path;
    return [
      { header: where },
      ...(at.line
        ? [
            { label: "이 줄의 blame", keys: "Enter", run: () => openBlame(index) },
            { label: "옆에 blame 열기", keys: "Ctrl+Enter", run: () => openBlame(index, true) },
          ]
        : [{ label: latest.current.collapsed.has(at.file.path) ? "펴기" : "접기", keys: "Space", run: () => toggleFile(index) }]),
      { label: "파일 히스토리", keys: "H", run: () => openHistory(index) },
      "separator",
      { label: "경로:줄 복사", keys: "y", run: () => copyLocation(index) },
      { label: "전체 경로 복사", run: () => copyPath(absolutePath(latest.current.meta?.repo ?? repo, at.file.path)) },
      { label: "에디터로 열기", keys: "o", run: () => openEditor(index) },
    ];
  };

  const count = rows.length;
  const { start, end } = drawWindow(viewport.top, viewport.height, ROW_HEIGHT, count);
  useLayoutEffect(() => {
    drawnRange.current = { start, end, height: viewport.height };
  });
  useLayoutEffect(() => {
    const list = listRef.current;
    if (list) setViewport({ top: list.scrollTop, height: list.clientHeight });
  }, [count]);

  const revLabel = meta?.rev ? shortSha(meta.rev) : revText.trim() ? revText.trim() : "작업트리";
  const status =
    progress.kind === "searching"
      ? files.length > 0
        ? `찾는 중… 파일 ${formatCount(files.length)}개`
        : "찾는 중…"
      : progress.kind === "done"
        ? progress.matches === 0
          ? "맞는 줄이 없다"
          : `${formatCount(progress.matches)}줄 · 파일 ${formatCount(progress.files)}개${progress.truncated ? " (앞의 것만)" : ""}`
        : null;

  return (
    <section className="search-view">
      <header
        className="blame-header"
        onContextMenu={(e) => openContextMenu(e, [copyItem(selectedText(e.target)), "separator", ...optionCommands], "검색")}
      >
        <span className="badge">검색</span>
        <span className="file-path mono" title={meta?.repo ?? repo}>
          <span className="file-name">{lastName(meta?.repo ?? repo)}</span>
        </span>
        <span className={`badge mono${meta?.rev || revText.trim() ? " warn" : ""}`} title="찾는 리비전(비우면 작업트리의 추적하는 파일)">
          {revLabel}
        </span>
        {depth > 1 && (
          <span className="badge" title="q로 앞 화면으로 돌아간다">
            뒤로 q · {depth - 1}
          </span>
        )}
        <span className="blame-status" role="status">
          {status}
        </span>
        <button type="button" className="icon" onClick={() => setHelp(true)} aria-label="단축키 (?)" title="단축키 (?)">
          ?
        </button>
      </header>
      <div className="search-bar">
        <input
          ref={inputRef}
          className="search-input"
          type="search"
          value={pattern}
          placeholder="저장소에서 찾기 (Enter 목록으로)"
          aria-label="찾을 글"
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => setPattern(e.target.value)}
          onKeyDown={(e) => {
            if (isComposing(e.nativeEvent)) return;
            if (e.key === "Enter" || e.key === "ArrowDown") {
              e.preventDefault();
              listRef.current?.focus({ preventScroll: true });
              select(nextLineRow(latest.current.rows, -1, 1) ?? 0);
            } else if (e.key === "Escape") {
              e.preventDefault();
              listRef.current?.focus({ preventScroll: true });
            }
          }}
        />
        <span className="segmented" role="group" aria-label="찾는 방법">
          {(
            [
              ["caseSensitive", "Aa", "대소문자 가리기 (Alt+C)"],
              ["word", "ab", "낱말로만 (Alt+W)"],
              ["regex", ".*", "정규식 (Alt+R, git의 확장 정규식)"],
            ] as const
          ).map(([key, label, title]) => (
            <button
              key={key}
              type="button"
              className={options[key] ? "toggle primary mono" : "toggle mono"}
              aria-pressed={options[key]}
              title={title}
              onClick={() => setOption(key)}
            >
              {label}
            </button>
          ))}
        </span>
        <input
          className="search-paths"
          value={paths}
          placeholder="경로 (src/ *.ts !*.lock)"
          aria-label="찾을 경로"
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => setPaths(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && listRef.current?.focus({ preventScroll: true })}
        />
        <input
          className="search-rev mono"
          value={revText}
          placeholder="리비전 (비우면 작업트리)"
          aria-label="찾을 리비전"
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => setRevText(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && listRef.current?.focus({ preventScroll: true })}
        />
      </div>
      <div
        className="search-list"
        ref={listRef}
        tabIndex={0}
        role="listbox"
        aria-label="검색 결과"
        onScroll={(e) => {
          const list = e.currentTarget;
          if (visible) savedTop.current = list.scrollTop;
          if (insideDrawn(list.scrollTop, list.clientHeight, ROW_HEIGHT, count, drawnRange.current)) return;
          setViewport({ top: list.scrollTop, height: list.clientHeight });
        }}
      >
        <div className="search-rows" style={{ height: count * ROW_HEIGHT }}>
          {rows.slice(start, end).map((row, offset) => {
            const i = start + offset;
            return (
              <ResultRow
                key={row.kind === "file" ? `f:${files[row.file].path}` : `l:${files[row.file].path}:${row.line}`}
                row={row}
                files={files}
                collapsed={collapsed}
                selected={i === selected}
                top={i * ROW_HEIGHT}
                onClick={() => {
                  setSelected(i);
                  if (row.kind === "file") toggleFile(i);
                }}
                onDoubleClick={() => row.kind === "line" && openBlame(i)}
                onContextMenu={(e) => {
                  setSelected(i);
                  openContextMenu(e, rowMenu(i), "검색 결과");
                }}
              />
            );
          })}
        </div>
        {progress.kind === "failed" && (
          <div className="blame-message error">
            <p>{progress.message}</p>
          </div>
        )}
        {progress.kind === "idle" && (
          <div className="blame-message">
            <p className="muted">
              찾을 글을 치면 {revLabel === "작업트리" ? "작업트리의 추적하는 파일" : `${revLabel}의 파일`}에서 찾는다.
            </p>
          </div>
        )}
      </div>
      {help && <Help view="search" onClose={() => setHelp(false)} />}
      {quickOpen.overlay}
      {palette.overlay}
    </section>
  );
}

function ResultRow({
  row,
  files,
  collapsed,
  selected,
  top,
  onClick,
  onDoubleClick,
  onContextMenu,
}: {
  row: Row;
  files: readonly GrepFile[];
  collapsed: ReadonlySet<string>;
  selected: boolean;
  top: number;
  onClick(): void;
  onDoubleClick(): void;
  onContextMenu(e: React.MouseEvent): void;
}) {
  const file = files[row.file];
  const style = { top, height: ROW_HEIGHT };
  const className = `search-row ${row.kind === "file" ? "search-file" : "search-line"}${selected ? " selected" : ""}`;
  if (row.kind === "file") {
    const slash = file.path.lastIndexOf("/");
    const shut = collapsed.has(file.path);
    return (
      <div className={className} style={style} role="option" aria-selected={selected} aria-expanded={!shut} onClick={onClick} onContextMenu={onContextMenu}>
        <span className="tree-chevron">{shut ? "▸" : "▾"}</span>
        <span className="search-file-name">{file.path.slice(slash + 1)}</span>
        {slash > 0 && <span className="muted search-file-dir">{file.path.slice(0, slash)}</span>}
        <span className="badge search-count">{file.lines.length}</span>
      </div>
    );
  }
  const line = file.lines[row.line];
  return (
    <div
      className={className}
      style={style}
      role="option"
      aria-selected={selected}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      <span className="search-line-no mono">{line.line}</span>
      <span className="search-text mono">
        {line.cutStart && <span className="muted">…</span>}
        {splitRanges(line.text, line.ranges).map((part, k) =>
          part.hit ? (
            <mark key={k} className="search-hit">
              {part.text}
            </mark>
          ) : (
            <span key={k}>{part.text}</span>
          ),
        )}
        {line.cutEnd && <span className="muted">…</span>}
      </span>
    </div>
  );
}
