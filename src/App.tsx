import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { appInfo, onRequest, scriptStartMs, type AppInfo } from "./boot";
import { Hud } from "./Hud";
import { MenuHost } from "./menu/Menu";
import { useNativeMenu } from "./menu/native";
import { Notices, useNoticesRequest } from "./notices/Notices";
import { call } from "./ipc";
import { Start } from "./start/Start";
import { isOpenRepoKey } from "./start/keys";
import { openPath, openRepoDialog } from "./start/open";
import { Toasts } from "./Toasts";
import { startUpdateChecks } from "./update/check";
import type { Navigation, PlxRequest, Shown } from "./nav";
import { afterPaint, bootMarks, getPerfInfo, nowMs, phasesBetween, record } from "./perf";
import { IS_MAC } from "./workspace/keys";
import { WorkspaceView } from "./workspace/Workspace";
import { loadLayout } from "./workspace/layout";
import {
  emptyWorkspace,
  setLayoutSource,
  windowTitle,
  workspaceReducer,
  type Place,
  type Workspace,
  type WorkspaceAction,
} from "./workspace/state";

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
        ["JS 읽기", performance.timeOrigin],
        ["JS 실행", scriptStartMs],
        ["시작 IPC", bootMarks.modulesMs],
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
        // plx 실행: 실행 파일을 띄우고 main까지(동적 라이브러리 싣기), 전달: 인자 검사·저장소 열어 보기·소켓으로 넘기기
        ...(request.mainMs > request.t0Ms
          ? ([
              ["plx 실행", request.t0Ms],
              ["전달", request.mainMs],
            ] as [string, number][])
          : ([["plx 실행·전달", request.t0Ms]] as [string, number][])),
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

type Reload = Extract<WorkspaceAction, { type: "reload" }>;

// 저장소를 열 때 그 저장소에 남긴 배치를 되살린다(M7-5).
setLayoutSource(loadLayout);

const firstState = (request: PlxRequest | null): Workspace =>
  request ? workspaceReducer(emptyWorkspace(), { type: "request", request, receivedMs: nowMs() }) : emptyWorkspace();

/** initial: 첫 렌더에 띄울 요청(boot.ts가 물어 둔 것) */
export default function App({ initial }: { initial: PlxRequest | null }) {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [state, dispatch] = useReducer(workspaceReducer, initial, firstState);
  const shownRef = useRef<Shown | null>(null);
  shownRef.current = state.shown;
  const readySent = useRef(false);
  const [notices, setNotices] = useState(false);
  useNoticesRequest(useCallback(() => setNotices(true), []));
  // 라이선스 창은 모든 화면 위에 떠서 키를 다 먹는다. 떠 있는 동안 늦게 온 이동(`<`의 답 등)은 버리고 다시 열기
  // (저장소 변화)는 닫을 때까지 미룬다: 새 화면이 포커스를 가져가 창은 떠 있는데 키는 아래 화면으로 간다.
  // 새 plx 호출은 그 화면을 보이려는 것이라 창을 닫는다.
  const noticesRef = useRef(notices);
  noticesRef.current = notices;
  const deferredReloads = useRef<Reload[]>([]);
  const reload = useCallback((action: Reload) => {
    if (noticesRef.current) deferredReloads.current.push(action);
    else dispatch(action);
  }, []);
  const navigate = useCallback((navigation: Navigation, from: Place) => {
    if (!noticesRef.current) dispatch({ type: "navigate", navigation, from });
  }, []);
  // macOS 메뉴 막대(Windows·Linux는 작업 공간의 도구 막대가 그린다). 항목을 고르면 라이선스 창을 닫고 실행한다.
  useNativeMenu(IS_MAC, () => {
    noticesRef.current = false;
    setNotices(false);
  });
  useEffect(() => {
    if (notices) return;
    const actions = deferredReloads.current;
    deferredReloads.current = [];
    // 그사이 닫혔거나 다시 연 화면(옛 key)의 것은 reducer가 버린다.
    for (const action of actions) dispatch(action);
  }, [notices]);

  // 콜드 스타트에서 창은 첫 화면을 그린 뒤에 보인다(빈 화면 깜빡임 방지).
  const markReady = () => {
    if (readySent.current) return;
    readySent.current = true;
    call("app_ready").catch((e) => console.error("app_ready 실패", e));
  };

  // 새 버전 알림(M8-2): 개발 빌드는 묻지 않는다.
  useEffect(() => {
    if (info) return startUpdateChecks(info.version, info.updateCheck && !import.meta.env.DEV);
  }, [info]);

  useEffect(() => {
    let disposed = false;
    appInfo.then(
      (appInfo) => !disposed && setInfo(appInfo),
      (e) => console.error("앱 정보를 받지 못했다", e),
    );
    // 띄울 화면이 없으면(Finder·시작 메뉴로 띄움) 첫 화면 안내를 보인다.
    if (!initial) markReady();
    const stop = onRequest((request) => {
      setNotices(false);
      dispatch({ type: "request", request, receivedMs: nowMs() });
    });
    return () => {
      disposed = true;
      stop();
    };
    // 첫 렌더의 요청만 본다. markReady는 ref만 쓴다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 화면이 첫 화면을 DOM에 넣은 직후(페인트 전) 시각을 받아, 다음 페인트까지 기다려 기록한다.
  // 창이 숨겨져 있으면 페인트가 없으므로 먼저 창을 보인다.
  // 요청마다 한 번만 잰다(저장소가 바뀌어 다시 연 화면은 재지 않는다).
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

  // 저장소 열기(Ctrl+O·⌘O)는 어느 화면에서든 받는다. 화면들은 수정 키가 붙은 키를 건너뛴다(keys.ts routeKey).
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented || !isOpenRepoKey(e)) return;
      e.preventDefault();
      openRepoDialog();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // 창에 끌어 놓은 폴더(그 저장소의 그래프)나 파일(작업트리 blame)을 연다. 여럿이면 첫 것만.
  const [dragging, setDragging] = useState(false);
  useEffect(() => {
    const stop = getCurrentWebview().onDragDropEvent(({ payload }) => {
      // 파일이 아닌 것(글자, 웹 링크)을 끌면 경로가 없다. 그때는 안내를 띄우지 않는다.
      if (payload.type === "enter") setDragging(payload.paths.length > 0);
      else if (payload.type !== "over") setDragging(false);
      if (payload.type === "drop" && payload.paths.length > 0) openPath(payload.paths[0]);
    });
    return () => {
      stop.then((unlisten) => unlisten(), () => {});
    };
  }, []);

  // 창 제목은 키를 받는 화면을 따른다. plx 요청을 받으면 백엔드가 먼저 그 호출의 요약으로 바꿔 둔다.
  const title = windowTitle(state);
  const titled = useRef(initial?.label ?? null);
  useEffect(() => {
    if (!title || title === titled.current) return;
    titled.current = title;
    invoke("set_window_title", { title }).catch(() => {});
  }, [title]);

  return (
    <div className="app">
      {state.shown || state.repo ? (
        <WorkspaceView
          state={state}
          dispatch={dispatch}
          reload={reload}
          navigate={navigate}
          onShown={onShown}
          hideWindow={hideWindow}
        />
      ) : (
        <Start version={info?.version ?? null} />
      )}
      {notices && <Notices onClose={() => setNotices(false)} />}
      {dragging && (
        <div className="drop-overlay" aria-hidden="true">
          <p>놓으면 연다: 폴더는 그 저장소의 그래프, 파일은 blame</p>
        </div>
      )}
      <MenuHost />
      <Toasts />
      <Hud />
    </div>
  );
}
