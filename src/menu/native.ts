// macOS 시스템 메뉴 막대. 메뉴 모형(키를 받는 화면, 명령 이름, 최근 저장소)이 바뀌면 Rust에 보내 다시 만든다
// (src-tauri/src/menu.rs). 항목을 누르면 "menu-action"(항목 id)이 온다. Windows·Linux는 도구 막대의 HTML 메뉴를 쓴다.
import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { subscribeBlameGutter } from "../blame/gutterSetting";
import { call } from "../ipc";
import { closeMenu } from "./Menu";
import { nowMs, record, subscribe as subscribePerf } from "../perf";
import { subscribeRecent } from "../start/open";
import { toNative } from "./model";
import { subscribe } from "./registry";
import { currentMenus } from "./sources";

/** 바뀐 것을 모아 보내는 간격(ms): 탭을 옮기면 화면 둘이 차례로 명령을 낸다. */
const SETTLE_MS = 30;

/**
 * 떠 있는 창(팔레트·Quick Open·도움말·워크트리·가속 설정 확인·라이선스)과 메뉴를 닫는다: 창 바탕을 누른 것과 같다.
 * 창이 떠 있는 동안 화면은 이동을 버린다(그 창에 치던 글자가 새 화면의 키가 된다). 메뉴 막대는 창 위에서도 눌린다.
 */
function closeOverlays(): void {
  closeMenu(false);
  for (const overlay of document.querySelectorAll<HTMLElement>(".overlay")) overlay.click();
}

/** `beforeRun`: 항목을 실행하기 전에 부른다(App이 라이선스 창의 상태를 닫는다). */
export function useNativeMenu(enabled: boolean, beforeRun: () => void): void {
  useEffect(() => {
    if (!enabled) return;
    let actions = new Map<string, () => void>();
    let sent = "";
    let timer = 0;
    let first = true;
    const sync = () => {
      timer = 0;
      const native = toNative(currentMenus(true));
      actions = native.actions;
      const json = JSON.stringify(native.menus);
      if (json === sent) return;
      sent = json;
      const t0 = nowMs();
      const label = first ? "메뉴 막대 만들기" : "메뉴 막대 다시 만들기";
      first = false;
      call("set_app_menu", { menus: native.menus }).then(
        () => record({ kind: "menu", label, totalMs: nowMs() - t0, phases: [], ok: true }),
        (e: unknown) => {
          sent = "";
          console.error("메뉴 막대를 만들지 못했다", e);
          record({ kind: "menu", label, totalMs: nowMs() - t0, phases: [], detail: String(e), ok: false });
        },
      );
    };
    const schedule = () => {
      if (!timer) timer = window.setTimeout(sync, SETTLE_MS);
    };
    const stops = [subscribe(schedule), subscribeRecent(schedule), subscribeBlameGutter(schedule), subscribePerf(schedule)];
    const unlisten = listen<string>("menu-action", (e) => {
      const run = actions.get(e.payload);
      if (!run) return;
      closeOverlays();
      beforeRun();
      run();
    });
    sync();
    return () => {
      window.clearTimeout(timer);
      for (const stop of stops) stop();
      unlisten.then((stop) => stop(), () => {});
    };
    // beforeRun은 ref만 쓴다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled]);
}
