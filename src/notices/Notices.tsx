// 오픈 소스 라이선스: 릴리스 빌드가 넣은 제3자 저작권 고지(scripts/third-party-notices.mjs가 만든
// public/third-party-notices.txt)를 보인다. 명령 팔레트·도움말·첫 화면에서 연다(App이 그린다).
// 열려 있는 동안 키를 모두 먹는다: 화면들은 window에서 키를 받으므로 여기서 막지 않으면 아래 화면이 움직인다.
import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { isNotices, noticeKey } from "./keys";

const OPEN_EVENT = "plx-open-notices";
const NOTICES_URL = "/third-party-notices.txt";
const LINE_PX = 18;

/** 어느 화면에서든 오픈 소스 라이선스 창을 연다. */
export function openNotices() {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

/** App: 창을 열라는 요청을 받는다. */
export function useNoticesRequest(open: () => void) {
  useEffect(() => {
    window.addEventListener(OPEN_EVENT, open);
    return () => window.removeEventListener(OPEN_EVENT, open);
  }, [open]);
}

type Loaded = { kind: "loading" } | { kind: "text"; text: string } | { kind: "missing" } | { kind: "error"; message: string };

export function Notices({ onClose }: { onClose(): void }) {
  const [loaded, setLoaded] = useState<Loaded>({ kind: "loading" });
  const body = useRef<HTMLDivElement>(null);

  // 열 때 포커스를 가져오고, 닫으면 전에 포커스가 있던 곳(아래 화면)으로 돌려준다.
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    body.current?.focus({ preventScroll: true });
    return () => previous?.focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    let stale = false;
    fetch(NOTICES_URL)
      .then(async (response) => {
        const text = response.ok ? await response.text() : "";
        if (!stale) setLoaded(isNotices(text) ? { kind: "text", text } : { kind: "missing" });
      })
      .catch((e: unknown) => {
        if (!stale) setLoaded({ kind: "error", message: String(e) });
      });
    return () => {
      stale = true;
    };
  }, []);

  const onKeyDown = useCallback(
    (e: KeyboardEvent) => {
      const action = noticeKey(e.nativeEvent);
      if (action.kind === "pass") return;
      e.preventDefault();
      e.stopPropagation();
      const scroller = body.current;
      if (action.kind === "close") onClose();
      else if (!scroller) return;
      else if (action.kind === "lines") scroller.scrollBy({ top: action.count * LINE_PX });
      else if (action.kind === "pages") scroller.scrollBy({ top: action.count * scroller.clientHeight * 0.9 });
      else if (action.kind === "edge") scroller.scrollTo({ top: action.to === "top" ? 0 : scroller.scrollHeight });
    },
    [onClose],
  );

  return (
    <div className="overlay" role="dialog" aria-label="오픈 소스 라이선스" onClick={onClose}>
      <div className="help notices" onClick={(e) => e.stopPropagation()} onKeyDown={onKeyDown}>
        <h2>
          오픈 소스 라이선스 <span className="muted">(j/k · PageUp/PageDown · Home/End · Esc 닫기)</span>
        </h2>
        <div ref={body} className="notices-body" tabIndex={-1}>
          {loaded.kind === "loading" ? (
            <p className="muted">읽는 중…</p>
          ) : loaded.kind === "missing" ? (
            <p className="muted">
              이 빌드에는 고지가 들어 있지 않다. 릴리스 빌드(<code>npm run tauri build</code>)가 만들어 넣고, 개발
              중에는 <code>npm run notices</code>로 만든다.
            </p>
          ) : loaded.kind === "error" ? (
            <p className="error-text">고지를 읽지 못했다: {loaded.message}</p>
          ) : (
            <pre>{loaded.text}</pre>
          )}
        </div>
      </div>
    </div>
  );
}
