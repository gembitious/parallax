// 브랜치·태그 목록(M14): 로컬 브랜치(upstream 대비 앞섬·뒤처짐, 체크아웃한 워크트리), 원격 브랜치, 태그를 최근 커밋부터
// 보인다. 에이전트가 만든 브랜치가 무엇을 바꿨는지(갈라진 지점부터의 diff), 원격과 얼마나 어긋났는지를 그래프를 훑지
// 않고 본다. 읽기만 한다(체크아웃·지우기·푸시는 없다).
// 키: j/k 이동, Enter 커밋 상세, G 그래프, d 지금 HEAD에서 갈라진 뒤 바꾼 것, u upstream에서 갈라진 뒤 바꾼 것,
//     w 작업트리와 비교, T 파일 트리, F 찾기, y sha 복사, s 브랜치 → 원격 → 태그, / 거르기, q 뒤로, ? 도움말.
// 백엔드: plx-core/src/refs.rs, src-tauri/src/git.rs refs_list·ref_tracking. HEAD·참조가 바뀌면 다시 읽는다.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { formatCount, formatDate, relativeTime, shortSha } from "../blame/format";
import { copyText } from "../clipboard";
import { Help } from "../Help";
import { toggleHud } from "../Hud";
import { call } from "../ipc";
import { isComposing, routeKey } from "../keys";
import { copyItem, openContextMenu, selectedText, type MenuEntry } from "../menu/Menu";
import { usePublish, type ViewSlot } from "../menu/registry";
import { lastName, type Compare, type Navigation, type RefKind } from "../nav";
import { commonCommands, usePalette, type Command } from "../palette/Palette";
import { getPerfInfo, nowMs, record } from "../perf";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { drawWindow, insideDrawn, type DrawnRange } from "../scrollWindow";
import { toast } from "../Toasts";
import { watchRepo } from "../watch";
import {
  KIND_LABEL,
  KINDS,
  keepSelection,
  matchesQuery,
  nextKind,
  shortRefName,
  trackingText,
  type RefItem,
  type RefList,
  type Tracking,
} from "./types";

type Props = {
  /** 저장소 안의 아무 경로 */
  repo: string;
  /** 처음 보일 종류 */
  show: RefKind;
  visible: boolean;
  active: boolean;
  slot: ViewSlot | null;
  depth: number;
  onShown?(committedMs: number): void;
  onBack(): void;
  onNavigate(navigation: Navigation): void;
};

type Load = { kind: "loading" } | { kind: "done"; list: RefList } | { kind: "failed"; message: string };

const ROW_HEIGHT = 24;

