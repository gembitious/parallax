// 커밋 상세: 머리(요약·작성자·날짜·sha·메시지), 바뀐 파일 목록, 고른 파일의 diff(분할·통합).
// 파일 히스토리에서 열면 그 파일만 보인다(a로 모든 파일).
// 큰 커밋은 목록을 먼저 보이고 시간 안에 세지 못한 줄 수는 나눠 받는다(file_stats). 목록은 보이는 행만 그린다.
// 키: j/k 파일 이동, n/N 다음·이전 바뀐 부분, u 분할↔통합, Enter 그 리비전의 파일 blame, a 이 파일만↔모든 파일,
//     y sha 복사, G 그래프, q 뒤로, ? 도움말.
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Help } from "../Help";
import { WorktreePicker } from "../worktree/WorktreePicker";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { commonCommands, usePalette, type Command } from "../palette/Palette";
import { routeKey } from "../keys";
import { openInEditor } from "../editor";
import { toggleHud } from "../Hud";
import { commitSwitch } from "../worktree/switch";
import { copyText } from "../clipboard";
import { displayText } from "../blame/BlameView";
import { formatDate, relativeTime, shortSha } from "../blame/format";
import { call } from "../ipc";
import type { Navigation } from "../nav";
import { afterPaint, getPerfInfo, longFrameIn, nowMs, record } from "../perf";
import { scrollToRow } from "../history/list";
import { drawWindow, insideDrawn, type DrawnRange } from "../scrollWindow";
import {
  VERSIONS_LIMITS,
  formatBytes,
  modeNote,
  remember,
  sameNote,
  statBatches,
  statRequest,
  versionsSize,
  withStats,
} from "./detail";
import { createDiffView, type DiffMode, type DiffView } from "./diff";
import {
  STATUS_LETTER,
  STATUS_NAME,
  type CommitDetail,
  type FileChange,
  type FileStat,
  type FileVersions,
} from "./types";

type Props = {
  /** 저장소 안의 아무 경로 */
  repo: string;
  rev: string;
  /** 처음 고를 파일(저장소 기준 경로) */
  path: string | null;
  /** `path` 파일만 보인다(파일 히스토리에서 열었다). */
  only?: boolean;
  /** 보이는 화면인가(열린 패널의 스택 맨 위) */
  visible: boolean;
  /** 키를 받는 화면인가(보이고, 패널에 포커스가 있다) */
  active: boolean;
  depth: number;
  onShown?(committedMs: number): void;
  onBack(): void;
  onNavigate(navigation: Navigation): void;
};

/** 성능 예산(docs/MILESTONES.md): 커밋 상세(파일 목록 + diff) ≤ 50ms */
const BUDGET_MS = 50;
const TOAST_MS = 1800;
/** 파일 목록 행 높이(px). styles.css의 .file-row와 같다. */
const ROW_HEIGHT = 22;
const MODE_KEY = "plx.diff.mode";

function readMode(): DiffMode {
  try {
    return localStorage.getItem(MODE_KEY) === "unified" ? "unified" : "split";
  } catch {
    return "split";
  }
}

function writeMode(mode: DiffMode) {
  try {
    localStorage.setItem(MODE_KEY, mode);
  } catch {
    // 저장하지 못해도 이번 실행에는 반영된다.
  }
}

type VersionsState = FileVersions | "loading" | { error: string };

/** 받은 두 버전(받는 중·실패가 아니면) */
function loaded(state: VersionsState | undefined): FileVersions | null {
  return state && state !== "loading" && !("error" in state) ? state : null;
}

/** diff로 보일 수 있는 두 버전(바이너리·받지 않은 blob·큰 파일이 아니다) */
function diffable(state: VersionsState | undefined): FileVersions | null {
  const v = loaded(state);
  return v && !v.binary && !v.missing && v.tooLarge === null ? v : null;
}

/** 파일 하나의 두 버전을 기억하는 열쇠. `diff` 속성이 경로를 보므로 경로도 넣는다. */
const versionsKey = (f: FileChange) => `${f.oldId ?? ""}:${f.newId ?? ""}:${f.path}`;

