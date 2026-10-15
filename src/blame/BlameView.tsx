// blame 뷰: 파일 내용을 먼저 그리고, 거터를 blame 결과로 점진적으로 채운다.
// 에디터(CodeMirror)는 React 밖에서 직접 다룬다. 이벤트 처리기는 한 번 만들어지므로 최신 상태는 ref로 읽는다.
import { useEffect, useRef, useState } from "react";
import { Help } from "../Help";
import { call } from "../ipc";
import { afterPaint, nowMs, record } from "../perf";
import { CommitCard, type InfoState } from "./CommitCard";
import { createEditor, type Editor } from "./editor";
import { formatCount, isUncommitted, shortSha } from "./format";
import { blameChanged } from "./gutter";
import { BlameModel } from "./model";
import { openBlame } from "./session";
import type { BlameMeta, BlameStats, CommitInfo } from "./types";

type Props = {
  /** 파일 절대 경로(plx 호출에서 온다) */
  path: string;
  /** 커밋 고정 blame이면 리비전, 작업트리면 null */
  rev: string | null;
  /** 처음 선택할 줄 */
  line: number | null;
  /** 콜드 스타트의 첫 요청. 창이 보일 때까지 페인트가 없어 blame 예산과 비교하지 않는다. */
  cold?: boolean;
  /** 첫 화면(내용 또는 오류)을 DOM에 넣었다. App이 첫 페인트를 잰다. */
  onShown(committedMs: number): void;
  /** `q`: 뒤로. 더 돌아갈 곳이 없으면 창을 숨긴다. */
  onBack(): void;
};

type Progress =
  | { kind: "loading" }
  | { kind: "streaming"; blamed: number }
  | { kind: "done" }
  | { kind: "failed"; message: string };

type Hover = { line: number; anchor: DOMRect; pinned: boolean };

/** 에디터 키와 거터가 부르는 동작. 키 처리기는 true를 돌려주면 처리한 것이다. */
type Actions = {
  hover(line: number | null, anchor: DOMRect | null): void;
  pin(line: number): void;
  copySha(): boolean;
  enter(): boolean;
  back(): boolean;
  help(): boolean;
  escape(): boolean;
};

const SHOW_DELAY_MS = 150;
const HIDE_DELAY_MS = 250;
const TOAST_MS = 1800;
/** 성능 예산(docs/MILESTONES.md): blame 첫 줄 표시 ≤ git 첫 출력 + 30ms */
const FIRST_LINE_SLACK_MS = 30;

/** git blame과 같은 줄 수로 보이게 한다: CRLF는 LF로, 끝의 개행 하나는 빈 줄을 만들지 않게 뗀다. */
export function displayText(text: string): { text: string; crlf: boolean } {
  const crlf = text.includes("\r\n");
  let out = crlf ? text.replace(/\r\n/g, "\n") : text;
  if (out.endsWith("\n")) out = out.slice(0, -1);
  return { text: out, crlf };
}

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // 클립보드 API가 막힌 WebView를 위한 대체 경로
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.append(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    if (!ok) throw new Error("클립보드에 쓰지 못했다");
  }
}

