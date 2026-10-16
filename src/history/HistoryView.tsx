// 파일 히스토리: 파일을 바꾼 커밋 목록(가상 스크롤). 이름을 바꾸기 전 구간은 옛 이름으로 이어진다.
// 라인 히스토리(`lines`)면 그 줄 범위를 바꾼 커밋 목록과, 고른 커밋이 그 범위를 어떻게 바꿨는지(hunk)를 아래에 보인다.
// 키: j/k 이동, Enter 커밋 상세(이 파일 diff만), b 그 리비전 blame, , 바꾸기 전 리비전 blame, y sha 복사,
//     / 검색, n/N 다음·이전 결과, G 그래프, q 뒤로, ? 도움말.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Help } from "../Help";
import { WorktreePicker } from "../worktree/WorktreePicker";
import { call } from "../ipc";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { commonCommands, usePalette, type Command } from "../palette/Palette";
import { routeKey } from "../keys";
import { openInEditor } from "../editor";
import { toggleHud } from "../Hud";
import { historySwitch } from "../worktree/switch";
import { copyText } from "../clipboard";
import { formatCount, formatDate, parseTz, relativeTime, shortSha } from "../blame/format";
import { STATUS_LETTER, STATUS_NAME } from "../commit/types";
import type { Navigation } from "../nav";
import { watchRepo } from "../watch";
import { afterPaint, getPerfInfo, nowMs, record } from "../perf";
import { Acceleration } from "./Acceleration";
import { findMatch, formerPaths, matches, scrollToRow, visibleRange } from "./list";
import { openHistory } from "./session";
import {
  PAGE,
  type HistoryMeta,
  type HistoryStats,
  type HistoryTarget,
  type Hunk,
  type LineRange,
  type LogEntry,
} from "./types";

type Props = {
  target: HistoryTarget;
  /** 있으면 그 줄 범위의 라인 히스토리 */
  lines?: LineRange | null;
  /** 탐색 스택 맨 위(보이는 화면)인가 */
  active: boolean;
  depth: number;
  /** 콜드 스타트의 첫 요청. 창이 보일 때까지 페인트가 없어 예산과 비교하지 않는다. */
  cold?: boolean;
  /** 첫 화면(첫 커밋들 또는 오류)을 DOM에 넣었다. */
  onShown?(committedMs: number): void;
  onBack(): void;
  onNavigate(navigation: Navigation): void;
  /**
   * 저장소가 바뀌었다(HEAD부터 거슬러 오르는 히스토리의 HEAD 이동): 이 화면을 새로 연다. 라인 히스토리는 줄 범위를
   * 새 작업트리의 줄로 옮겨 넘긴다(못 옮기면 없다).
   */
  onReload?(lines?: LineRange): void;
};

type Progress =
  | { kind: "loading" }
  | { kind: "streaming" }
  | { kind: "done"; stats: HistoryStats }
  | { kind: "failed"; message: string };

const ROW_HEIGHT = 24;
const TOAST_MS = 1800;
/** 성능 예산(docs/MILESTONES.md): 파일 히스토리 첫 50개 ≤ git log 첫 50개 + 30ms, 캐시 히트 재표시 ≤ 16ms */
const PAGE_SLACK_MS = 30;
const CACHE_HIT_BUDGET_MS = 16;