export function RefsView({ repo, show, visible, active, slot, depth, onShown, onBack, onNavigate }: Props) {
  const [kind, setKind] = useState<RefKind>(show);
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  /** 로컬 브랜치 전체 이름 → upstream·앞섬/뒤처짐(목록 뒤에 온다) */
  const [tracking, setTracking] = useState<Map<string, Tracking>>(new Map());
  const [trackingError, setTrackingError] = useState<string | null>(null);
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

  const all = load.kind === "done" ? load.list.refs : [];
  const ofKind = useMemo(() => all.filter((item) => item.kind === kind), [all, kind]);
  const rows = useMemo(
    () => ofKind.filter((item) => matchesQuery(item, tracking.get(item.full), query)),
    [ofKind, tracking, query],
  );
  const counts = useMemo(() => {
    const out: Record<RefKind, number> = { branch: 0, remote: 0, tag: 0 };
    for (const item of all) out[item.kind] += 1;
    return out;
  }, [all]);

  // 읽기: 목록을 먼저 그리고 upstream·앞섬/뒤처짐(git for-each-ref)을 이어 받는다. 고른 참조는 이름으로 이어 간다.
  useEffect(() => {
    let stale = false;
    const started = nowMs();
    const before = latest.current.rows[latest.current.selected];
    const index = latest.current.selected;
    call<RefList>("refs_list", { repo }).then(
      (list) => {
        if (stale) return;
        const listed = nowMs();
        nowAt.current = Date.now();
        setLoad({ kind: "done", list });
        const { kind: shownKind, query: q } = latest.current;
        const next = list.refs.filter(
          (item) => item.kind === shownKind && matchesQuery(item, latest.current.tracking.get(item.full), q),
        );
        setSelected(keepSelection(before, next, index));
        call<Tracking[]>("ref_tracking", { repo }).then(
          (items) => {
            if (stale) return;
            setTracking(new Map(items.map((t) => [t.full, t])));
            setTrackingError(null);
            record({
              kind: "refs",
              label: `브랜치·태그 ${lastName(repo)}`,
              totalMs: nowMs() - started,
              phases: [
                { label: "목록", ms: listed - started },
                { label: "upstream", ms: nowMs() - listed },
              ],
              ok: true,
            });
          },
          (e: unknown) => {
            if (stale) return;
            setTrackingError(String(e));
          },
        );
      },
      (e: unknown) => !stale && setLoad({ kind: "failed", message: String(e) }),
    );
    return () => {
      stale = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo, version]);

  // HEAD·참조가 바뀌면(커밋, 체크아웃, fetch, 브랜치·태그 만들기) 다시 읽는다. 숨어 있으면 보일 때.
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

  const current = (index = latest.current.selected): RefItem | null => latest.current.rows[index] ?? null;
  const repoOf = () => (latest.current.load.kind === "done" ? latest.current.load.list.repo : repo);
  /** 커밋을 가리키는 참조(아니면 알린다) */
  const existing = (index?: number): RefItem | null => {
    const item = current(index);
    if (item && !item.exists) {
      toast(`커밋을 가리키지 않는다: ${item.name}`);
      return null;
    }
    return item;
  };
  const upstreamOf = (item: RefItem): Tracking | undefined => latest.current.tracking.get(item.full);

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

  const openCompare = (compare: Compare) =>
    onNavigate({ kind: "commit", repo: repoOf(), rev: compare.base, path: null, compare });
  const openCommit = (index?: number) => {
    const item = existing(index);
    if (item) onNavigate({ kind: "commit", repo: repoOf(), rev: item.sha, path: null });
  };
  const openGraph = (index?: number) => {
    const item = existing(index);
    if (item) onNavigate({ kind: "graph", repo: repoOf(), rev: item.sha });
  };
  /** 지금 HEAD에서 갈라진 뒤 이 참조가 바꾼 것(HEAD...이름) */
  const compareHead = (index?: number) => {
    const item = existing(index);
    if (!item) return;
    const { load: l } = latest.current;
    const head = l.kind === "done" ? l.list.head : null;
    if (!head) return toast("HEAD가 없다(빈 저장소)");
    if (item.sha === head) return toast(`${item.name}은(는) HEAD와 같은 커밋이다`);
    openCompare({ base: "HEAD", target: item.name, mergeBase: true });
  };
  /** upstream에서 갈라진 뒤 이 브랜치가 바꾼 것(upstream...이름) */
  const compareUpstream = (index?: number) => {
    const item = existing(index);
    if (!item) return;
    if (item.kind !== "branch") return toast("upstream은 로컬 브랜치에만 있다");
    const t = upstreamOf(item);
    if (!t) return toast(trackingError ? `upstream을 읽지 못했다: ${trackingError}` : `${item.name}에는 upstream이 없다`);
    if (t.gone) return toast(`upstream ${shortRefName(t.upstream)}이(가) 없다(원격에서 지웠다)`);
    if (t.ahead === 0 && t.behind === 0) return toast(`${item.name}은(는) ${shortRefName(t.upstream)}과 같다`);
    openCompare({ base: shortRefName(t.upstream), target: item.name, mergeBase: true });
  };
  const compareWorktree = (index?: number) => {
    const item = existing(index);
    if (item) openCompare({ base: item.name, target: null, mergeBase: false });
  };
  const browse = (index?: number) => {
    const item = existing(index);
    if (item) onNavigate({ kind: "browse", repo: repoOf(), rev: item.sha });
  };
  const searchIn = (index?: number) => {
    const item = existing(index);
    if (item) onNavigate({ kind: "search", repo: repoOf(), rev: item.sha });
  };
  /** 이 브랜치를 체크아웃한 다른 워크트리로 옮긴다(그 워크트리의 그래프) */
  const openWorktree = (item: RefItem, path: string) =>
    onNavigate({ kind: "graph", repo: path, rev: item.sha, worktree: true });
  const copy = (text: string) =>
    copyText(text).then(
      () => toast(`복사했다: ${text.length > 40 ? shortSha(text) : text}`),
      (e: unknown) => toast(String(e)),
    );
  const copySha = (index?: number) => {
    const item = current(index);
    if (item) void copy(item.sha);
  };
  const copyName = (index?: number) => {
    const item = current(index);
    if (item) void copy(item.name);
  };
  const switchKind = (next?: RefKind) => {
    const to = next ?? nextKind(latest.current.kind);
    if (to === latest.current.kind) return;
    setKind(to);
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
      const item = current();
      setQuery("");
      if (item) setSelected(Math.max(0, latest.current.ofKind.indexOf(item)));
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

  const isBranch = kind === "branch";
  const commands: Command[] = [
    { id: "commit", label: "끝 커밋 상세", keys: "Enter", run: () => openCommit() },
    { id: "graph", label: "그래프에서 보기", keys: "G", run: () => openGraph() },
    { label: "HEAD에서 갈라진 뒤 바꾼 것", keys: "d", detail: "HEAD...이름", run: () => compareHead() },
    ...(isBranch
      ? [{ label: "upstream에서 갈라진 뒤 바꾼 것", keys: "u", detail: "upstream...이름", run: () => compareUpstream() }]
      : []),
    { label: "작업트리와 비교", keys: "w", run: () => compareWorktree() },
    { label: "이 커밋의 파일 트리", keys: "T", detail: "리비전 둘러보기", run: () => browse() },
    { label: "이 커밋의 파일에서 찾기", keys: "F", detail: "git grep (아래 패널)", run: () => searchIn() },
    { label: "커밋 sha 복사", keys: "y", run: () => copySha() },
    { label: "이름 복사", run: () => copyName() },
    { label: `${KIND_LABEL[nextKind(kind)]} 목록으로`, keys: "s", run: () => switchKind() },
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
  usePublish(slot, visible, "브랜치·태그", commands, {
    palette: palette.open,
    help: () => setHelp(true),
    files: quickOpen.open,
  });

  const latest = useRef({ rows, ofKind, selected, load, kind, query, tracking, visible, active, help, palette: palette.open, openFiles: quickOpen.open });
  latest.current = { rows, ofKind, selected, load, kind, query, tracking, visible, active, help, palette: palette.open, openFiles: quickOpen.open };

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
        case "G":
          openGraph();
          break;
        case "d":
          compareHead();
          break;
        case "u":
          if (latest.current.kind === "branch") compareUpstream();
          else handled = false;
          break;
        case "w":
          compareWorktree();
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
          switchKind();
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
    const branch = item.kind === "branch";
    const t = upstreamOf(item);
    const others = item.worktrees.filter((w) => !w.current);
    return [
      { header: `${item.name} · ${shortSha(item.sha)}` },
      { label: "끝 커밋 상세", keys: "Enter", disabled: gone, run: () => openCommit(index) },
      { label: "그래프에서 보기", keys: "G", disabled: gone, run: () => openGraph(index) },
      "separator",
      { label: "HEAD에서 갈라진 뒤 바꾼 것", keys: "d", disabled: gone || item.current, run: () => compareHead(index) },
      ...(branch
        ? [
            {
              label: "upstream에서 갈라진 뒤 바꾼 것",
              keys: "u",
              detail: t && !t.gone ? shortRefName(t.upstream) : undefined,
              disabled: gone || !t || t.gone || (t.ahead === 0 && t.behind === 0),
              run: () => compareUpstream(index),
            },
          ]
        : []),
      { label: "작업트리와 비교", keys: "w", disabled: gone, run: () => compareWorktree(index) },
      { label: "이 커밋의 파일 트리", keys: "T", disabled: gone, run: () => browse(index) },
      { label: "이 커밋의 파일에서 찾기", keys: "F", disabled: gone, run: () => searchIn(index) },
      ...others.map((w) => ({
        label: `워크트리 ${w.name}(으)로 옮기기`,
        detail: "그 워크트리의 그래프",
        disabled: gone,
        run: () => openWorktree(item, w.path),
      })),
      "separator",
      { label: "이름 복사", detail: item.name, run: () => copyName(index) },
      { label: "커밋 sha 복사", keys: "y", run: () => copySha(index) },
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
  const chosenTracking = chosen ? tracking.get(chosen.full) : undefined;
  const status =
    load.kind === "loading"
      ? "읽는 중…"
      : load.kind === "done"
        ? query
          ? `${formatCount(rows.length)} / ${formatCount(ofKind.length)}개`
          : `${formatCount(ofKind.length)}개${load.list.truncated ? " (최근 것만)" : ""}`
        : null;
  const empty: Record<RefKind, string> = {
    branch: "로컬 브랜치가 없다.",
    remote: "원격 브랜치가 없다. git fetch로 받은 브랜치가 여기 보인다.",
    tag: "태그가 없다.",
  };

  return (
    <section className="reflog-view refs-view">
      <header className="blame-header" onContextMenu={(e) => openContextMenu(e, [copyItem(selectedText(e.target))], "브랜치·태그")}>
        <span className="segmented" role="group" aria-label="종류">
          {KINDS.map((value) => (
            <button
              key={value}
              type="button"
              className={kind === value ? "toggle primary" : "toggle"}
              aria-pressed={kind === value}
              title={`${KIND_LABEL[value]} (s로 바꾼다)`}
              onClick={() => switchKind(value)}
            >
              {KIND_LABEL[value]}
              {load.kind === "done" && <span className="muted"> {formatCount(counts[value])}</span>}
            </button>
          ))}
        </span>
        <span className="file-path mono" title={repoOf()}>
          <span className="file-name">{lastName(repoOf())}</span>
        </span>
        {load.kind === "done" && load.list.detached && (
          <span className="badge warn" title="HEAD가 브랜치에 붙어 있지 않다">
            HEAD {load.list.head ? shortSha(load.list.head) : ""} (detached)
          </span>
        )}
        {depth > 1 && (
          <span className="badge" title="q로 앞 화면으로 돌아간다">
            뒤로 q · {depth - 1}
          </span>
        )}
        {trackingError && (
          <span className="muted" title={trackingError}>
            upstream을 읽지 못했다
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
            placeholder="거르기: 이름·sha·요약·작성자·upstream (Enter 목록으로, Esc 지우기)"
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
        aria-label={`${KIND_LABEL[kind]} 목록`}
        aria-activedescendant={chosen ? `refs-row-${selected}` : undefined}
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
                key={item.full}
                id={`refs-row-${i}`}
                role="option"
                aria-selected={i === selected}
                className={`history-row refs-row${i === selected ? " selected" : ""}${item.exists ? "" : " gone"}${item.current ? " current" : ""}`}
                style={{ top: i * ROW_HEIGHT, height: ROW_HEIGHT }}
                onClick={() => select(i)}
                onDoubleClick={() => {
                  select(i);
                  openCommit(i);
                }}
                onContextMenu={(e) => {
                  select(i);
                  openContextMenu(e, rowMenu(i), "브랜치·태그");
                }}
              >
                <Row item={item} tracking={tracking.get(item.full)} nowMs={nowAt.current} />
              </div>
            );
          })}
        </div>
        {load.kind === "done" && ofKind.length === 0 && (
          <div className="blame-message">
            <p className="muted">{empty[kind]}</p>
          </div>
        )}
        {load.kind === "done" && ofKind.length > 0 && rows.length === 0 && (
          <div className="blame-message">
            <p className="muted">맞는 것이 없다.</p>
          </div>
        )}
        {load.kind === "failed" && (
          <div className="blame-message error">
            <p>{load.message}</p>
          </div>
        )}
      </div>
      {chosen && (
        <footer className="history-detail" onContextMenu={(e) => openContextMenu(e, rowMenu(selected), "브랜치·태그")}>
          <span className="mono" title={chosen.full}>
            {chosen.full}
          </span>
          <span className="mono muted" title="y로 복사">
            {shortSha(chosen.sha)}
          </span>
          {chosenTracking && (
            <span title="upstream (u로 비교)">
              {shortRefName(chosenTracking.upstream)}{" "}
              <span className="muted">
                {chosenTracking.gone
                  ? "원격에서 지웠다"
                  : `앞섬 ${chosenTracking.ahead} · 뒤처짐 ${chosenTracking.behind}`}
              </span>
            </span>
          )}
          {chosen.author && <span>{chosen.author}</span>}
          {chosen.exists && <span className="muted">{formatDate(chosen.time, chosen.tzOffset)}</span>}
          {!chosen.exists && <span className="badge warn">커밋 아님</span>}
        </footer>
      )}
      {help && <Help view="refs" onClose={() => setHelp(false)} />}
      {quickOpen.overlay}
      {palette.overlay}
    </section>
  );
}

function Row({ item, tracking, nowMs: now }: { item: RefItem; tracking: Tracking | undefined; nowMs: number }) {
  const others = item.worktrees.filter((w) => !w.current);
  return (
    <>
      <span className="refs-current" aria-label={item.current ? "지금 브랜치" : undefined} title={item.current ? "지금 브랜치 (HEAD)" : undefined}>
        {item.current ? "●" : ""}
      </span>
      <span className="refs-name mono" title={item.full}>
        {item.name}
      </span>
      {item.kind === "tag" && item.annotated && (
        <span className="badge refs-annotated" title="태그 객체가 있는 태그 (git tag -a)">
          a
        </span>
      )}
      {others.map((w) => (
        <span key={w.path} className="badge refs-worktree" title={`워크트리 ${w.path}에서 체크아웃했다`}>
          ⌂ {w.name}
        </span>
      ))}
      {tracking && (
        <span
          className={`badge refs-track${tracking.gone ? " gone" : tracking.ahead || tracking.behind ? " diverged" : ""}`}
          title={
            tracking.gone
              ? `upstream ${shortRefName(tracking.upstream)}이(가) 없다`
              : `${shortRefName(tracking.upstream)}보다 앞섬 ${tracking.ahead} · 뒤처짐 ${tracking.behind}`
          }
        >
          {trackingText(tracking)}
        </span>
      )}
      <span className="history-sha mono">{shortSha(item.sha)}</span>
      <span className="history-summary">
        {item.summary ?? <span className="muted">(커밋 아님)</span>}
        {item.author && <span className="muted"> · {item.author}</span>}
      </span>
      <span className="history-time" title={item.exists ? formatDate(item.time, item.tzOffset) : undefined}>
        {item.exists ? relativeTime(item.time, now) : ""}
      </span>
    </>
  );
}
