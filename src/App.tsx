import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BlameView } from "./blame/BlameView";
import { CommitView } from "./commit/CommitView";
import { GraphView } from "./graph/GraphView";
import { HistoryView } from "./history/HistoryView";
import { Hud } from "./Hud";
import { call } from "./ipc";
import {
  backDirectionOf,
  titleOf,
  viewReducer,
  type Navigation,
  type PlxRequest,
  type Shown,
  type StackEntry,
} from "./nav";
import { afterPaint, getPerfInfo, initPerf, nowMs, phasesBetween, record } from "./perf";

type AppInfo = { version: string; coreVersion: string };

// docs/MILESTONES.md 성능 예산
const COLD_BUDGET_MS = 1000;
const PLX_BUDGET_MS = 100;

/** plx 호출(또는 콜드 스타트)부터 대상 뷰의 첫 화면(내용 또는 오류)이 그려지기까지를 구간별로 남긴다. */
function recordShown({ request, receivedMs }: Shown, committedMs: number, paintedMs: number | null) {
  const end = paintedMs ?? committedMs;
  const paint = paintedMs === null ? "페인트(측정 못 함)" : "페인트";
  if (request.cold) {
    record({
      kind: "cold",
      label: `콜드 스타트 · ${request.label}`,
      budgetMs: COLD_BUDGET_MS,
      totalMs: end - request.t0Ms,
      phases: phasesBetween([
        ["프로세스", request.t0Ms],
        ["Tauri·WebView", getPerfInfo().mainStartMs],
        ["JS·첫 렌더", performance.timeOrigin],
        ["내용", receivedMs],
        [paint, committedMs],
        ["", end],
      ]),
      ok: paintedMs !== null,
    });
  } else {
    record({
      kind: "plx",
      label: request.label,
      budgetMs: PLX_BUDGET_MS,
      totalMs: end - request.t0Ms,
      phases: phasesBetween([
        ["plx 실행·전달", request.t0Ms],
        ["이벤트", request.recvMs],
        ["내용", receivedMs],
        [paint, committedMs],
        ["", end],
      ]),
      ok: paintedMs !== null,
    });
  }
}

const hideWindow = () => invoke("hide_window").catch((e) => console.error("창 숨기기 실패", e));

export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [{ shown, stack }, dispatch] = useReducer(viewReducer, { shown: null, stack: [] });
  const shownRef = useRef<Shown | null>(null);
  shownRef.current = shown;
  const readySent = useRef(false);

  // 콜드 스타트에서 창은 첫 화면을 그린 뒤에 보인다(빈 화면 깜빡임 방지).
  const markReady = () => {
    if (readySent.current) return;
    readySent.current = true;
    call("app_ready").catch((e) => console.error("app_ready 실패", e));
  };

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const accept = (request: PlxRequest) => dispatch({ type: "request", request, receivedMs: nowMs() });
    (async () => {
      await initPerf();
      // 먼저 듣기 시작한 뒤 놓친 요청을 가져온다. 순서를 바꾸면 그 사이에 온 요청을 잃는다.
      unlisten = await listen<PlxRequest>("plx-request", (e) => accept(e.payload));
      if (disposed) {
        unlisten();
        return;
      }
      const [latest, appInfo] = await Promise.all([
        call<PlxRequest | null>("current_request"),
        call<AppInfo>("app_info"),
      ]);
      if (disposed) return;
      setInfo(appInfo);
      if (latest) accept(latest);
      else markReady();
    })().catch((e) => {
      console.error("초기화 실패", e);
      markReady();
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
    // markReady는 ref만 쓰므로 의존성이 없다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 뷰가 첫 화면을 DOM에 넣은 직후(페인트 전) 시각을 받아, 다음 페인트까지 기다려 기록한다.
  // 창이 숨겨져 있으면 페인트가 없으므로 먼저 창을 보인다.
  // 요청마다 한 번만 잰다(저장소가 바뀌어 다시 연 맨 아래 화면은 재지 않는다).
  const measured = useRef(new Set<number>());
  const onShown = useCallback((requestId: number, committedMs: number) => {
    markReady();
    if (measured.current.has(requestId)) return;
    measured.current.add(requestId);
    afterPaint().then((paintedMs) => {
      const current = shownRef.current;
      if (current?.request.id === requestId) recordShown(current, committedMs, paintedMs);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 탐색으로 화면이 바뀌면 창 제목도 바꾼다. plx 호출로 연 맨 아래 화면은 호출 요약(백엔드가 처음 정한 제목)이다.
  const top = stack[stack.length - 1];
  const rootLabel = shown?.request.label;
  const titled = useRef<StackEntry | null>(null);
  useEffect(() => {
    if (!top || titled.current === top) return;
    const first = titled.current === null;
    titled.current = top;
    if (top.via === "request" && first) return;
    const title = top.via === "request" ? rootLabel : titleOf(top);
    if (title) invoke("set_window_title", { title }).catch(() => {});
  }, [top, rootLabel]);

  const back = () => {
    if (stack.length > 1) dispatch({ type: "back" });
    else hideWindow();
  };

  const request = shown?.request;
  return (
    <div className="app">
      {request ? (
        <main className="view">
          {stack.map((entry, i) => {
            const isTop = i === stack.length - 1;
            const common = {
              active: isTop,
              depth: i + 1,
              onShown: entry.via === "request" ? (ms: number) => onShown(request.id, ms) : undefined,
              onBack: back,
              onNavigate: (navigation: Navigation) => dispatch({ type: "navigate", navigation }),
              onReload: (line?: number | null) => dispatch({ type: "reload", key: entry.key, line }),
            };
            const cold = entry.via === "request" && request.cold;
            return (
              <div key={entry.key} className="stack-entry" hidden={!isTop}>
                {entry.view === "blame" ? (
                  <BlameView
                    {...common}
                    target={entry.target}
                    line={entry.line}
                    backDirection={backDirectionOf(entry.via)}
                    cold={cold}
                  />
                ) : entry.view === "history" ? (
                  <HistoryView
                    {...common}
                    onReload={(lines) => dispatch({ type: "reload", key: entry.key, lines })}
                    target={entry.target}
                    lines={entry.lines}
                    cold={cold}
                  />
                ) : entry.view === "graph" ? (
                  <GraphView {...common} repo={entry.repo} focus={entry.focus} cold={cold} />
                ) : (
                  <CommitView {...common} repo={entry.repo} rev={entry.rev} path={entry.path} only={entry.only} />
                )}
              </div>
            );
          })}
        </main>
      ) : (
        <main className="empty">
          <p>
            <span className="brand">parallax</span>
            {info && <span className="muted"> v{info.version}</span>}
          </p>
          <p className="muted">터미널에서 plx를 실행하면 이 창에 대상 뷰가 열린다.</p>
        </main>
      )}
      <Hud />
    </div>
  );
}
