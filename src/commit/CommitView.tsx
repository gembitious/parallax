// 커밋 상세: 머리(요약·작성자·날짜·sha·메시지), 바뀐 파일 목록, 고른 파일의 diff(분할·통합).
// 파일 히스토리에서 열면 그 파일만 보인다(a로 모든 파일).
// 키: j/k 파일 이동, n/N 다음·이전 바뀐 부분, u 분할↔통합, Enter 그 리비전의 파일 blame, a 이 파일만↔모든 파일,
//     y sha 복사, G 그래프, q 뒤로, ? 도움말.
import { useEffect, useMemo, useRef, useState } from "react";
import { Help } from "../Help";
import { WorktreePicker } from "../worktree/WorktreePicker";
import { isQuickOpenKey, useQuickOpen } from "../quickopen/QuickOpen";
import { copyText } from "../clipboard";
import { displayText } from "../blame/BlameView";
import { formatDate, relativeTime, shortSha } from "../blame/format";
import { call } from "../ipc";
import type { Navigation } from "../nav";
import { afterPaint, nowMs, record } from "../perf";
import { createDiffView, type DiffMode, type DiffView } from "./diff";
import { STATUS_LETTER, STATUS_NAME, type CommitDetail, type FileChange, type FileVersions } from "./types";

type Props = {
  /** 저장소 안의 아무 경로 */
  repo: string;
  rev: string;
  /** 처음 고를 파일(저장소 기준 경로) */
  path: string | null;
  /** `path` 파일만 보인다(파일 히스토리에서 열었다). */
  only?: boolean;
  active: boolean;
  depth: number;
  onShown?(committedMs: number): void;
  onBack(): void;
  onNavigate(navigation: Navigation): void;
};

/** 성능 예산(docs/MILESTONES.md): 커밋 상세(파일 목록 + diff) ≤ 50ms */
const BUDGET_MS = 50;
const TOAST_MS = 1800;
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

