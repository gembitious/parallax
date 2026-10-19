// 앱 안에서 저장소 열기(OS 폴더 창, 끌어 놓기, 최근 저장소)와 터미널 명령 설치. 백엔드는 src-tauri/src/open.rs이고,
// 연 대상은 plx 호출과 같은 요청으로 온다(boot.ts onRequest). 최근 저장소 목록은 백엔드가 들고, 바뀌면
// "recent-changed"를 보낸다. 창으로 돌아올 때도 다시 읽는다(그사이 폴더를 지웠거나 볼륨을 꺼냈을 수 있다).
import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { call } from "../ipc";
import { toast } from "../Toasts";

/** src-tauri/src/open.rs RecentRepo */
export type RecentRepo = {
  path: string;
  /** 폴더 이름 */
  name: string;
  /** 화면에 보일 경로(홈은 `~`) */
  display: string;
  /** 폴더가 없다 */
  missing: boolean;
};

/** plx-core/src/cli_install.rs Status */
export type CliStatus = {
  installed: boolean;
  location: string;
  /** 설치하면 할 일 */
  plan: string;
  blocked: string | null;
  note: string | null;
};

type RecentState = { repos: RecentRepo[]; loaded: boolean };

let state: RecentState = { repos: [], loaded: false };
const subscribers = new Set<() => void>();
let started = false;

function refresh() {
  call<RecentRepo[]>("recent_repos").then(
    (repos) => {
      state = { repos, loaded: true };
      for (const notify of subscribers) notify();
    },
    (e) => console.error("최근 저장소를 읽지 못했다", e),
  );
}

function start() {
  if (started) return;
  started = true;
  listen("recent-changed", refresh).catch((e) => console.error("최근 저장소 변화를 듣지 못했다", e));
  window.addEventListener("focus", refresh);
  refresh();
}

/** 최근 저장소가 바뀌면 부른다(처음 부르면 읽기 시작한다). */
export function subscribeRecent(notify: () => void) {
  start();
  subscribers.add(notify);
  return () => {
    subscribers.delete(notify);
  };
}

/** 첫 화면: 최근 저장소(최근 순)와 처음 읽기를 마쳤는지 */
export function useRecentRepos(): RecentState {
  return useSyncExternalStore(subscribeRecent, () => state);
}

/** 지금 아는 최근 저장소. 명령 팔레트가 렌더할 때 읽는다(처음 부르면 읽기 시작한다). */
export function recentRepos(): RecentRepo[] {
  start();
  return state.repos;
}

let dialogOpen = false;

/** OS 폴더 창으로 저장소를 골라 연다(Ctrl+O·⌘O). 창이 이미 떠 있으면 아무것도 하지 않는다. */
export function openRepoDialog() {
  if (dialogOpen) return;
  dialogOpen = true;
  call<boolean>("open_dialog")
    .catch((e) => toast(String(e), true))
    .finally(() => {
      dialogOpen = false;
    });
}

/** 폴더(그 저장소의 그래프)나 파일(작업트리 blame)을 연다. 저장소가 아니면 알린다. */
export function openPath(path: string) {
  call("open_path", { path }).catch((e) => toast(String(e), true));
}

/** 최근 저장소에서 뺀다(폴더는 그대로다). */
export function forgetRecent(path: string) {
  call("forget_recent", { path }).catch((e) => toast(String(e), true));
}

export function cliStatus(): Promise<CliStatus> {
  return call<CliStatus>("cli_status");
}

/** 터미널 명령 `plx`를 설치하고 결과를 알린다. macOS는 관리자 암호 창이 뜰 수 있다. */
export async function installCli(): Promise<CliStatus | null> {
  try {
    const status = await call<CliStatus>("install_cli");
    toast(`터미널 명령 plx를 쓸 수 있다: ${status.location}${status.note ? ` · ${status.note}` : ""}`);
    return status;
  } catch (e) {
    toast(`터미널 명령을 설치하지 못했다: ${String(e)}`, true);
    return null;
  }
}
