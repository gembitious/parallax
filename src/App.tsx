import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Hud } from "./Hud";
import { call } from "./ipc";
import { afterPaint, getPerfInfo, initPerf, nowMs, phasesBetween, record } from "./perf";

type AppInfo = { version: string; coreVersion: string };

/** 열 뷰. plx-core/src/cli.rs의 Request와 같은 모양. */
type ViewRequest =
  | { kind: "graph" }
  | { kind: "blame"; path: string; line: number | null }
  | { kind: "log"; path: string }
  | { kind: "lineLog"; path: string; start: number; end: number }
  | { kind: "show"; rev: string };

/** `plx` 호출 하나. src-tauri/src/instance.rs의 PlxRequest와 같은 모양. */
type PlxRequest = {
  id: number;
  request: ViewRequest;
  label: string;
  args: string[];
  cwd: string;
  t0Ms: number;
  recvMs: number;
  cold: boolean;
};

/** 화면에 띄운 요청과 프론트엔드가 그 요청을 받은 시각. */
type Shown = { request: PlxRequest; receivedMs: number };

// docs/MILESTONES.md 성능 예산
const COLD_BUDGET_MS = 1000;
const PLX_BUDGET_MS = 100;

const VIEW_NAMES: Record<ViewRequest["kind"], string> = {
  graph: "커밋 그래프 (M3)",
  blame: "blame (M1)",
  log: "파일 히스토리 (M2)",
  lineLog: "라인 히스토리 (M2)",
  show: "커밋 상세 (M2)",
};

function ViewDetails({ request }: { request: ViewRequest }) {
  switch (request.kind) {
    case "graph":
      return null;
    case "blame":
      return (
        <>
          <dt>파일</dt>
          <dd className="mono">{request.path}</dd>
          <dt>줄</dt>
          <dd className="mono">{request.line ?? "—"}</dd>
        </>
      );
    case "log":
      return (
        <>
          <dt>파일</dt>
          <dd className="mono">{request.path}</dd>
        </>
      );
    case "lineLog":
      return (
        <>
          <dt>파일</dt>
          <dd className="mono">{request.path}</dd>
          <dt>범위</dt>
          <dd className="mono">
            {request.start}–{request.end}
          </dd>
        </>
      );
    case "show":
      return (
        <>
          <dt>리비전</dt>
          <dd className="mono">{request.rev}</dd>
        </>
      );
  }
}

/** plx 호출(또는 콜드 스타트)부터 대상 뷰가 화면에 그려지기까지를 구간별로 남긴다. */
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
        ["렌더", receivedMs],
        [paint, committedMs],
        ["", end],
      ]),
      ok: paintedMs !== null,
    });
  }
}

export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [shown, setShown] = useState<Shown | null>(null);
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
    // 이벤트와 current_request가 같은 요청을 두 번 줄 수 있다. 더 최근 것만 받는다.
    const accept = (request: PlxRequest) => {
      const receivedMs = nowMs();
      setShown((cur) => (cur && cur.request.id >= request.id ? cur : { request, receivedMs }));
    };
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

  // 요청을 DOM에 반영한 직후(페인트 전) 시각을 잡고, 다음 페인트까지 기다려 기록한다.
  useLayoutEffect(() => {
    if (!shown) return;
    const committedMs = nowMs();
    markReady();
    afterPaint().then((paintedMs) => recordShown(shown, committedMs, paintedMs));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown]);

  const request = shown?.request;
  return (
    <div className="app">
      <header className="titlebar">
        <span className="brand">parallax</span>
        {info && <span className="muted">v{info.version}</span>}
      </header>
      <main className="placeholder">
        {request ? (
          <dl className="request">
            <dt>뷰</dt>
            <dd>{VIEW_NAMES[request.request.kind]}</dd>
            <ViewDetails request={request.request} />
            <dt>호출</dt>
            <dd className="mono">plx {request.args.join(" ")}</dd>
            <dt>위치</dt>
            <dd className="mono">{request.cwd}</dd>
          </dl>
        ) : (
          <p className="muted">터미널에서 plx를 실행하면 이 창에 대상 뷰가 열린다.</p>
        )}
      </main>
      <Hud />
    </div>
  );
}