export function CommitView({ repo, rev, path, only = false, active, depth, onShown, onBack, onNavigate }: Props) {
  const [full, setFull] = useState<CommitDetail | null>(null);
  /** 이 파일만 보는 중(히스토리에서 열었고 그 파일이 이 커밋에 있다) */
  const [narrow, setNarrow] = useState(only);
  const [fatal, setFatal] = useState<string | null>(null);
  const [selected, setSelected] = useState(0);
  const [mode, setMode] = useState<DiffMode>(readMode);
  const [versions, setVersions] = useState<ReadonlyMap<string, VersionsState>>(() => new Map());
  const [help, setHelp] = useState(false);
  const [worktrees, setWorktrees] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const diffParent = useRef<HTMLDivElement>(null);
  const diff = useRef<DiffView | null>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const quickOpen = useQuickOpen({ repo, current: null, onNavigate });
  const toastTimer = useRef(0);
  /** 첫 diff까지의 측정: 상세 도착 → 목록 표시 → diff 도착 → diff 표시 */
  const timing = useRef({ t0: nowMs(), detailMs: NaN, listPainted: null as Promise<number | null> | null, reported: false });
  const detail = useMemo(() => narrowed(full, narrow ? path : null), [full, narrow, path]);
  const latest = useRef({ active, detail, selected, help, mode, narrow, openFiles: quickOpen.open });
  latest.current = { active, detail, selected, help, mode, narrow, openFiles: quickOpen.open };

  const showToast = (message: string) => {
    setToast(message);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), TOAST_MS);
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

  // 목록을 그린 시각(첫 화면)
  useEffect(() => {
    if (!full) return;
    onShown?.(nowMs());
    timing.current.listPainted = afterPaint();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [full]);

  const file: FileChange | undefined = detail?.files[selected];
  const fileKey = file ? `${file.oldId ?? ""}:${file.newId ?? ""}` : "";
  const current = versions.get(fileKey);

  // 고른 파일의 두 버전을 받는다(한 번 받은 것은 다시 받지 않는다).
  useEffect(() => {
    if (!file || file.submodule || versions.has(fileKey)) return;
    const put = (value: VersionsState) => setVersions((m) => new Map(m).set(fileKey, value));
    put("loading");
    call<FileVersions>("file_versions", { repo, oldId: file.oldId, newId: file.newId }).then(put, (e: unknown) =>
      put({ error: String(e) }),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fileKey]);

  // diff 뷰를 만든다(파일이나 보기 방식이 바뀌면 새로).
  useEffect(() => {
    const parent = diffParent.current;
    if (!parent || !file || !current || current === "loading" || "error" in current || current.binary) return;
    const old = current.old ? displayText(current.old.text).text : "";
    const now = current.new ? displayText(current.new.text).text : "";
    const view = createDiffView(parent, { path: file.path, old, new: now, mode });
    diff.current = view;
    if (latest.current.active) view.focus();
    const t = timing.current;
    if (!t.reported) {
      t.reported = true;
      const diffMs = nowMs();
      void Promise.all([t.listPainted, afterPaint()]).then(([list, painted]) => {
        const end = painted ?? diffMs;
        record({
          kind: "commit",
          label: `커밋 ${shortSha(rev)}`,
          totalMs: end - t.t0,
          budgetMs: BUDGET_MS,
          phases: [
            { label: "상세 도착", ms: t.detailMs - t.t0 },
            { label: "목록 표시", ms: (list ?? NaN) - t.t0 },
            { label: "diff 표시", ms: end - t.t0 },
          ].filter((p) => Number.isFinite(p.ms)),
          detail: `파일 ${latest.current.detail?.files.length ?? 0}개 · ${file.path}`,
          ok: painted !== null,
        });
      });
    }
    return () => {
      view.destroy();
      diff.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fileKey, current, mode]);

  // 고른 파일이 목록에서 보이게 한다.
  useEffect(() => {
    listRef.current?.querySelector(".selected")?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  useEffect(() => {
    if (active) diff.current?.focus();
    else setHelp(false);
  }, [active]);

  const select = (index: number) => {
    const count = latest.current.detail?.files.length ?? 0;
    if (count === 0) return;
    setSelected(Math.max(0, Math.min(count - 1, index)));
  };

  const openBlame = () => {
    const d = latest.current.detail;
    const f = d?.files[latest.current.selected];
    if (!d || !f) return;
    if (f.status === "deleted") {
      showToast("이 커밋에서 지운 파일이다");
      return;
    }
    if (f.submodule || f.binary) {
      showToast("하위 모듈·바이너리 파일은 blame하지 않는다");
      return;
    }
    onNavigate({ kind: "file", target: { repo, path: f.path, rev: d.info.sha }, line: diff.current?.firstChangedLine() ?? null });
  };

  /** 이 파일만 ↔ 모든 파일. 고른 파일은 그대로 둔다. */
  const toggleNarrow = () => {
    const { detail: shown, selected: at, narrow: isNarrow } = latest.current;
    if (!full || path === null) return;
    const chosen = shown?.files[at];
    if (isNarrow) {
      setNarrow(false);
      setSelected(Math.max(0, full.files.findIndex((f) => f === chosen)));
    } else if (full.files.some((f) => isPath(f, path))) {
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
      if (!latest.current.active || e.defaultPrevented) return;
      if (isQuickOpenKey(e)) {
        latest.current.openFiles();
        e.preventDefault();
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if ((e.target as HTMLElement | null)?.closest("input, textarea")) return;
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
          setMode((m) => {
            const next = m === "split" ? "unified" : "split";
            writeMode(next);
            return next;
          });
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
          onNavigate({ kind: "graph", repo, rev: latest.current.detail?.info.sha ?? rev });
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

  if (fatal) {
    return (
      <section className="commit">
        <div className="blame-message error">
          <p>{fatal}</p>
        </div>
        {quickOpen.overlay}
      </section>
    );
  }

  const info = detail?.info;
  const body = info ? info.message.split("\n").slice(1).join("\n").trim() : "";
  const nowAt = Date.now();

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
        <span className="blame-status">
          {full && (
            <>
              파일 {full.files.length}개 <span className="added">+{full.additions}</span>{" "}
              <span className="deleted">−{full.deletions}</span>
            </>
          )}
        </span>
        {full && path !== null && full.files.some((f) => isPath(f, path)) && (only || narrow) && (
          <button type="button" className="toggle" onClick={toggleNarrow} title="이 파일만 ↔ 모든 파일 (a)">
            {narrow ? "이 파일만" : "모든 파일"}
          </button>
        )}
        <button
          type="button"
          className="toggle"
          onClick={() =>
            setMode((m) => {
              const next = m === "split" ? "unified" : "split";
              writeMode(next);
              return next;
            })
          }
          title="분할·통합 보기 (u)"
        >
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
          {body && <pre className="commit-body">{body}</pre>}
        </div>
      )}
      <div className="commit-main">
        <ul className="file-list" ref={listRef} aria-label="바뀐 파일">
          {detail?.files.map((f, i) => (
            <li
              key={`${f.oldPath ?? ""}→${f.path}`}
              className={i === selected ? "selected" : undefined}
              onClick={() => setSelected(i)}
              title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}
            >
              <span className={`status status-${f.status}`} title={STATUS_NAME[f.status]}>
                {STATUS_LETTER[f.status]}
              </span>
              <FileName change={f} />
              <span className="file-stat">
                {f.binary ? (
                  "바이너리"
                ) : (
                  <>
                    <span className="added">+{f.additions}</span> <span className="deleted">−{f.deletions}</span>
                  </>
                )}
              </span>
            </li>
          ))}
          {detail && detail.files.length === 0 && <li className="muted">바뀐 파일이 없다</li>}
        </ul>
        <div className="diff-pane">
          {/* CodeMirror가 직접 다루는 자리. React 자식과 섞지 않는다. */}
          <div className="diff-editor" ref={diffParent} />
          {file && (file.submodule || (current && current !== "loading" && ("error" in current || current.binary))) && (
            <div className="blame-message">
              <p>
                {file.submodule
                  ? `하위 모듈: ${shortSha(file.oldId ?? "") || "—"} → ${shortSha(file.newId ?? "") || "—"}`
                  : current && current !== "loading" && "error" in current
                    ? current.error
                    : "바이너리 파일이라 내용을 비교하지 않는다."}
              </p>
            </div>
          )}
        </div>
      </div>
      {help && <Help view="commit" onClose={() => setHelp(false)} />}
      {worktrees && (
        <WorktreePicker
          repo={repo}
          onClose={() => setWorktrees(false)}
          onPick={(worktree) => {
            setWorktrees(false);
            onNavigate({ kind: "graph", repo: worktree.path, rev: null });
          }}
        />
      )}
      {quickOpen.overlay}
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}

const isPath = (f: FileChange, path: string) => f.path === path || f.oldPath === path;

/** `path`가 있으면 그 파일만 남긴 상세. 줄 수 합계는 커밋 전체 그대로다. */
function narrowed(detail: CommitDetail | null, path: string | null): CommitDetail | null {
  if (!detail || path === null) return detail;
  const files = detail.files.filter((f) => isPath(f, path));
  return files.length > 0 ? { ...detail, files } : detail;
}

/** 파일 이름을 먼저, 디렉터리는 흐리게 뒤에(좁은 목록에서도 이름이 보이게). rename은 원래 경로를 함께 보인다. */
function FileName({ change }: { change: FileChange }) {
  const cut = change.path.lastIndexOf("/") + 1;
  return (
    <span className="file-name-cell">
      <span className="file-base">{change.path.slice(cut)}</span>
      {cut > 0 && <span className="file-dir">{change.path.slice(0, cut - 1)}</span>}
      {change.oldPath && <span className="file-dir">← {change.oldPath}</span>}
    </span>
  );
}