export function HistoryView({
  target,
  lines = null,
  active,
  depth,
  cold = false,
  onShown,
  onBack,
  onNavigate,
  onReload,
}: Props) {
  const [meta, setMeta] = useState<HistoryMeta | null>(null);
  /** 받은 커밋. 수천 개가 쌓이므로 배열은 고치고 길이만 상태로 둔다. */
  const entries = useRef<LogEntry[]>([]);
  const [count, setCount] = useState(0);
  const [progress, setProgress] = useState<Progress>({ kind: "loading" });
  const [selected, setSelected] = useState(0);
  const [viewport, setViewport] = useState({ top: 0, height: 0 });
  const [search, setSearch] = useState({ open: false, query: "" });
  const [help, setHelp] = useState(false);
  const [worktrees, setWorktrees] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const quickOpen = useQuickOpen({
    repo: meta?.repo ?? target.repo ?? target.path,
    current: meta?.path ?? null,
    onNavigate,
    onClosed: () => listRef.current?.focus({ preventScroll: true }),
  });
  /** 숨겼다 다시 보일 때 되돌릴 스크롤 위치(숨긴 요소의 스크롤은 브라우저마다 남지 않을 수 있다) */
  const savedTop = useRef(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const toastTimer = useRef(0);
  const nowAtOpen = useRef(Date.now());
  /** 첫 화면·첫 50개 측정 */
  const timing = useRef({
    t0: nowMs(),
    shown: false,
    page: null as Promise<number | null> | null,
    reported: false,
  });
  const [accelRequest, setAccelRequest] = useState(0);
  // 명령 팔레트: 키로 하는 일 모두와 머리줄의 가속 설정 버튼. 함수들은 아래에 있다(누를 때 부른다).
  const commands: Command[] = [
    { label: "커밋 상세 (이 파일 diff)", keys: "Enter", run: () => openCommit() },
    { label: "이 리비전의 파일 blame", keys: "b", run: () => openBlame(false) },
    { label: "바꾸기 전 리비전의 파일 blame", keys: ",", run: () => openBlame(true) },
    { label: "커밋 그래프 (고른 커밋)", keys: "G", run: () => openGraph() },
    { label: "커밋 sha 복사", keys: "y", run: () => copySha() },
    { label: "검색 (요약·작성자·sha·경로)", keys: "/", run: () => setSearch((s) => ({ ...s, open: true })) },
    { label: "다음 검색 결과", keys: "n", run: () => findNext(1) },
    { label: "이전 검색 결과", keys: "N", run: () => findNext(-1) },
    { label: "처음 커밋으로", keys: "Home", run: () => select(0) },
    { label: "마지막 커밋으로", keys: "End", run: () => select(latest.current.count - 1) },
    { label: "에디터로 열기", keys: "o", detail: "작업트리 파일", run: () => openEditor() },
    { label: "가속 설정 (commit-graph·Bloom 필터)", detail: "히스토리를 빠르게", run: () => setAccelRequest((n) => n + 1) },
    ...commonCommands({
      files: quickOpen.open,
      worktrees: () => setWorktrees(true),
      help: () => setHelp(true),
      back: onBack,
      hud: getPerfInfo().enabled ? toggleHud : undefined,
    }),
  ];
  const palette = usePalette(commands, () => listRef.current?.focus({ preventScroll: true }));
  const latest = useRef({ active, meta, selected, help, search, count, openFiles: quickOpen.open, palette: palette.open });
  latest.current = { active, meta, selected, help, search, count, openFiles: quickOpen.open, palette: palette.open };

  const showToast = (message: string) => {
    setToast(message);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), TOAST_MS);
  };

  // 히스토리를 받는다.
  useEffect(() => {
    const handle = openHistory(target, lines, (event) => {
      switch (event.kind) {
        case "meta": {
          const { kind: _kind, ...m } = event;
          setMeta(m);
          setProgress({ kind: "streaming" });
          call("files_touch", { repo: m.repo, path: m.path }).catch(() => {});
          break;
        }
        case "entries":
          for (const entry of event.entries) entries.current.push(entry);
          setCount(entries.current.length);
          break;
        case "done":
          setProgress({ kind: "done", stats: event.stats });
          break;
        case "error":
          setProgress({ kind: "failed", message: event.message });
          break;
        case "cancelled":
          break;
      }
    });
    return () => {
      handle.cancel();
      window.clearTimeout(toastTimer.current);
    };
    // 요청마다 새 컴포넌트다(App이 key를 준다).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // HEAD부터 거슬러 오르는 히스토리: HEAD가 바뀌면(새 커밋 등) 다시 연다. 고정 리비전은 바뀌지 않는다.
  // 열기를 시작할 때 요청 대상으로 구독한다(meta를 기다리면 다시 열다 실패한 화면은 더는 바뀌지 않는다).
  // 라인 히스토리의 줄 범위는 시작 커밋 버전의 줄이라, 새 작업트리의 줄로 옮겨 다시 연다(위에 줄이 늘어도 같은 코드).
  useEffect(() => {
    if (target.rev !== null || !onReload) return;
    return watchRepo(target.repo ?? target.path, (change) => {
      if (!change.headChanged) return;
      const m = latest.current.meta;
      if (!lines || !m?.lines || !m.start) {
        onReload();
        return;
      }
      const from = { rev: m.start, path: m.path };
      const to = { rev: null, path: m.path };
      const map = (line: number) => call<number>("blame_map_line", { repo: m.repo, from, to, line });
      Promise.all([map(m.lines.start), map(m.lines.end)]).then(
        ([start, end]) => onReload({ start, end: Math.max(start, end) }),
        () => onReload(),
      );
    });
    // 요청마다 새 컴포넌트다(App이 key를 준다).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 첫 화면과 첫 50개(적으면 전부)를 그린 시각
  useLayoutEffect(() => {
    const t = timing.current;
    const ended = progress.kind === "done" || progress.kind === "failed";
    if (!t.shown && (count > 0 || ended)) {
      t.shown = true;
      onShown?.(nowMs());
    }
    if (!t.page && (count >= PAGE || ended)) t.page = afterPaint();
  }, [count, progress]);

  // 끝나면 성능 HUD에 남긴다: 첫 50개를 그린 때까지, 예산은 git이 50개째를 낸 때 + 30ms.
  useEffect(() => {
    const t = timing.current;
    if (progress.kind !== "done" || t.reported || !t.page) return;
    t.reported = true;
    const { stats } = progress;
    void t.page.then((painted) => {
      const end = painted ?? nowMs();
      const budgetMs = cold
        ? undefined
        : stats.cached
          ? CACHE_HIT_BUDGET_MS
          : stats.gitPageMs === null
            ? undefined
            : Math.round(stats.gitPageMs + PAGE_SLACK_MS);
      const phases = [
        { label: "첫 묶음", ms: stats.firstEntryMs ?? NaN },
        { label: `${PAGE}개째 보냄`, ms: stats.firstPageMs ?? NaN },
        { label: "표시", ms: end - t.t0 },
        { label: "끝", ms: stats.totalMs },
      ].filter((p) => Number.isFinite(p.ms));
      const range = latest.current.meta?.lines ?? lines;
      const what = range
        ? `라인 히스토리 ${latest.current.meta?.path ?? target.path}:${range.start}-${range.end}`
        : `히스토리 ${latest.current.meta?.path ?? target.path}`;
      record({
        kind: "history",
        label: `${what}${stats.cached ? " (캐시)" : ""}${cold ? " (콜드 스타트)" : ""}`,
        totalMs: end - t.t0,
        budgetMs,
        phases,
        detail: `${stats.cached ? "캐시" : `git ${PAGE}개째 ${stats.gitPageMs?.toFixed(1) ?? "—"}`} · 커밋 ${stats.entries} · 구간 ${stats.segments}`,
        ok: painted !== null,
      });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [progress]);

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

  useEffect(() => {
    const list = listRef.current;
    if (active && list) {
      if (list.scrollTop !== savedTop.current) list.scrollTop = savedTop.current;
      setViewport({ top: list.scrollTop, height: list.clientHeight });
      list.focus({ preventScroll: true });
    } else {
      setHelp(false);
      setSearch((s) => ({ ...s, open: false }));
    }
  }, [active]);

  useEffect(() => {
    if (search.open) searchRef.current?.focus();
    else if (latest.current.active) listRef.current?.focus({ preventScroll: true });
  }, [search.open]);

  /** `index`를 고르고 보이게 스크롤한다. */
  const select = (index: number) => {
    const n = latest.current.count;
    if (n === 0) return;
    const next = Math.max(0, Math.min(n - 1, index));
    setSelected(next);
    const list = listRef.current;
    if (list) {
      const top = scrollToRow(next, list.scrollTop, list.clientHeight, ROW_HEIGHT);
      if (top !== list.scrollTop) list.scrollTop = top;
    }
  };

  const pageRows = () => Math.max(1, Math.floor((listRef.current?.clientHeight ?? ROW_HEIGHT) / ROW_HEIGHT) - 1);

  /** 디렉터리·저장소 전체 히스토리면 커밋마다 파일 상태가 없다. */
  const isFileHistory = () => entries.current.some((e) => e.status !== null);

  const current = (index = latest.current.selected): { m: HistoryMeta; e: LogEntry } | null => {
    const m = latest.current.meta;
    const e = entries.current[index];
    return m && e ? { m, e } : null;
  };

  const openCommit = (index?: number) => {
    const at = current(index);
    if (!at) return;
    const file = isFileHistory();
    onNavigate({ kind: "commit", repo: at.m.repo, rev: at.e.sha, path: file ? at.e.path : null, only: file });
  };

  /** 그 리비전(`parent`면 바꾸기 전 리비전)의 파일 blame. 라인 히스토리면 그 범위가 시작하는 줄로 간다. */
  const openBlame = (parent: boolean) => {
    const at = current();
    if (!at) return;
    const { m, e } = at;
    if (!isFileHistory()) {
      showToast("디렉터리 히스토리에서는 blame하지 않는다");
      return;
    }
    const first = e.parents[0];
    const hunk = e.hunks?.[0];
    if (parent) {
      if (e.status === "added" || !first) {
        showToast("이 커밋에서 생긴 파일이라 앞 버전이 없다");
        return;
      }
      // 이름을 바꾼 커밋의 앞 버전은 옛 이름이다.
      const line = hunk ? Math.max(1, hunk.oldStart) : null;
      onNavigate({ kind: "file", target: { repo: m.repo, path: e.oldPath ?? e.path, rev: first }, line });
    } else if (e.status === "deleted") {
      // 지운 커밋에는 파일이 없다. 지우기 전 버전을 연다.
      if (first) onNavigate({ kind: "file", target: { repo: m.repo, path: e.path, rev: first }, line: null });
    } else {
      const line = hunk ? Math.max(1, hunk.newStart) : (m.lines?.start ?? null);
      onNavigate({ kind: "file", target: { repo: m.repo, path: e.path, rev: e.sha }, line });
    }
  };

  const openGraph = () => {
    const at = current();
    if (at) onNavigate({ kind: "graph", repo: at.m.repo, rev: at.e.sha });
  };

  /** 작업트리 파일을 에디터로(라인 히스토리면 그 범위의 첫 줄). 저장소 전체 히스토리면 작업트리 폴더. */
  const openEditor = () => {
    const m = latest.current.meta;
    if (!m) {
      showToast("아직 히스토리를 여는 중이다");
      return;
    }
    const path = m.path === "." ? null : m.path;
    openInEditor({ repo: m.repo, path, rev: m.rev, line: lines?.start ?? null }).then(showToast);
  };

  const copySha = () => {
    const at = current();
    if (!at) return;
    copyText(at.e.sha).then(
      () => showToast(`복사했다: ${shortSha(at.e.sha)}`),
      (e: unknown) => showToast(String(e)),
    );
  };

  const findNext = (direction: 1 | -1, from = latest.current.selected) => {
    const query = latest.current.search.query;
    if (!query.trim()) return;
    const found = findMatch(entries.current, from, query, direction);
    if (found < 0) showToast(`찾지 못했다: ${query}`);
    else select(found);
  };

  // 키: 화면이 맨 위일 때만. 검색 입력 중이면 넘긴다(입력 칸이 Enter·Esc를 따로 다룬다).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { active: on, openFiles, palette: openPalette } = latest.current;
      if (!on || routeKey(e, { files: openFiles, palette: openPalette }) !== "view") return;
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
          select(latest.current.count - 1);
          break;
        case "Enter":
          openCommit();
          break;
        case "b":
          openBlame(false);
          break;
        case ",":
          openBlame(true);
          break;
        case "y":
          copySha();
          break;
        case "/":
          setSearch((s) => ({ ...s, open: true }));
          break;
        case "n":
          findNext(1);
          break;
        case "N":
          findNext(-1);
          break;
        case "G":
          openGraph();
          break;
        case "o":
          openEditor();
          break;
        case "q":
          onBack();
          break;
        case "W":
          setWorktrees(true);
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

  const all = entries.current;
  const { start, end } = visibleRange(viewport.top, viewport.height, ROW_HEIGHT, count);
  const path = meta?.path ?? target.path;
  const range = meta?.lines ?? lines;
  const former = progress.kind === "done" ? formerPaths(all, path) : [];
  const chosen = all[selected];
  const query = search.query.trim();

  return (
    <section className={lines ? "history line-history" : "history"}>
      <header className="blame-header">
        <span className="badge">{lines ? "라인 히스토리" : "히스토리"}</span>
        <FilePath path={path} />
        {range && (
          <span className="badge mono" title={meta?.rev || !meta ? undefined : "작업트리 줄 번호를 HEAD 버전의 줄로 옮긴 범위"}>
            {range.start === range.end ? `${range.start}줄` : `${range.start}–${range.end}줄`}
          </span>
        )}
        {former.length > 0 && (
          <span className="badge" title={`이전 이름: ${former.join(" ← ")}`}>
            이름 바뀜 {former.length}번
          </span>
        )}
        {meta?.rev && (
          <span className="badge mono" title={`${meta.rev}부터 거슬러 올라간다`}>
            @{shortSha(meta.rev)}
          </span>
        )}
        {depth > 1 && (
          <span className="badge" title="q로 앞 화면으로 돌아간다">
            뒤로 q · {depth - 1}
          </span>
        )}
        {meta && <Acceleration repo={meta.repo} request={accelRequest} onDone={showToast} />}
        <span className="blame-status">
          {progress.kind === "loading" && "여는 중…"}
          {progress.kind === "streaming" && `읽는 중… 커밋 ${formatCount(count)}개`}
          {progress.kind === "done" && `커밋 ${formatCount(count)}개`}
          {progress.kind === "failed" && <span className="error-text">실패</span>}
        </span>
        <button type="button" className="icon" onClick={() => setHelp(true)} aria-label="단축키 (?)" title="단축키 (?)">
          ?
        </button>
      </header>
      {search.open && (
        <div className="history-search">
          <input
            ref={searchRef}
            type="search"
            value={search.query}
            placeholder="요약·작성자·sha·경로 (Enter 다음, Shift+Enter 이전, Esc 닫기)"
            aria-label="히스토리 검색"
            onChange={(e) => setSearch({ open: true, query: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                findNext(e.shiftKey ? -1 : 1);
              } else if (e.key === "Escape") {
                e.preventDefault();
                setSearch((s) => ({ ...s, open: false }));
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
        aria-label={`${path} 히스토리`}
        aria-activedescendant={chosen ? `history-row-${selected}` : undefined}
        onScroll={(e) => {
          const list = e.currentTarget;
          if (latest.current.active) savedTop.current = list.scrollTop;
          setViewport({ top: list.scrollTop, height: list.clientHeight });
        }}
      >
        <div className="history-rows" style={{ height: count * ROW_HEIGHT }}>
          {all.slice(start, end).map((entry, offset) => {
            const i = start + offset;
            const classes = ["history-row"];
            if (i === selected) classes.push("selected");
            if (query && matches(entry, query)) classes.push("match");
            return (
              <div
                key={entry.sha}
                id={`history-row-${i}`}
                role="option"
                aria-selected={i === selected}
                className={classes.join(" ")}
                style={{ top: i * ROW_HEIGHT, height: ROW_HEIGHT }}
                onClick={() => select(i)}
                onDoubleClick={() => {
                  select(i);
                  openCommit(i);
                }}
              >
                <Row entry={entry} path={path} nowMs={nowAtOpen.current} />
              </div>
            );
          })}
        </div>
        {progress.kind === "done" && count === 0 && (
          <div className="blame-message">
            <p>{lines ? "이 줄들을 바꾼 커밋이 없다." : "이 경로를 바꾼 커밋이 없다."}</p>
          </div>
        )}
        {progress.kind === "failed" && (
          <div className="blame-message error">
            <p>{progress.message}</p>
          </div>
        )}
      </div>
      {lines && chosen && <Hunks key={chosen.sha} entry={chosen} />}
      {chosen && (
        <footer className="history-detail">
          <span className="mono" title="y로 복사">
            {chosen.sha}
          </span>
          <span>
            {chosen.author} <span className="muted">&lt;{chosen.authorMail}&gt;</span>
          </span>
          <span className="muted">{formatDate(chosen.authorTime, parseTz(chosen.authorTz))}</span>
          {chosen.parents.length > 1 && <span className="badge">merge</span>}
          {chosen.path !== path && <span className="badge mono">{chosen.path}</span>}
        </footer>
      )}
      {help && <Help view="history" onClose={() => setHelp(false)} />}
      {worktrees && meta && (
        <WorktreePicker
          repo={meta.repo}
          onClose={() => setWorktrees(false)}
          onPick={(worktree) => {
            setWorktrees(false);
            // 리비전에서 시작한 히스토리는 그대로, HEAD에서 시작한 것은 그 워크트리의 HEAD에서(라인 히스토리면 줄 범위를
            // 그 워크트리의 작업트리 줄로 옮긴다: src/worktree/switch.ts).
            const mapLine = (from: { rev: string; path: string }, line: number) =>
              call<number>("blame_map_line", { repo: worktree.path, from, to: { rev: null, path: from.path }, line });
            void historySwitch(worktree, meta, async (from, range) => {
              const [start, end] = await Promise.all([mapLine(from, range.start), mapLine(from, range.end)]);
              return { start, end: Math.max(start, end) };
            }).then((next) => next && onNavigate(next));
          }}
        />
      )}
      {quickOpen.overlay}
      {palette.overlay}
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}

function Row({ entry, path, nowMs }: { entry: LogEntry; path: string; nowMs: number }) {
  const status = entry.status;
  return (
    <>
      {status ? (
        <span className={`status status-${status}`} title={STATUS_NAME[status]}>
          {STATUS_LETTER[status]}
        </span>
      ) : (
        <span className="status status-merge" title="merge 커밋 (바뀐 파일은 첫 부모와 비교해 본다)">
          ·
        </span>
      )}
      <span className="history-sha mono">{shortSha(entry.sha)}</span>
      <span className="history-summary">{entry.summary}</span>
      {entry.oldPath ? (
        <span className="history-path mono" title={`${entry.oldPath} → ${entry.path}`}>
          ← {entry.oldPath}
        </span>
      ) : (
        entry.path !== path && (
          <span className="history-path mono" title="이 커밋 때의 이름">
            {entry.path}
          </span>
        )
      )}
      <span className="history-author" title={entry.authorMail}>
        {entry.author}
      </span>
      <span className="history-time" title={formatDate(entry.authorTime, parseTz(entry.authorTz))}>
        {relativeTime(entry.authorTime, nowMs)}
      </span>
    </>
  );
}

/** 라인 히스토리: 고른 커밋이 그 줄 범위를 어떻게 바꿨는지(옛·새 줄 번호와 함께) */
function Hunks({ entry }: { entry: LogEntry }) {
  const hunks = entry.hunks ?? [];
  return (
    <div className="history-hunks" aria-label="바뀐 줄">
      {hunks.length === 0 ? (
        <p className="muted">이 커밋(merge)은 이 줄들을 직접 바꾸지 않았다. Enter로 커밋 상세를 본다.</p>
      ) : (
        hunks.map((hunk, i) => <HunkTable key={i} hunk={hunk} />)
      )}
    </div>
  );
}

function HunkTable({ hunk }: { hunk: Hunk }) {
  let oldLine = hunk.oldStart;
  let newLine = hunk.newStart;
  return (
    <table className="hunk">
      <thead>
        <tr>
          <td colSpan={3} className="hunk-header mono">
            @@ −{hunk.oldStart},{hunk.oldLines} +{hunk.newStart},{hunk.newLines} @@
          </td>
        </tr>
      </thead>
      <tbody>
        {hunk.lines.map((line, i) => {
          const kind = line[0];
          let before = "";
          let after = "";
          if (kind === " ") {
            before = String(oldLine++);
            after = String(newLine++);
          } else if (kind === "-") {
            before = String(oldLine++);
          } else if (kind === "+") {
            after = String(newLine++);
          }
          const className = kind === "+" ? "add" : kind === "-" ? "del" : kind === "\\" ? "note" : "ctx";
          return (
            <tr key={i} className={className}>
              <td className="ln">{before}</td>
              <td className="ln">{after}</td>
              <td className="code">{kind === "\\" ? line.slice(1).trim() : line.slice(1)}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

/** 파일 이름을 먼저, 디렉터리는 흐리게. */
function FilePath({ path }: { path: string }) {
  const cut = path.lastIndexOf("/") + 1;
  return (
    <span className="file-path mono" title={path}>
      {cut > 0 && <span className="file-dir">{path.slice(0, cut)}</span>}
      <span className="file-name">{path.slice(cut)}</span>
    </span>
  );
}
