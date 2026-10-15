// 저장소·파일 변화 구독. 백엔드(plx-core/src/watch.rs)가 HEAD·참조가 바뀌면 "repo-changed",
// 지켜보는 파일이 바뀌면 "file-changed" 이벤트를 보낸다.
import { listen } from "@tauri-apps/api/event";
import { call } from "./ipc";

/** plx-core/src/watch.rs RepoChange */
export type RepoChange = {
  /** 작업트리 경로 */
  repo: string;
  head: string | null;
  headChanged: boolean;
  refsChanged: boolean;
};

/** plx-core/src/watch.rs FileChange */
export type FileChange = { path: string };

type Watching = { id: number; target: string };

type Channel<T> = { event: string; key(payload: T): string; subscribers: Map<string, Set<(payload: T) => void>> };

const repoChannel: Channel<RepoChange> = { event: "repo-changed", key: (c) => c.repo, subscribers: new Map() };
const fileChannel: Channel<FileChange> = { event: "file-changed", key: (c) => c.path, subscribers: new Map() };
const listening = new Set<string>();

function subscribe<T>(
  channel: Channel<T>,
  command: string,
  args: Record<string, unknown>,
  onChange: (payload: T) => void,
) {
  if (!listening.has(channel.event)) {
    listening.add(channel.event);
    listen<T>(channel.event, (event) => {
      for (const notify of channel.subscribers.get(channel.key(event.payload)) ?? []) notify(event.payload);
    }).catch((e: unknown) => console.error(`${channel.event}을 듣지 못했다`, e));
  }
  let stopped = false;
  let watching: Watching | null = null;
  call<Watching>(command, args).then(
    (w) => {
      if (stopped) {
        call("unwatch_repo", { id: w.id }).catch(() => {});
        return;
      }
      watching = w;
      let set = channel.subscribers.get(w.target);
      if (!set) channel.subscribers.set(w.target, (set = new Set()));
      set.add(onChange);
    },
    (e: unknown) => console.error("감시 실패", e),
  );
  return () => {
    stopped = true;
    if (!watching) return;
    const set = channel.subscribers.get(watching.target);
    set?.delete(onChange);
    if (set?.size === 0) channel.subscribers.delete(watching.target);
    call("unwatch_repo", { id: watching.id }).catch(() => {});
  };
}

/** `repo`(저장소 안의 아무 경로)의 HEAD·참조가 바뀌면 `onChange`를 부른다. 돌려준 함수로 그만둔다. */
export function watchRepo(repo: string, onChange: (change: RepoChange) => void): () => void {
  return subscribe(repoChannel, "watch_repo", { repo }, onChange);
}

/** 파일 내용이 바뀌면(지우거나 다시 생겨도) `onChange`를 부른다. 돌려준 함수로 그만둔다. */
export function watchFile(path: string, onChange: (change: FileChange) => void): () => void {
  return subscribe(fileChannel, "watch_file", { path }, onChange);
}
