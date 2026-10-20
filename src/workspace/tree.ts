// 파일 트리: 폴더 목록(백엔드 files_dir, 인덱스의 추적 파일)을 펼친 폴더만큼 행으로 늘어놓고, 키로 옮겨 다닌다.
// 순수 함수라 단위 테스트한다(tree.test.ts).

/** plx-core/src/files.rs DirListing */
export type DirListing = {
  /** 작업트리 경로 */
  repo: string;
  /** 저장소 기준 폴더(루트는 빈 문자열) */
  dir: string;
  dirs: string[];
  files: { name: string; missing: boolean }[];
};

export type TreeRow = {
  /** 저장소 기준 경로 */
  path: string;
  name: string;
  depth: number;
  dir: boolean;
  /** 작업트리에서 지웠다(아직 커밋하지 않았다) */
  missing: boolean;
  /** 펼친 폴더 */
  open: boolean;
  /** 펼쳤는데 아직 목록이 없다 */
  loading: boolean;
};

// 탐색기처럼 숫자는 수로(file2 < file10), 대소문자는 가리지 않고 늘어놓는다. 같다고 보는 이름은 바이트 순으로.
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
export const compareNames = (a: string, b: string) => collator.compare(a, b) || (a < b ? -1 : a > b ? 1 : 0);

/** 받은 목록을 늘어놓을 순서로(폴더·파일 각각 이름 순) 정렬해 둔다. 펼칠 때마다 정렬하지 않는다. */
export function sortListing(listing: DirListing): DirListing {
  return {
    ...listing,
    dirs: [...listing.dirs].sort(compareNames),
    files: [...listing.files].sort((a, b) => compareNames(a.name, b.name)),
  };
}

export const joinPath = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

export function parentOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

/** 경로를 담은 폴더들(바깥부터). `a/b/c.rs` → `a`, `a/b` */
export function ancestorsOf(path: string): string[] {
  const parts = path.split("/");
  return parts.slice(0, -1).map((_, i) => parts.slice(0, i + 1).join("/"));
}

/** 펼친 폴더만큼 행으로 늘어놓는다(폴더 먼저). 목록이 없는 폴더(루트 포함)는 비었다. */
export function treeRows(listings: ReadonlyMap<string, DirListing>, expanded: ReadonlySet<string>): TreeRow[] {
  const rows: TreeRow[] = [];
  const walk = (dir: string, depth: number) => {
    const listing = listings.get(dir);
    if (!listing) return;
    for (const name of listing.dirs) {
      const path = joinPath(dir, name);
      const open = expanded.has(path);
      rows.push({ path, name, depth, dir: true, missing: false, open, loading: open && !listings.has(path) });
      if (open) walk(path, depth + 1);
    }
    for (const file of listing.files) {
      rows.push({
        path: joinPath(dir, file.name),
        name: file.name,
        depth,
        dir: false,
        missing: file.missing,
        open: false,
        loading: false,
      });
    }
  };
  walk("", 0);
  return rows;
}

/** 트리 키가 하는 일 */
export type TreeMove =
  | { kind: "select"; path: string }
  | { kind: "toggle"; path: string }
  /** 파일 열기. `preview`면 미리보기 탭 */
  | { kind: "open"; path: string; missing: boolean; preview: boolean };

/**
 * 고른 행(`selected`, 없으면 null)에서 친 키. ↑↓·j/k 옮기기, →·l 펼치기(펼쳤으면 첫 자식으로), ←·h 접기(접혔거나
 * 파일이면 부모 폴더로), Home·End, Enter 열기(폴더는 펼치기·접기), Space 미리보기. 하는 일이 없으면 null.
 */
export function treeKey(rows: readonly TreeRow[], selected: string | null, key: string, page = 10): TreeMove | null {
  const at = selected === null ? -1 : rows.findIndex((row) => row.path === selected);
  const row = at >= 0 ? rows[at] : undefined;
  const to = (index: number): TreeMove | null => {
    const target = rows[Math.max(0, Math.min(rows.length - 1, index))];
    return target && target.path !== selected ? { kind: "select", path: target.path } : null;
  };
  switch (key) {
    case "ArrowDown":
    case "j":
      return to(at + 1);
    case "ArrowUp":
    case "k":
      return to(at < 0 ? 0 : at - 1);
    case "PageDown":
      return to(at + page);
    case "PageUp":
      return to(at < 0 ? 0 : at - page);
    case "Home":
      return to(0);
    case "End":
      return to(rows.length - 1);
    case "ArrowRight":
    case "l":
      if (!row?.dir) return null;
      if (!row.open) return { kind: "toggle", path: row.path };
      return rows[at + 1]?.depth === row.depth + 1 ? { kind: "select", path: rows[at + 1].path } : null;
    case "ArrowLeft":
    case "h": {
      if (!row) return null;
      if (row.dir && row.open) return { kind: "toggle", path: row.path };
      const parent = parentOf(row.path);
      return parent ? { kind: "select", path: parent } : null;
    }
    case "Enter":
      if (!row) return null;
      return row.dir ? { kind: "toggle", path: row.path } : { kind: "open", path: row.path, missing: row.missing, preview: false };
    case " ":
      if (!row || row.dir) return null;
      return { kind: "open", path: row.path, missing: row.missing, preview: true };
    default:
      return null;
  }
}
