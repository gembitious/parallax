// reflog·stash 목록(M10): HEAD가 거쳐 간 커밋(이 워크트리의 `git reflog`)과 stash(`git stash list`)를 최근 것부터 보인다.
// 리셋·리베이스로 브랜치에서 떨어진 커밋을 찾아 커밋 상세·작업트리 비교로 본다. 읽기만 한다(되돌리기·stash 꺼내기는 없다).
// 키: j/k 이동, Enter 커밋 상세(stash면 stash한 변경), u stash의 추적하지 않던 파일, d 작업트리와 비교, G 그래프,
//     T 그 커밋의 파일 트리, F 그 커밋의 파일에서 찾기, y sha 복사, s HEAD reflog ↔ stash, / 거르기, q 뒤로, ? 도움말.
// 백엔드: plx-core/src/reflog.rs, src-tauri/src/git.rs reflog. HEAD·참조가 바뀌면(커밋, 체크아웃, stash) 다시 읽는다.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { formatCount, formatDate, relativeTime, shortSha } from "../blame/format";
import { copyText } from "../clipboard";
import { Help } from "../Help";
import { toggleHud } from "../Hud";
import { call } from "../ipc";
import { isComposing, routeKey } from "../keys";
import { copyItem, openContextMenu, selectedText, type MenuEntry } from "../menu/Menu";
import { usePublish, type ViewSlot } from "../menu/registry";
import { lastName, type Navigation, type ReflogWhich } from "../nav";
import { commonCommands, usePalette, type Command } from "../palette/Palette";
import { getPerfInfo, nowMs, record } from "../perf";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { drawWindow, insideDrawn, type DrawnRange } from "../scrollWindow";
import { toast } from "../Toasts";
import { watchRepo } from "../watch";
import { keepSelection, matchesQuery, refName, type Reflog, type ReflogItem } from "./types";

type Props = {
  /** 저장소 안의 아무 경로 */
  repo: string;
  /** 처음 보일 목록 */
  which: ReflogWhich;
  visible: boolean;
  active: boolean;
  slot: ViewSlot | null;
  depth: number;
  onShown?(committedMs: number): void;
  onBack(): void;
  onNavigate(navigation: Navigation): void;
};

type Load = { kind: "loading" } | { kind: "done"; list: Reflog } | { kind: "failed"; message: string };

const ROW_HEIGHT = 24;

