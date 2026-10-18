// 커밋 상세 화면의 순수 계산: 나중에 센 줄 수 넣기, diff에 보이지 않는 바뀜 설명, 받은 두 버전의 기억 한도.
// 단위 테스트한다(detail.test.ts).
import type { CommitDetail, FileChange, FileStat, FileStatus, FileVersions, StatRequest } from "./types";

/** 세지 않은 파일의 줄 수를 한 번에 요청하는 수. 커밋이 크면 키워 화면 갱신을 STATS_UPDATES번 안팎으로 둔다. */
export const STATS_BATCH_MIN = 200;
const STATS_UPDATES = 20;

/** 아직 세지 않은 파일 번호들을 요청 묶음으로 나눈다(목록 순서: 위에 보이는 것부터 센다). */
export function statBatches(files: readonly FileChange[]): number[][] {
  const pending: number[] = [];
  files.forEach((f, i) => {
    if (!f.stat) pending.push(i);
  });
  const size = Math.max(STATS_BATCH_MIN, Math.ceil(pending.length / STATS_UPDATES));
  const batches: number[][] = [];
  for (let i = 0; i < pending.length; i += size) batches.push(pending.slice(i, i + size));
  return batches;
}

export function statRequest(f: FileChange): StatRequest {
  return { path: f.path, oldPath: f.oldPath, oldId: f.oldId, newId: f.newId, submodule: f.submodule };
}

/** `indices` 파일에 받은 줄 수(같은 순서)를 넣고 합계를 다시 낸 상세. 다른 파일은 그대로(같은 객체)다. */
export function withStats(detail: CommitDetail, indices: readonly number[], stats: readonly FileStat[]): CommitDetail {
  const files = detail.files.slice();
  indices.forEach((index, k) => {
    const stat = stats[k];
    if (stat && files[index]) files[index] = { ...files[index], stat };
  });
  let additions = 0;
  let deletions = 0;
  let counted = true;
  for (const f of files) {
    if (f.stat) {
      additions += f.stat.additions;
      deletions += f.stat.deletions;
    } else {
      counted = false;
    }
  }
  return { ...detail, files, additions, deletions, counted };
}

const MODE_NAME: Record<string, string> = {
  "100644": "파일",
  "100755": "실행 파일",
  "120000": "심볼릭 링크",
  "160000": "하위 모듈",
};

/** 파일 모드가 바뀌었으면 그 설명(줄 diff에는 보이지 않는다). */
export function modeNote(f: Pick<FileChange, "oldMode" | "newMode">): string | null {
  const { oldMode: before, newMode: after } = f;
  if (!before || !after || before === after) return null;
  if (before === "100644" && after === "100755") return "실행 권한을 켰다 (100644 → 100755)";
  if (before === "100755" && after === "100644") return "실행 권한을 껐다 (100755 → 100644)";
  const name = (mode: string) => MODE_NAME[mode] ?? mode;
  return `종류: ${name(before)} → ${name(after)} (${before} → ${after})`;
}

/**
 * diff에 바뀐 줄이 없을 때의 설명. `raw`는 받은 그대로, `shown`은 화면에 보이는 글(CRLF·끝 개행 정리).
 * `explained`면 다른 알림(줄 끝·BOM·인코딩)이 이미 설명하므로 바이트가 다르다고 따로 말하지 않는다.
 */
export function sameNote(
  status: FileStatus,
  raw: { old: string; new: string },
  shown: { old: string; new: string },
  explained: boolean,
): string | null {
  if (status === "added" || status === "deleted") return raw.old === "" && raw.new === "" ? "빈 파일" : null;
  if (raw.old === raw.new) return "내용은 그대로다";
  if (shown.old === shown.new && !explained) return "보이는 글자는 같다 (바이트가 다르다)";
  return null;
}

/** 바이트 수를 읽기 좋게 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)}MB`;
}

/** 받은 두 버전을 기억하는 수와 글자 수 한도 */
export type Limits = { entries: number; chars: number };

export const VERSIONS_LIMITS: Limits = { entries: 48, chars: 32 << 20 };

/** 기억한 두 버전이 차지하는 글자 수(받는 중·실패는 0) */
export function versionsSize(value: unknown): number {
  if (typeof value !== "object" || value === null || !("notes" in value)) return 0;
  const v = value as FileVersions;
  return (v.old?.text.length ?? 0) + (v.new?.text.length ?? 0);
}

/**
 * `key`를 맨 뒤(가장 최근)에 넣은 새 표. 한도를 넘으면 앞(오래된 것)부터 잊는다. `key`와 `keep`(지금 고른
 * 파일)은 잊지 않는다: 늦게 온 다른 파일의 답이 보고 있는 파일을 밀어내면 diff가 비기 때문이다.
 */
export function remember<V>(
  map: ReadonlyMap<string, V>,
  key: string,
  value: V,
  size: (value: V) => number,
  limits: Limits,
  keep: string | null,
): Map<string, V> {
  const next = new Map(map);
  next.delete(key);
  next.set(key, value);
  let total = 0;
  for (const v of next.values()) total += size(v);
  for (const [k, v] of next) {
    if (next.size <= limits.entries && total <= limits.chars) break;
    if (k === key || k === keep) continue;
    next.delete(k);
    total -= size(v);
  }
  return next;
}