export function CommitView({ repo, rev, path, only = false, visible, active, depth, onShown, onBack, onNavigate }: Props) {
  const [full, setFull] = useState<CommitDetail | null>(null);
  /** 이 파일만 보는 중(히스토리에서 열었고 그 파일이 이 커밋에 있다) */
  const [narrow, setNarrow] = useState(only);
  const [fatal, setFatal] = useState<string | null>(null);
  /** 시간 안에 세지 못한 줄 수를 받다 실패했다 */
  const [statsError, setStatsError] = useState<string | null>(null);
  const [selected, setSelected] = useState(0);
  const [mode, setMode] = useState<DiffMode>(readMode);
  const [versions, setVersions] = useState<ReadonlyMap<string, VersionsState>>(() => new Map());
  /** 바뀌면 고른 파일을 다시 받는다(받다 실패한 파일의 "다시 시도") */
  const [retries, setRetries] = useState(0);
  const [viewport, setViewport] = useState({ top: 0, height: 0 });
  const [help, setHelp] = useState(false);
  const [worktrees, setWorktrees] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const diffParent = useRef<HTMLDivElement>(null);
  const diff = useRef<DiffView | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  /** 다시 보일 때 되돌릴 목록 스크롤 위치(숨긴 요소는 스크롤 위치를 잃는다) */
  const savedTop = useRef(0);
  const drawnRange = useRef<DrawnRange>({ start: 0, end: 0, height: 0 });
  /** 두 버전을 받는 중이다. 한 번에 하나만 받는다. */
  const fetching = useRef(false);
  // 닫으면 diff에 포커스를 돌려준다(입력 칸이 사라져 body로 가면 화살표·PageDown이 diff를 움직이지 않는다).
  const focusDiff = () => diff.current?.focus();
  const quickOpen = useQuickOpen({ repo, current: null, onNavigate, onClosed: focusDiff });
  const toastTimer = useRef(0);
  /** 첫 파일까지의 측정: 상세 도착 → 목록 표시 → 첫 파일(diff나 알림) 표시 */
  // 첫 파일의 두 버전이 온 때(versionsMs)와 diff 뷰를 만든 때(diffMs)도 남겨 '첫 파일 표시'가 어디서 늘었는지 본다.
  const timing = useRef({
    t0: nowMs(),
    detailMs: NaN,
    versionsMs: NaN,
    diffMs: NaN,
    listPainted: null as Promise<number | null> | null,
    reported: false,
  });
  const detail = useMemo(() => narrowed(full, narrow ? path : null), [full, narrow, path]);
  const hasDetail = full !== null;
  const file: FileChange | undefined = detail?.files[selected];
  const fileKey = file ? versionsKey(file) : "";
  const current = versions.get(fileKey);
  const shown = loaded(current);
  /** diff에 넣을 두 글(CRLF·끝 개행을 정리한 것) */
  const texts = useMemo(() => {
    const v = diffable(current);
    if (!v) return null;
    return { old: v.old ? displayText(v.old.text).text : "", new: v.new ? displayText(v.new.text).text : "" };
  }, [current]);
  // 명령 팔레트: 키로 하는 일 모두. 함수들은 아래에 있다(누를 때 부른다).
  const commands: Command[] = [
    { label: "다음 파일", keys: "j", run: () => select(latest.current.selected + 1) },
    { label: "이전 파일", keys: "k", run: () => select(latest.current.selected - 1) },
    { label: "다음 바뀐 부분", keys: "n", run: () => diff.current?.next() },
    { label: "이전 바뀐 부분", keys: "N", run: () => diff.current?.previous() },
    { label: "분할 ↔ 통합 보기", keys: "u", run: () => toggleMode() },
    { label: "이 리비전의 파일 blame", keys: "Enter", run: () => openBlame() },
    ...(path !== null ? [{ label: "이 파일만 ↔ 모든 파일", keys: "a", run: () => toggleNarrow() }] : []),
    ...(shown?.tooLarge != null ? [{ label: "큰 파일도 diff 보기", run: () => fetchVersions(true) }] : []),
    ...(isError(current) ? [{ label: "이 파일 다시 받기", run: () => retry() }] : []),
    { label: "커밋 sha 복사", keys: "y", run: () => copySha() },
    { label: "커밋 그래프 (이 커밋)", keys: "G", run: () => openGraph() },
    { label: "에디터로 열기", keys: "o", detail: "고른 파일의 첫 바뀐 줄(작업트리)", run: () => openEditor() },
    ...commonCommands({
      files: quickOpen.open,
      worktrees: () => setWorktrees(true),
      help: () => setHelp(true),
      back: onBack,
      hud: getPerfInfo().enabled ? toggleHud : undefined,
    }),
  ];
  const palette = usePalette(commands, focusDiff);
  const snapshot = () => ({
    visible,
    active,
    full,
    detail,
    selected,
    file,
    fileKey,
    versions,
    help,
    mode,
    narrow,
    openFiles: quickOpen.open,
    palette: palette.open,
  });
  const latest = useRef(snapshot());
  latest.current = snapshot();

  const showToast = (message: string) => {
    setToast(message);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), TOAST_MS);
  };

  /** 시간 안에 세지 못한 줄 수를 묶음으로 나눠 받는다(목록 순서대로). 화면을 닫으면 그만둔다. */
  const countRest = async (d: CommitDetail, disposed: () => boolean) => {
    for (const batch of statBatches(d.files)) {
      let stats: FileStat[];
      try {
        stats = await call<FileStat[]>("file_stats", { repo, files: batch.map((i) => statRequest(d.files[i])) });
      } catch (e) {
        if (!disposed()) setStatsError(String(e));
        return;
      }
      if (disposed()) return;
      setFull((prev) => (prev ? withStats(prev, batch, stats) : prev));
    }
  };

  // 상세를 받는다.
  useEffect(() => {
    let disposed = false;
    call<CommitDetail>("commit_detail", { repo, rev, focus: path }).then(
      (d) => {
        if (disposed) return;
        timing.current.detailMs = nowMs();
        setFull(d);
        const focus = path === null ? -1 : d.files.findIndex((f) => isPath(f, path));
        // 그 파일이 없으면(merge의 첫 부모 비교 등) 모든 파일을 보인다.
        if (focus < 0) setNarrow(false);
        setSelected(only ? 0 : Math.max(0, focus));
        if (!d.counted) void countRest(d, () => disposed);
      },
      (e: unknown) => {
        if (disposed) return;
        setFatal(String(e));
        onShown?.(nowMs());
      },
    );
    return () => {
      disposed = true;
      window.clearTimeout(toastTimer.current);
    };
    // 요청마다 새 컴포넌트다(App이 key를 준다).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 목록을 그린 시각(첫 화면). 줄 수가 나중에 와도 다시 재지 않는다.
  useEffect(() => {
    if (!hasDetail) return;
    onShown?.(nowMs());
    timing.current.listPainted = afterPaint();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasDetail]);

  /** 보고 있는 파일은 잊지 않게 넣는다(늦게 온 다른 파일의 답이 밀어내지 않는다). */
  const put = (key: string, value: VersionsState) =>
    setVersions((m) => remember(m, key, value, versionsSize, VERSIONS_LIMITS, latest.current.fileKey));

  /**
   * 고른 파일의 두 버전을 받는다. 받는 중이면 그것이 끝난 뒤 그때 고른 파일을 받는다(j를 누르고 있어도 요청이
   * 쌓이지 않는다). `large`면 큰 파일도 내용을 받는다("그래도 보기").
   */
  const fetchVersions = (large = false) => {
    const { file: f, fileKey: key, versions: have } = latest.current;
    if (!f || f.submodule) return;
    if (!large) {
      if (fetching.current || have.has(key)) return;
      fetching.current = true;
    }
    put(key, "loading");
    call<FileVersions>("file_versions", { repo, oldId: f.oldId, newId: f.newId, path: f.path, full: large })
      .then(
        (v) => {
          if (Number.isNaN(timing.current.versionsMs)) timing.current.versionsMs = nowMs();
          put(key, v);
        },
        (e: unknown) => put(key, { error: String(e) }),
      )
      .finally(() => {
        if (large) return;
        fetching.current = false;
        fetchVersions();
      });
  };

  const retry = () => {
    const key = latest.current.fileKey;
    setVersions((m) => {
      const next = new Map(m);
      next.delete(key);
      return next;
    });
    setRetries((n) => n + 1);
  };

  // 고른 파일이 바뀌면 그 두 버전을 받는다(한 번 받은 것은 다시 받지 않는다).
  useEffect(() => {
    fetchVersions();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fileKey, retries]);

  // diff 뷰를 만든다(파일이나 보기 방식이 바뀌면 새로).
  useEffect(() => {
    const parent = diffParent.current;
    if (!parent || !file || !texts) return;
    const view = createDiffView(parent, { path: file.path, old: texts.old, new: texts.new, mode });
    if (Number.isNaN(timing.current.diffMs)) timing.current.diffMs = nowMs();
    diff.current = view;
    if (latest.current.active) view.focus();
    return () => {
      view.destroy();
      diff.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fileKey, texts, mode]);

  // 첫 파일(diff나 알림)을 보인 때를 잰다. 바뀐 파일이 없으면 목록을 보인 때다.
  useEffect(() => {
    const t = timing.current;
    if (!hasDetail || t.reported) return;
    if (file && !file.submodule && (current === undefined || current === "loading")) return;
    t.reported = true;
    const doneMs = nowMs();
    const what = file ? file.path : "바뀐 파일 없음";
    void Promise.all([t.listPainted, afterPaint()]).then(([list, painted]) => {
      const end = painted ?? doneMs;
      record({
        kind: "commit",
        label: `커밋 ${shortSha(rev)}`,
        totalMs: end - t.t0,
        budgetMs: BUDGET_MS,
        phases: [
          { label: "상세 도착", ms: t.detailMs - t.t0 },
          { label: "목록 표시", ms: (list ?? NaN) - t.t0 },
          { label: "버전 도착", ms: t.versionsMs - t.t0 },
          { label: "diff 만듦", ms: t.diffMs - t.t0 },
          { label: "첫 파일 표시", ms: end - t.t0 },
        ].filter((p) => Number.isFinite(p.ms)),
        detail: [`파일 ${latest.current.full?.files.length ?? 0}개`, what, longFrameIn(t.t0, end)].filter(Boolean).join(" · "),
        ok: painted !== null,
      });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasDetail, fileKey, current]);

  // 목록 크기를 따라간다. 처음 재는 것은 그리기 전에(빈 목록을 한 번 보이지 않게).
  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const update = () => setViewport({ top: list.scrollTop, height: list.clientHeight });
    update();
    const observer = new ResizeObserver(update);
    observer.observe(list);
    return () => observer.disconnect();
  }, []);

  // 고른 파일이 목록에서 보이게 한다. 줄 수가 와서 목록이 바뀔 때는 움직이지 않는다(마우스로 내려 본 자리를 둔다).
  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list || !latest.current.visible) return;
    const top = scrollToRow(selected, list.scrollTop, list.clientHeight, ROW_HEIGHT);
    if (top === list.scrollTop) return;
    list.scrollTop = top;
    savedTop.current = list.scrollTop;
    setViewport({ top: list.scrollTop, height: list.clientHeight });
  }, [selected, narrow, hasDetail]);

  // 다시 보이면 목록 스크롤을 되돌린다(그리기 전에. 숨은 동안에는 읽을 수 없다).
  useLayoutEffect(() => {
    const list = listRef.current;
    if (!visible || !list) return;
    if (list.scrollTop !== savedTop.current) list.scrollTop = savedTop.current;
    setViewport({ top: list.scrollTop, height: list.clientHeight });
  }, [visible]);

  // 키를 받는 화면이 되면 diff에 포커스를 준다. 키를 놓으면(숨거나 다른 칸으로) 떠 있는 창을 닫는다(돌아왔을 때
  // 포커스 없이 남지 않게).
  useLayoutEffect(() => {
    if (active) {
      diff.current?.focus();
      return;
    }
    setHelp(false);
    setWorktrees(false);
    quickOpen.close();
    palette.close();
    // close는 상태만 바꾼다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  const files = detail?.files ?? [];
  const count = files.length;
  const { start, end } = drawWindow(viewport.top, viewport.height, ROW_HEIGHT, count);
  useLayoutEffect(() => {
    drawnRange.current = { start, end, height: viewport.height };
  });

  const select = (index: number) => {
    const n = latest.current.detail?.files.length ?? 0;
    if (n === 0) return;
    setSelected(Math.max(0, Math.min(n - 1, index)));
  };

  const openBlame = () => {
    const { detail: d, file: f, versions: have, fileKey: key } = latest.current;
    if (!d || !f) return;
    if (f.pathLossy) {
      showToast("경로가 UTF-8이 아니라 blame을 열 수 없다");
      return;
    }
    if (f.status === "deleted") {
      showToast("이 커밋에서 지운 파일이다");
      return;
    }
    if (f.submodule || f.stat?.binary || loaded(have.get(key))?.binary) {
      showToast("하위 모듈·바이너리 파일은 blame하지 않는다");
      return;
    }
    onNavigate({ kind: "file", target: { repo, path: f.path, rev: d.info.sha }, line: diff.current?.firstChangedLine() ?? null });
  };

  /** 고른 파일을 에디터로: 작업트리 파일의 줄(이 커밋 버전의 첫 바뀐 줄을 옮긴 것). 지운 파일은 옛 경로로 본다. */
  const openEditor = () => {
    const { detail: d, file: f } = latest.current;
    if (!d || !f) return;
    if (f.pathLossy) {
      showToast("경로가 UTF-8이 아니라 에디터로 열 수 없다");
      return;
    }
    const deleted = f.status === "deleted";
    openInEditor({
      repo,
      path: f.path,
      rev: deleted ? null : d.info.sha,
      line: deleted ? null : (diff.current?.firstChangedLine() ?? null),
    }).then(showToast);
  };

  const openGraph = () => onNavigate({ kind: "graph", repo, rev: latest.current.detail?.info.sha ?? rev });

  const toggleMode = () =>
    setMode((m) => {
      const next = m === "split" ? "unified" : "split";
      writeMode(next);
      return next;
    });

  /** 이 파일만 ↔ 모든 파일. 고른 파일은 그대로 둔다. 키 처리는 첫 렌더의 함수를 부르므로 latest에서 읽는다. */
  const toggleNarrow = () => {
    const { full: all, detail: shownDetail, selected: at, narrow: isNarrow } = latest.current;
    if (!all || path === null) return;
    const chosen = shownDetail?.files[at];
    if (isNarrow) {
      setNarrow(false);
      setSelected(Math.max(0, all.files.findIndex((f) => f === chosen)));
    } else if (all.files.some((f) => isPath(f, path))) {
      setNarrow(true);
      setSelected(0);
    }
  };

  const copySha = () => {
    const sha = latest.current.detail?.info.sha ?? rev;
    copyText(sha).then(
      () => showToast(`복사했다: ${shortSha(sha)}`),
      (e: unknown) => showToast(String(e)),
    );
  };

  // 키: 화면이 맨 위일 때만. 검색 입력 중이면 넘긴다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { active: on, openFiles, palette: openPalette } = latest.current;
      if (!on || routeKey(e, { files: openFiles, palette: openPalette }) !== "view") return;
      const { selected: at, help: helpOpen } = latest.current;
      let handled = true;
      switch (e.key) {
        case "j":
          select(at + 1);
          break;
        case "k":
          select(at - 1);
          break;
        case "n":
          diff.current?.next();
          break;
        case "N":
          diff.current?.previous();
          break;
        case "u":
          toggleMode();
          break;
        case "o":
          openEditor();
          break;
        case "Enter":
          openBlame();
          break;
        case "a":
          toggleNarrow();
          break;
        case "y":
          copySha();
          break;
        case "G":
          openGraph();
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

  // 도움말·워크트리·Quick Open·팔레트·알림. 상세를 열지 못한 화면에서도 `?`·`W`가 먹게 같이 그린다.
  const overlays = (
    <>
      {help && <Help view="commit" onClose={() => setHelp(false)} />}
      {worktrees && (
        <WorktreePicker
          repo={repo}
          onClose={() => {
            setWorktrees(false);
            focusDiff();
          }}
          onPick={(worktree) => {
            setWorktrees(false);
            // 그 워크트리의 그래프에서 이 커밋을 고른다(객체는 워크트리끼리 같다).
            const next = commitSwitch(worktree, latest.current.detail?.info.sha ?? rev);
            if (next) onNavigate({ ...next, worktree: true });
            else focusDiff();
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
    </>
  );

  if (fatal) {
    return (
      <section className="commit">
        <div className="blame-message error">
          <p>{fatal}</p>
        </div>
        {overlays}
      </section>
    );
  }

  const info = detail?.info;
  const body = info ? info.message.split("\n").slice(1).join("\n").trim() : "";
  const nowAt = Date.now();
  const countedFiles = full && !full.counted ? full.files.filter((f) => f.stat).length : 0;

  // diff에 보이지 않는 바뀜: 모드, 줄 끝·끝 개행·BOM·인코딩, 바뀐 줄이 없는 까닭
  const notes: string[] = [];
  const mode_ = file ? modeNote(file) : null;
  if (mode_) notes.push(mode_);
  if (shown) notes.push(...shown.notes);
  if (file && shown && texts) {
    const raw = { old: shown.old?.text ?? "", new: shown.new?.text ?? "" };
    const same = sameNote(file.status, raw, texts, shown.notes.length > 0);
    if (same) notes.push(same);
  }

  let message: ReactNode = null;
  if (file?.submodule) {
    message = `하위 모듈: ${shortSha(file.oldId ?? "") || "—"} → ${shortSha(file.newId ?? "") || "—"}`;
  } else if (isError(current)) {
    message = (
      <>
        {current.error}{" "}
        <button type="button" className="link" onClick={retry}>
          다시 시도
        </button>
      </>
    );
  } else if (shown?.missing) {
    message = "부분 클론에서 받지 않은 파일이라 내용을 보이지 않는다 (plx는 받아 오지 않는다).";
  } else if (shown && shown.tooLarge !== null) {
    message = (
      <>
        파일이 커서({formatBytes(shown.tooLarge)}) 내용을 비교하지 않았다.{" "}
        <button type="button" className="link" onClick={() => fetchVersions(true)}>
          그래도 보기
        </button>
      </>
    );
  } else if (shown?.binary) {
    message = "바이너리 파일이라 내용을 비교하지 않는다 (NUL 바이트나 .gitattributes의 binary·-diff).";
  }

  return (
    <section className="commit">
      <header className="blame-header">
        <span className="badge mono" title={info?.sha ?? rev}>
          {shortSha(info?.sha ?? rev)}
        </span>
        <span className="commit-summary">{info?.summary ?? "여는 중…"}</span>
        {depth > 1 && (
          <span className="badge" title="q로 앞 화면으로 돌아간다">
            뒤로 q · {depth - 1}
          </span>
        )}
        <span
          className="blame-status"
          title={full && !full.counted ? `줄 수를 세는 중: 파일 ${countedFiles}/${full.files.length}개` : undefined}
        >
          {full && (
            <>
              파일 {full.files.length}개 <span className="added">+{full.additions}</span>{" "}
              <span className="deleted">−{full.deletions}</span>
              {!full.counted && !statsError && " …"}
            </>
          )}
        </span>
        {full && path !== null && full.files.some((f) => isPath(f, path)) && (only || narrow) && (
          <button type="button" className="toggle" onClick={toggleNarrow} title="이 파일만 ↔ 모든 파일 (a)">
            {narrow ? "이 파일만" : "모든 파일"}
          </button>
        )}
        <button type="button" className="toggle" onClick={toggleMode} title="분할·통합 보기 (u)">
          {mode === "split" ? "분할" : "통합"}
        </button>
        <button type="button" className="icon" onClick={() => setHelp(true)} aria-label="단축키 (?)" title="단축키 (?)">
          ?
        </button>
      </header>
      {info && (
        <div className="commit-meta">
          <span>
            {info.author.name} <span className="muted">&lt;{info.author.email}&gt;</span>
          </span>
          <span className="muted">
            {formatDate(info.author.time, info.author.tzOffset)} ({relativeTime(info.author.time, nowAt)})
          </span>
          {info.parents.length > 1 && <span className="badge">merge · 첫 부모와 비교</span>}
          {info.parents.length === 0 && <span className="badge">첫 커밋</span>}
          {full?.renamesLimited && (
            <span
              className="badge warn"
              title="지운 파일 × 새 파일이 git의 rename 한도(1,000²)를 넘어 내용이 비슷한 파일끼리는 rename을 찾지 않았다(git도 건너뛴다). 내용이 같거나 이름이 같은 짝은 찾았다."
            >
              rename 한도 넘음
            </span>
          )}
          {full?.renamesIncomplete && (
            <span className="badge warn" title="부분 클론에서 받지 않은 파일은 내용을 견주지 못해 rename을 다 찾지 못했다.">
              rename 일부만
            </span>
          )}
          {statsError && (
            <span className="badge warn" title={statsError}>
              줄 수를 다 세지 못했다
            </span>
          )}
          {body && <pre className="commit-body">{body}</pre>}
        </div>
      )}
      <div className="commit-main">
        <div
          className="file-list"
          ref={listRef}
          role="listbox"
          aria-label="바뀐 파일"
          onScroll={(e) => {
            const list = e.currentTarget;
            if (latest.current.visible) savedTop.current = list.scrollTop;
            const n = latest.current.detail?.files.length ?? 0;
            // 미리 그린 범위 안이면 브라우저가 옮기기만 한다(다시 그리지 않는다).
            if (insideDrawn(list.scrollTop, list.clientHeight, ROW_HEIGHT, n, drawnRange.current)) return;
            setViewport({ top: list.scrollTop, height: list.clientHeight });
          }}
        >
          <div className="file-rows" style={{ height: count * ROW_HEIGHT }}>
            {files.slice(start, end).map((f, offset) => {
              const i = start + offset;
              return (
                <div
                  key={`${f.status}:${f.oldPath ?? ""}→${f.path}`}
                  role="option"
                  aria-selected={i === selected}
                  className={i === selected ? "file-row selected" : "file-row"}
                  style={{ top: i * ROW_HEIGHT, height: ROW_HEIGHT }}
                  onClick={() => setSelected(i)}
                  title={rowTitle(f)}
                >
                  <span className={`status status-${f.status}`} title={STATUS_NAME[f.status]}>
                    {STATUS_LETTER[f.status]}
                  </span>
                  <FileName change={f} />
                  <StatCell stat={f.stat} />
                </div>
              );
            })}
          </div>
          {detail && count === 0 && <p className="file-empty muted">바뀐 파일이 없다</p>}
        </div>
        <div className="diff-pane">
          {notes.length > 0 && (
            <div className="diff-notes" aria-label="diff에 보이지 않는 바뀜">
              {notes.map((note) => (
                <span key={note} className="badge warn">
                  {note}
                </span>
              ))}
            </div>
          )}
          <div className="diff-body">
            {/* CodeMirror가 직접 다루는 자리. React 자식과 섞지 않는다. */}
            <div className="diff-editor" ref={diffParent} />
            {message && (
              <div className={isError(current) ? "blame-message error" : "blame-message"}>
                <p>{message}</p>
              </div>
            )}
          </div>
        </div>
      </div>
      {overlays}
    </section>
  );
}

function isError(state: VersionsState | undefined): state is { error: string } {
  return typeof state === "object" && "error" in state;
}

const isPath = (f: FileChange, path: string) => f.path === path || f.oldPath === path;

/** `path`가 있으면 그 파일만 남긴 상세. 줄 수 합계는 커밋 전체 그대로다. */
function narrowed(detail: CommitDetail | null, path: string | null): CommitDetail | null {
  if (!detail || path === null) return detail;
  const files = detail.files.filter((f) => isPath(f, path));
  return files.length > 0 ? { ...detail, files } : detail;
}

function rowTitle(f: FileChange): string {
  const name = f.oldPath ? `${f.oldPath} → ${f.path}` : f.path;
  return f.pathLossy ? `${name}\n경로가 UTF-8이 아니라 깨진 글자(�)로 보인다. blame·히스토리·에디터로 열 수 없다.` : name;
}

/** 파일 이름을 먼저, 디렉터리는 흐리게 뒤에(좁은 목록에서도 이름이 보이게). rename은 원래 경로를 함께 보인다. */
function FileName({ change }: { change: FileChange }) {
  const cut = change.path.lastIndexOf("/") + 1;
  return (
    <span className="file-name-cell">
      <span className="file-base">{change.path.slice(cut)}</span>
      {cut > 0 && <span className="file-dir">{change.path.slice(0, cut - 1)}</span>}
      {change.oldPath && <span className="file-dir">← {change.oldPath}</span>}
      {change.pathLossy && <span className="badge warn">UTF-8 아님</span>}
    </span>
  );
}

/** 줄 수. 아직 세지 않았으면 …, 받지 않은 blob이면 ? */
function StatCell({ stat }: { stat: FileStat | null }) {
  if (!stat) {
    return (
      <span className="file-stat" title="줄 수를 세는 중">
        …
      </span>
    );
  }
  if (stat.missing) {
    return (
      <span className="file-stat" title="부분 클론에서 받지 않은 파일이라 줄 수를 모른다">
        ?
      </span>
    );
  }
  if (stat.binary) return <span className="file-stat">바이너리</span>;
  return (
    <span className="file-stat">
      <span className="added">+{stat.additions}</span> <span className="deleted">−{stat.deletions}</span>
    </span>
  );
}