export function ReflogView({ repo, which: initial, visible, active, slot, depth, onShown, onBack, onNavigate }: Props) {
  const [which, setWhich] = useState<ReflogWhich>(initial);
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [selected, setSelected] = useState(0);
  const [query, setQuery] = useState("");
  const [filtering, setFiltering] = useState(false);
  const [help, setHelp] = useState(false);
  const [viewport, setViewport] = useState({ top: 0, height: 0 });
  /** 숨어 있는 동안 저장소가 바뀌었다(보이면 다시 읽는다) */
  const [stale, setStale] = useState(false);
  const [version, setVersion] = useState(0);
  const drawnRange = useRef<DrawnRange>({ start: 0, end: 0, height: 0 });
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const shown = useRef(false);
  const savedTop = useRef(0);
  const nowAt = useRef(Date.now());

  const entries = load.kind === "done" ? load.list.entries : [];
  const rows = useMemo(() => entries.filter((item) => matchesQuery(item, which, query)), [entries, which, query]);

  // 읽기: 목록을 바꾸거나 저장소가 바뀌면 다시. 고른 항목은 내용으로 이어 간다(새 항목이 위에 붙는다).
  useEffect(() => {
    let stale = false;
    const started = nowMs();
    const before = latest.current.rows[latest.current.selected];
    const index = latest.current.selected;
    const sameList = latest.current.load.kind === "done" && latest.current.load.list.which === which;
    call<Reflog>("reflog", { repo, which }).then(
      (list) => {
        if (stale) return;
        nowAt.current = Date.now();
        setLoad({ kind: "done", list });
        const next = list.entries.filter((item) => matchesQuery(item, which, latest.current.query));
        setSelected(sameList ? keepSelection(before, next, index) : 0);
        record({
          kind: "reflog",
          label: `${which === "stash" ? "stash" : "reflog"} ${lastName(repo)}`,
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
  }, [repo, which, version]);

  // HEAD·참조가 바뀌면(커밋, 체크아웃, stash push·drop) 다시 읽는다. 숨어 있으면 보일 때.
  useEffect(
    () =>
      watchRepo(repo, () => {
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

  const current = (index = latest.current.selected): ReflogItem | null => latest.current.rows[index] ?? null;
  const repoOf = () => (latest.current.load.kind === "done" ? latest.current.load.list.repo : repo);
  /** 커밋이 있는 항목(없으면 알린다) */
  const existing = (index?: number): ReflogItem | null => {
    const item = current(index);
    if (item && !item.exists) {
      toast(`커밋이 없다(gc로 지웠다): ${shortSha(item.sha)}`);
      return null;
    }
    return item;
  };

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

  const openCommit = (index?: number) => {
    const item = existing(index);
    if (item) onNavigate({ kind: "commit", repo: repoOf(), rev: item.sha, path: null });
  };
  const openUntracked = (index?: number) => {
    const item = existing(index);
    if (!item) return;
    if (item.untracked) onNavigate({ kind: "commit", repo: repoOf(), rev: item.untracked, path: null });
    else toast("이 stash에는 추적하지 않던 파일이 없다 (stash -u로 넣는다)");
  };
  const compareWorktree = (index?: number) => {
    const item = existing(index);
    if (item) {
      const compare = { base: item.sha, target: null, mergeBase: false };
      onNavigate({ kind: "commit", repo: repoOf(), rev: item.sha, path: null, compare });
    }
  };
  const openGraph = (index?: number) => {
    const item = existing(index);
    if (item) onNavigate({ kind: "graph", repo: repoOf(), rev: item.sha });
  };
  const browse = (index?: number) => {
    const item = existing(index);
    if (item) onNavigate({ kind: "browse", repo: repoOf(), rev: item.sha });
  };
  const searchIn = (index?: number) => {
    const item = existing(index);
    if (item) onNavigate({ kind: "search", repo: repoOf(), rev: item.sha });
  };
  const copy = (text: string, what: string) =>
    copyText(text).then(
      () => toast(`복사했다: ${what}`),
      (e: unknown) => toast(String(e)),
    );
  const copySha = (index?: number) => {
    const item = current(index);
    if (item) void copy(item.sha, shortSha(item.sha));
  };
  const copyName = (index?: number) => {
    const item = current(index);
    if (item) void copy(refName(latest.current.which, item.index), refName(latest.current.which, item.index));
  };
  const switchList = (next?: ReflogWhich) => {
    const to = next ?? (latest.current.which === "head" ? "stash" : "head");
    if (to === latest.current.which) return;
    setWhich(to);
    setLoad({ kind: "loading" });
    setSelected(0);
    savedTop.current = 0;
    if (listRef.current) listRef.current.scrollTop = 0;
  };
  const openFilter = () => {
    setFiltering(true);
    window.setTimeout(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
  };
  const closeFilter = (clear: boolean) => {
    if (clear) {
      // 거르기를 지워도 고른 항목은 그대로
      const item = current();
      setQuery("");
      if (item) setSelected(Math.max(0, latest.current.entries.indexOf(item)));
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

  const isStash = which === "stash";
  const commands: Command[] = [
    { id: "commit", label: isStash ? "stash한 변경 보기" : "커밋 상세", keys: "Enter", run: () => openCommit() },
    ...(isStash
      ? [{ label: "추적하지 않던 파일 보기", keys: "u", detail: "stash -u", run: () => openUntracked() }]
      : []),
    { label: "작업트리와 비교", keys: "d", run: () => compareWorktree() },
    { id: "graph", label: "그래프에서 보기", keys: "G", run: () => openGraph() },
    { label: "이 커밋의 파일 트리", keys: "T", detail: "리비전 둘러보기", run: () => browse() },
    { label: "이 커밋의 파일에서 찾기", keys: "F", detail: "git grep (아래 패널)", run: () => searchIn() },
    { label: "커밋 sha 복사", keys: "y", run: () => copySha() },
    { label: `이름 복사 (${isStash ? "stash" : "HEAD"}@{n})`, run: () => copyName() },
    { label: isStash ? "HEAD reflog로" : "stash 목록으로", keys: "s", run: () => switchList() },
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
  usePublish(slot, visible, isStash ? "stash" : "reflog", commands, {
    palette: palette.open,
    help: () => setHelp(true),
    files: quickOpen.open,
  });

  const latest = useRef({ rows, entries, selected, load, which, query, visible, active, help, palette: palette.open, openFiles: quickOpen.open });
  latest.current = { rows, entries, selected, load, which, query, visible, active, help, palette: palette.open, openFiles: quickOpen.open };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { active: on, openFiles, palette: openPalette, help: helpOpen, rows: list, selected: at } = latest.current;
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
          select(list.length - 1);
          break;
        case "Enter":
          openCommit();
          break;
        case "u":
          if (latest.current.which === "stash") openUntracked();
          else handled = false;
          break;
        case "d":
          compareWorktree();
          break;
        case "G":
          openGraph();
          break;
        case "T":
          browse();
          break;
        case "F":
          searchIn();
          break;
        case "y":
          copySha();
          break;
        case "s":
          switchList();
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
    const item = current(index);
    if (!item) return [];
    const gone = !item.exists;
    const stash = latest.current.which === "stash";
    return [
      { header: `${refName(latest.current.which, item.index)} · ${shortSha(item.sha)}` },
      { label: stash ? "stash한 변경 보기" : "커밋 상세", keys: "Enter", disabled: gone, run: () => openCommit(index) },
      ...(stash
        ? [{ label: "추적하지 않던 파일 보기", keys: "u", disabled: gone || !item.untracked, run: () => openUntracked(index) }]
        : []),
      { label: "작업트리와 비교", keys: "d", disabled: gone, run: () => compareWorktree(index) },
      { label: "그래프에서 보기", keys: "G", disabled: gone, run: () => openGraph(index) },
      { label: "이 커밋의 파일 트리", keys: "T", disabled: gone, run: () => browse(index) },
      { label: "이 커밋의 파일에서 찾기", keys: "F", disabled: gone, run: () => searchIn(index) },
      "separator",
      { label: "커밋 sha 복사", keys: "y", run: () => copySha(index) },
      { label: `이름 복사 (${refName(latest.current.which, item.index)})`, run: () => copyName(index) },
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

  const chosen = rows[selected] ?? null;
  const status =
    load.kind === "loading"
      ? "읽는 중…"
      : load.kind === "done"
        ? query
          ? `${formatCount(rows.length)} / ${formatCount(entries.length)}개`
          : `${formatCount(entries.length)}개${load.list.truncated ? " (앞의 것만)" : ""}`
        : null;

  return (
    <section className="reflog-view">
      <header className="blame-header" onContextMenu={(e) => openContextMenu(e, [copyItem(selectedText(e.target))], "reflog")}>
        <span className="segmented" role="group" aria-label="목록">
          {(
            [
              ["head", "HEAD reflog"],
              ["stash", "stash"],
            ] as const
          ).map(([value, label]) => (
            <button
              key={value}
              type="button"
              className={which === value ? "toggle primary" : "toggle"}
              aria-pressed={which === value}
              title={value === "head" ? "HEAD가 거쳐 간 커밋 (이 워크트리, s로 바꾼다)" : "stash 목록 (s로 바꾼다)"}
              onClick={() => switchList(value)}
            >
              {label}
            </button>
          ))}
        </span>
        <span className="file-path mono" title={repoOf()}>
          <span className="file-name">{lastName(repoOf())}</span>
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
      {(filtering || query) && (
        <div className="history-search">
          <input
            ref={inputRef}
            type="search"
            value={query}
            placeholder="거르기: 이름·sha·종류·메시지 (Enter 목록으로, Esc 지우기)"
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
        aria-label={isStash ? "stash 목록" : "HEAD reflog"}
        aria-activedescendant={chosen ? `reflog-row-${selected}` : undefined}
        onScroll={(e) => {
          const list = e.currentTarget;
          if (visible) savedTop.current = list.scrollTop;
          if (insideDrawn(list.scrollTop, list.clientHeight, ROW_HEIGHT, count, drawnRange.current)) return;
          setViewport({ top: list.scrollTop, height: list.clientHeight });
        }}
      >
        <div className="history-rows" style={{ height: count * ROW_HEIGHT }}>
          {rows.slice(start, end).map((item, offset) => {
            const i = start + offset;
            return (
              <div
                key={`${item.index}:${item.sha}:${item.who.time}`}
                id={`reflog-row-${i}`}
                role="option"
                aria-selected={i === selected}
                className={`history-row reflog-row${i === selected ? " selected" : ""}${item.exists ? "" : " gone"}`}
                style={{ top: i * ROW_HEIGHT, height: ROW_HEIGHT }}
                onClick={() => select(i)}
                onDoubleClick={() => {
                  select(i);
                  openCommit(i);
                }}
                onContextMenu={(e) => {
                  select(i);
                  openContextMenu(e, rowMenu(i), isStash ? "stash" : "reflog");
                }}
              >
                <Row item={item} which={which} nowMs={nowAt.current} />
              </div>
            );
          })}
        </div>
        {load.kind === "done" && entries.length === 0 && (
          <div className="blame-message">
            <p className="muted">
              {isStash ? "stash가 없다. git stash로 넣은 변경이 여기 보인다." : "reflog가 없다(core.logAllRefUpdates가 꺼져 있다)."}
            </p>
          </div>
        )}
        {load.kind === "done" && entries.length > 0 && rows.length === 0 && (
          <div className="blame-message">
            <p className="muted">맞는 항목이 없다.</p>
          </div>
        )}
        {load.kind === "failed" && (
          <div className="blame-message error">
            <p>{load.message}</p>
          </div>
        )}
      </div>
      {chosen && (
        <footer className="history-detail" onContextMenu={(e) => openContextMenu(e, rowMenu(selected), isStash ? "stash" : "reflog")}>
          <span className="mono" title="y로 복사">
            {chosen.sha}
          </span>
          {chosen.previous && chosen.previous !== chosen.sha && (
            <span className="muted mono" title="그 일 앞에 가리킨 커밋">
              ← {shortSha(chosen.previous)}
            </span>
          )}
          <span>
            {chosen.who.name} <span className="muted">&lt;{chosen.who.email}&gt;</span>
          </span>
          <span className="muted">{formatDate(chosen.who.time, chosen.who.tzOffset)}</span>
          {chosen.untracked && <span className="badge">추적하지 않던 파일 · u</span>}
          {!chosen.exists && <span className="badge warn">커밋 없음</span>}
        </footer>
      )}
      {help && <Help view="reflog" onClose={() => setHelp(false)} />}
      {quickOpen.overlay}
      {palette.overlay}
    </section>
  );
}

function Row({ item, which, nowMs: now }: { item: ReflogItem; which: ReflogWhich; nowMs: number }) {
  // 커밋을 만든 일(commit, merge …)은 메시지가 곧 요약이라 한 번만 보인다.
  const extra = item.summary && item.summary !== item.message && which === "head" ? item.summary : null;
  return (
    <>
      <span className="reflog-name mono">{refName(which, item.index)}</span>
      <span className="history-sha mono">{shortSha(item.sha)}</span>
      {item.action && <span className={`badge reflog-action reflog-${actionKind(item.action)}`}>{item.action}</span>}
      <span className="history-summary">
        {item.message}
        {extra && <span className="muted"> · {extra}</span>}
        {!item.exists && <span className="muted"> (커밋 없음)</span>}
      </span>
      <span className="history-time" title={formatDate(item.who.time, item.who.tzOffset)}>
        {relativeTime(item.who.time, now)}
      </span>
    </>
  );
}

/** 종류 이름표의 색: 커밋을 만든 일, 옮긴 일, 되돌린 일 */
export function actionKind(action: string): "commit" | "move" | "reset" | "other" {
  const word = action.split(/[\s(]/)[0];
  if (word === "commit" || word === "merge" || word === "cherry-pick" || word === "revert") return "commit";
  if (word === "checkout" || word === "switch" || word === "pull" || word === "rebase" || word === "clone") return "move";
  if (word === "reset") return "reset";
  return "other";
}