export function BlameView({ path, rev, line, cold = false, onShown, onBack }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const editor = useRef<Editor | null>(null);
  const model = useRef<BlameModel | null>(null);
  const nowAtOpen = useRef(Date.now());
  const [meta, setMeta] = useState<BlameMeta | null>(null);
  const [crlf, setCrlf] = useState(false);
  const [fatal, setFatal] = useState<string | null>(null);
  const [progress, setProgress] = useState<Progress>({ kind: "loading" });
  const [hover, setHover] = useState<Hover | null>(null);
  const [infos, setInfos] = useState<ReadonlyMap<string, InfoState>>(() => new Map());
  const [help, setHelp] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

  const timers = useRef({ show: 0, hide: 0, toast: 0 });
  const pointerInCard = useRef(false);
  const latest = useRef({ hover, help, meta, infos });
  latest.current = { hover, help, meta, infos };

  const showToast = (message: string) => {
    setToast(message);
    window.clearTimeout(timers.current.toast);
    timers.current.toast = window.setTimeout(() => setToast(null), TOAST_MS);
  };

  const loadInfo = (sha: string) => {
    const repo = latest.current.meta?.repo;
    if (!repo || isUncommitted(sha) || latest.current.infos.has(sha)) return;
    const put = (value: InfoState) => setInfos((m) => new Map(m).set(sha, value));
    put("loading");
    call<CommitInfo>("commit_info", { repo, rev: sha }).then(put, (e: unknown) => put({ error: String(e) }));
  };

  /** 줄 `n`의 거터 칸 위치(키보드로 카드를 열 때). */
  const anchorOf = (n: number): DOMRect | null => {
    const view = editor.current?.view;
    if (!view) return null;
    const coords = view.coordsAtPos(view.state.doc.line(n).from);
    const gutter = view.dom.querySelector(".cm-blame-gutter")?.getBoundingClientRect();
    if (!coords || !gutter) return null;
    return new DOMRect(gutter.left, coords.top, gutter.width, coords.bottom - coords.top);
  };

  const cursorLine = () => {
    const view = editor.current?.view;
    return view ? view.state.doc.lineAt(view.state.selection.main.head).number : 1;
  };

  const openCard = (n: number, anchor: DOMRect, pinned: boolean) => {
    const sha = model.current?.sha(n);
    if (sha === undefined) return false;
    window.clearTimeout(timers.current.show);
    window.clearTimeout(timers.current.hide);
    setHover({ line: n, anchor, pinned });
    loadInfo(sha);
    return true;
  };

  const closeCard = () => {
    window.clearTimeout(timers.current.show);
    window.clearTimeout(timers.current.hide);
    setHover(null);
    editor.current?.view.focus();
  };

  // 에디터·거터가 부르는 동작. 렌더마다 새로 만들어 최신 상태를 쓴다.
  const actions = useRef<Actions>(null as unknown as Actions);
  actions.current = {
    hover(n, anchor) {
      const current = latest.current.hover;
      if (current?.pinned) return;
      window.clearTimeout(timers.current.show);
      if (n === null || anchor === null) {
        window.clearTimeout(timers.current.hide);
        timers.current.hide = window.setTimeout(() => {
          if (!pointerInCard.current) setHover((h) => (h?.pinned ? h : null));
        }, HIDE_DELAY_MS);
        return;
      }
      if (current?.line === n) return;
      // 카드가 이미 떠 있으면 바로 옮기고, 처음이면 잠깐 기다린다(지나가는 마우스에 깜빡이지 않게).
      if (current) openCard(n, anchor, false);
      else timers.current.show = window.setTimeout(() => openCard(n, anchor, false), SHOW_DELAY_MS);
    },
    pin(n) {
      const anchor = anchorOf(n);
      if (anchor) openCard(n, anchor, true);
    },
    copySha() {
      const n = cursorLine();
      const sha = model.current?.sha(n);
      if (sha === undefined) showToast("이 줄은 아직 blame 정보가 없다");
      else if (isUncommitted(sha)) showToast("커밋하지 않은 줄이다");
      else
        copyText(sha).then(
          () => showToast(`복사했다: ${shortSha(sha)}`),
          (e: unknown) => showToast(String(e)),
        );
      return true;
    },
    enter() {
      const current = latest.current.hover;
      const n = cursorLine();
      if (current?.pinned && current.line === n) {
        closeCard();
        return true;
      }
      const anchor = anchorOf(n);
      if (!anchor || !openCard(n, anchor, true)) showToast("이 줄은 아직 blame 정보가 없다");
      return true;
    },
    back() {
      onBack();
      return true;
    },
    help() {
      setHelp(true);
      return true;
    },
    escape() {
      if (latest.current.help) {
        setHelp(false);
        return true;
      }
      if (latest.current.hover) {
        closeCard();
        return true;
      }
      return false; // 검색 패널이 열려 있으면 닫게 넘긴다
    },
  };

  useEffect(() => {
    const t0 = nowMs();
    let metaMs = NaN;
    let chunkMs = NaN;
    let contentPainted: Promise<number | null> = Promise.resolve(null);
    let gutterPainted: Promise<number | null> = Promise.resolve(null);
    let gotMeta = false;
    let relPath = path;

    const report = async (stats: BlameStats, doneMs: number) => {
      const [content, gutter] = await Promise.all([contentPainted, gutterPainted]);
      const firstLine = gutter ?? content ?? doneMs;
      const phases = [
        { label: "내용 도착", ms: metaMs - t0 },
        { label: "내용 표시", ms: (content ?? NaN) - t0 },
        { label: "첫 묶음", ms: chunkMs - t0 },
        { label: "첫 줄 표시", ms: (gutter ?? NaN) - t0 },
        { label: "끝", ms: doneMs - t0 },
      ].filter((p) => Number.isFinite(p.ms));
      record({
        kind: "blame",
        label: `blame ${relPath}${rev ? ` @${shortSha(rev)}` : ""}${cold ? " (콜드 스타트)" : ""}`,
        totalMs: firstLine - t0,
        budgetMs:
          cold || stats.firstByteMs === null ? undefined : Math.round(stats.firstByteMs + FIRST_LINE_SLACK_MS),
        phases,
        detail: `git 첫 출력 ${stats.firstByteMs?.toFixed(1) ?? "—"} · 블록 ${stats.blocks} · 커밋 ${stats.commits}`,
        ok: gutter !== null || stats.blocks === 0,
      });
    };

    const session = openBlame(path, rev, (event) => {
      switch (event.kind) {
        case "meta": {
          gotMeta = true;
          metaMs = nowMs();
          const { kind: _kind, ...m } = event;
          relPath = m.path;
          setMeta(m);
          const parent = container.current;
          if (m.content && parent) {
            const shown = displayText(m.content.text);
            setCrlf(shown.crlf);
            const blame = new BlameModel(m.content.lineCount);
            model.current = blame;
            editor.current = createEditor(parent, {
              doc: shown.text,
              path: m.path,
              line,
              gutter: {
                model: blame,
                nowMs: nowAtOpen.current,
                onHover: (n, anchor) => actions.current.hover(n, anchor),
                onClick: (n) => actions.current.pin(n),
              },
              keys: {
                copySha: () => actions.current.copySha(),
                enter: () => actions.current.enter(),
                back: () => actions.current.back(),
                help: () => actions.current.help(),
                escape: () => actions.current.escape(),
              },
            });
            editor.current.view.focus();
            setProgress({ kind: "streaming", blamed: 0 });
          } else {
            setProgress({ kind: "done" });
          }
          onShown(nowMs());
          contentPainted = afterPaint();
          break;
        }
        case "chunk": {
          const blame = model.current;
          if (!blame) break;
          blame.add(event);
          editor.current?.view.dispatch({ effects: blameChanged.of(null) });
          if (Number.isNaN(chunkMs)) {
            chunkMs = nowMs();
            gutterPainted = afterPaint();
          }
          setProgress({ kind: "streaming", blamed: blame.blamedLines });
          break;
        }
        case "done":
          setProgress({ kind: "done" });
          void report(event.stats, nowMs());
          break;
        case "error":
          if (gotMeta) {
            setProgress({ kind: "failed", message: event.message });
          } else {
            setFatal(event.message);
            onShown(nowMs());
          }
          break;
        case "cancelled":
          break;
      }
    });

    const t = timers.current;
    return () => {
      session.cancel();
      editor.current?.destroy();
      editor.current = null;
      model.current = null;
      window.clearTimeout(t.show);
      window.clearTimeout(t.hide);
      window.clearTimeout(t.toast);
    };
    // 요청마다 새 컴포넌트다(App이 요청 id로 key를 준다). props가 바뀌어도 다시 열지 않는다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 에디터 밖(카드, 머리줄)에 포커스가 있어도 q·?·Esc는 먹게 한다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest(".cm-editor") || target?.closest("input, textarea")) return;
      const handled =
        e.key === "q" ? actions.current.back() : e.key === "?" ? actions.current.help() : e.key === "Escape" && actions.current.escape();
      if (handled) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const blame = model.current;
  const lineCount = meta?.content?.lineCount ?? 0;
  const cardBlock = hover && blame ? blame.block(hover.line) : undefined;

  return (
    <section className="blame">
      <header className="blame-header">
        <FilePath path={meta?.path ?? path} />
        <span className="badge" title={meta?.rev ?? "작업트리 파일"}>
          {meta?.rev ? shortSha(meta.rev) : rev ? rev : "작업트리"}
        </span>
        {meta?.content && meta.content.encoding !== "UTF-8" && (
          <span className="badge warn" title="파일 인코딩">
            {meta.content.encoding}
          </span>
        )}
        {crlf && <span className="badge">CRLF</span>}
        <span className="blame-status" aria-live="polite">
          <ProgressText progress={progress} blamed={blame?.blamedLines ?? 0} lines={lineCount} commits={blame?.commits.size ?? 0} />
        </span>
        <button type="button" className="icon" onClick={() => setHelp(true)} aria-label="단축키 (?)" title="단축키 (?)">
          ?
        </button>
      </header>
      <div className="blame-body">
        {/* CodeMirror가 직접 다루는 자리. React 자식과 섞지 않는다. */}
        <div className="blame-editor" ref={container} />
        {fatal && (
          <div className="blame-message error">
            <p>{fatal}</p>
          </div>
        )}
        {meta?.binary && (
          <div className="blame-message">
            <p>바이너리 파일이라 내용을 보여 주지 않는다.</p>
          </div>
        )}
      </div>
      {hover && cardBlock && meta && (
        <CommitCard
          commit={blame?.commits.get(cardBlock.sha)}
          block={cardBlock}
          info={infos.get(cardBlock.sha)}
          path={meta.path}
          anchor={hover.anchor}
          pinned={hover.pinned}
          nowMs={nowAtOpen.current}
          onCopy={(sha) =>
            copyText(sha).then(
              () => showToast(`복사했다: ${shortSha(sha)}`),
              (e: unknown) => showToast(String(e)),
            )
          }
          onClose={closeCard}
          onPointerInside={(inside) => {
            pointerInCard.current = inside;
            if (inside) window.clearTimeout(timers.current.hide);
            else if (!latest.current.hover?.pinned) actions.current.hover(null, null);
          }}
        />
      )}
      {help && (
        <Help
          onClose={() => {
            setHelp(false);
            editor.current?.view.focus();
          }}
        />
      )}
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}

function FilePath({ path }: { path: string }) {
  const cut = path.lastIndexOf("/") + 1;
  return (
    <span className="file-path mono" title={path}>
      {cut > 0 && <span className="file-dir">{path.slice(0, cut)}</span>}
      <span className="file-name">{path.slice(cut)}</span>
    </span>
  );
}

function ProgressText({
  progress,
  blamed,
  lines,
  commits,
}: {
  progress: Progress;
  blamed: number;
  lines: number;
  commits: number;
}) {
  switch (progress.kind) {
    case "loading":
      return <>여는 중…</>;
    case "streaming":
      return (
        <>
          blame {formatCount(blamed)} / {formatCount(lines)}줄
        </>
      );
    case "done":
      return lines > 0 ? <>커밋 {formatCount(commits)}개</> : null;
    case "failed":
      return <span className="error-text">blame 실패: {progress.message}</span>;
  }
}
