// 파일 트리·탭의 변경 표시(M13): 백엔드 worktree_status(HEAD ↔ 작업트리에서 바뀐 파일)를 경로·폴더로 찾게 모으고,
// 인덱스에 없는 추적하지 않는 파일을 트리 목록에 더한다. 순수 함수라 단위 테스트한다(status.test.ts).
// 구독(작업트리·저장소 감시, 창 포커스)은 useWorktreeStatus.ts가 한다.
import { parentOf, sortListing, type DirListing } from "./tree";

export type ChangeKind = "modified" | "added" | "deleted" | "untracked" | "conflicted";

/** plx-core/src/changes.rs WorktreeStatus */
export type WorktreeStatus = {
  head: string | null;
  /** 경로 순 */
  files: { path: string; kind: ChangeKind }[];
  untrackedOmitted: number;
};

export type StatusIndex = {
  /** 저장소 기준 경로 → 상태 */
  files: ReadonlyMap<string, ChangeKind>;
  /** 바뀐 파일이 든 폴더(루트는 빼고) */
  dirs: ReadonlySet<string>;
  /** 폴더 → 그 바로 아래의 추적하지 않는 파일·폴더 이름(트리에 더한다) */
  untracked: ReadonlyMap<string, { dirs: ReadonlySet<string>; files: ReadonlySet<string> }>;
  /** 바뀐 파일 수(추적하지 않는 파일 포함, 보이지 않은 것은 빼고) */
  count: number;
};

export const EMPTY_STATUS: StatusIndex = { files: new Map(), dirs: new Set(), untracked: new Map(), count: 0 };

export function indexStatus(status: WorktreeStatus): StatusIndex {
  const files = new Map<string, ChangeKind>();
  const dirs = new Set<string>();
  const untracked = new Map<string, { dirs: Set<string>; files: Set<string> }>();
  const slot = (dir: string) => {
    let entry = untracked.get(dir);
    if (!entry) untracked.set(dir, (entry = { dirs: new Set(), files: new Set() }));
    return entry;
  };
  const nameIn = (dir: string, path: string) => path.slice(dir ? dir.length + 1 : 0);
  for (const { path, kind } of status.files) {
    files.set(path, kind);
    let dir = parentOf(path);
    if (kind === "untracked") slot(dir).files.add(nameIn(dir, path));
    while (dir) {
      dirs.add(dir);
      const parent = parentOf(dir);
      if (kind === "untracked") slot(parent).dirs.add(nameIn(parent, dir));
      dir = parent;
    }
  }
  return { files, dirs, untracked, count: files.size };
}

/** 폴더 목록에 인덱스에 없는 추적하지 않는 파일·폴더를 더한다(이름 순을 지킨다). 더할 것이 없으면 그대로 돌려준다. */
export function withUntracked(listing: DirListing, index: StatusIndex): DirListing {
  const extra = index.untracked.get(listing.dir);
  if (!extra) return listing;
  const dirs = [...extra.dirs].filter((name) => !listing.dirs.includes(name));
  const known = new Set(listing.files.map((f) => f.name));
  const files = [...extra.files].filter((name) => !known.has(name)).map((name) => ({ name, missing: false }));
  if (dirs.length === 0 && files.length === 0) return listing;
  return sortListing({ ...listing, dirs: [...listing.dirs, ...dirs], files: [...listing.files, ...files] });
}

/** 상태 글자(VS Code와 같다. 충돌은 VS Code의 `!`) */
export const CHANGE_LETTER: Record<ChangeKind, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  untracked: "U",
  conflicted: "!",
};

export const CHANGE_TITLE: Record<ChangeKind, string> = {
  modified: "HEAD 뒤로 바꿨다",
  added: "새 파일 (인덱스에 올렸다)",
  deleted: "작업트리에서 지웠다",
  untracked: "추적하지 않는 새 파일",
  conflicted: "충돌: 고친 뒤 git add로 해결했다고 알린다",
};

