// 저장소·파일 변화 구독. 백엔드(plx-core/src/watch.rs)가 HEAD·참조가 바뀌면 "repo-changed",
// 지켜보는 파일이 바뀌면 "file-changed", 지켜보는 작업트리(.git 밖)의 파일이 바뀌면 "tree-changed" 이벤트를 보낸다.
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

/** plx-core/src/watch.rs TreeChange */
export type TreeChange = { repo: string };

type Watching = { id: number; target: string };

/**
 * 구독을 끝낸 뒤 백엔드 감시를 이만큼 더 둔다. 화면을 다시 열면(변화가 와서 reload) 새 화면이 같은 감시를 이어받아,
 * 그 변화 때의 지문과 견준다. 감시를 바로 버리면 새 화면이 자기 데이터를 읽은 뒤에야 새 지문을 재서, 그사이의
 * 변화(예: 저장 직후 포매터가 다시 쓴 파일)를 놓친다.
 */
const UNWATCH_DELAY_MS = 2000;

type Channel<T> = { event: string; key(payload: T): string; subscribers: Map<string, Set<(payload: T) => void>> };

const repoChannel: Channel<RepoChange> = { event: "repo-changed", key: (c) => c.repo, subscribers: new Map() };
const fileChannel: Channel<FileChange> = { event: "file-changed", key: (c) => c.path, subscribers: new Map() };
const treeChannel: Channel<TreeChange> = { event: "tree-changed", key: (c) => c.repo, subscribers: new Map() };
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
    const { id } = watching;
    window.setTimeout(() => call("unwatch_repo", { id }).catch(() => {}), UNWATCH_DELAY_MS);
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

/** `repo`(저장소 안의 아무 경로)의 작업트리에서 파일이 생기거나 바뀌거나 없어지면 `onChange`를 부른다(작업트리 변경 화면). */
export function watchTree(repo: string, onChange: (change: TreeChange) => void): () => void {
  return subscribe(treeChannel, "watch_tree", { repo }, onChange);
}

/**
 * HEAD가 움직였다는 알림으로 화면을 다시 열지. 감시는 구독이 끝나도 잠시 남아 그때의 지문을 이어받으므로
 * (`UNWATCH_DELAY_MS`), 화면을 열기 바로 전에 만든 커밋의 알림이 화면이 열린 뒤에 올 수 있다(`git commit && plx log …`).
 * 화면이 이미 그 HEAD로 읽었으면 다시 열지 않는다.
 * - `movedTo`: 마지막 알림의 HEAD. 알림이 없었으면 undefined
 * - `seen`: 화면이 읽은 기준 HEAD. 아직 모르면(읽는 중) undefined, 커밋이 없는 저장소면 null
 * - `failed`: 열기에 실패했다. 기준이 없으니 HEAD가 움직이면 다시 연다(체크아웃으로 돌아온 파일 등).
 */
export function headMoved(movedTo: string | null | undefined, seen: string | null | undefined, failed: boolean): boolean {
  if (movedTo === undefined) return false;
  if (failed) return true;
  return seen !== undefined && movedTo !== seen;
}
