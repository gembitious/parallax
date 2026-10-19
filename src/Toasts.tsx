// 화면 아래 알림: 앱 안에서 연 대상이 저장소가 아닐 때, 터미널 명령 설치 결과 등. 결과를 돌려받을 호출이 없는
// 백엔드 일(Dock에 놓은 폴더, 트레이 메뉴)은 "notice" 이벤트로 온다(src-tauri/src/open.rs).
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

const EVENT = "plx-toast";
const SHOW_MS = 6000;
const ERROR_SHOW_MS = 12000;
/** 한 번에 보이는 알림 수. 넘으면 오래된 것부터 내린다. */
const MAX = 3;

type Notice = { text: string; error: boolean };
type Toast = Notice & { id: number };

/** 어느 화면에서든 알림을 띄운다. */
export function toast(text: string, error = false) {
  window.dispatchEvent(new CustomEvent<Notice>(EVENT, { detail: { text, error } }));
}

let nextId = 1;

/** App이 한 번 그린다. 누르면 닫는다. */
export function Toasts() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  useEffect(() => {
    const timers = new Set<ReturnType<typeof setTimeout>>();
    const add = (notice: Notice) => {
      const id = nextId++;
      setToasts((list) => [...list.slice(-(MAX - 1)), { ...notice, id }]);
      const timer = setTimeout(
        () => {
          timers.delete(timer);
          setToasts((list) => list.filter((t) => t.id !== id));
        },
        notice.error ? ERROR_SHOW_MS : SHOW_MS,
      );
      timers.add(timer);
    };
    const onToast = (e: Event) => add((e as CustomEvent<Notice>).detail);
    window.addEventListener(EVENT, onToast);
    const stop = listen<Notice>("notice", (e) => add(e.payload));
    return () => {
      window.removeEventListener(EVENT, onToast);
      stop.then((unlisten) => unlisten(), () => {});
      for (const timer of timers) clearTimeout(timer);
    };
  }, []);
  if (toasts.length === 0) return null;
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`app-toast${t.error ? " error" : ""}`}
          onClick={() => setToasts((list) => list.filter((x) => x.id !== t.id))}
        >
          {t.text}
        </div>
      ))}
    </div>
  );
}
