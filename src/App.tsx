import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

/** 더 최근 요청만 받는다. 이벤트와 current_request가 같은 요청을 두 번 줄 수 있다. */
function newer(current: PlxRequest | null, next: PlxRequest): PlxRequest | null {
  return current && current.id >= next.id ? current : next;
}

export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [request, setRequest] = useState<PlxRequest | null>(null);

  useEffect(() => {
    invoke<AppInfo>("app_info").then(setInfo, (e) => console.error("app_info 실패", e));
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    (async () => {
      // 먼저 듣기 시작한 뒤 놓친 요청을 가져온다. 순서를 바꾸면 그 사이에 온 요청을 잃는다.
      unlisten = await listen<PlxRequest>("plx-request", (e) => setRequest((cur) => newer(cur, e.payload)));
      if (disposed) {
        unlisten();
        return;
      }
      const latest = await invoke<PlxRequest | null>("current_request");
      if (!disposed && latest) setRequest((cur) => newer(cur, latest));
    })().catch((e) => console.error("plx-request 구독 실패", e));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

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
    </div>
  );
}
