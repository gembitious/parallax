// 백엔드(crates/plx-core)가 보내는 blame 데이터. serde 직렬화 모양과 같아야 한다.

/** plx-core/src/blame.rs BlameCommit */
export type BlameCommit = {
  sha: string;
  author: string;
  authorMail: string;
  /** Unix 시각(초) */
  authorTime: number;
  /** 예: "+0900" */
  authorTz: string;
  committer: string;
  committerMail: string;
  committerTime: number;
  committerTz: string;
  summary: string;
  boundary: boolean;
};

/** plx-core/src/blame.rs BlameBlock */
export type BlameBlock = {
  sha: string;
  origLine: number;
  finalLine: number;
  numLines: number;
  path: string;
  previous: { sha: string; path: string } | null;
};

/** plx-core/src/blame.rs Chunk */
export type Chunk = { commits: BlameCommit[]; blocks: BlameBlock[] };

/** plx-core/src/blame.rs Stats */
export type BlameStats = {
  spawnMs: number;
  firstByteMs: number | null;
  firstBlockMs: number | null;
  totalMs: number;
  blocks: number;
  commits: number;
  /** git을 띄우지 않고 캐시에서 읽었다 */
  cached: boolean;
};

/** plx-core/src/text.rs Decoded */
export type Decoded = {
  text: string;
  encoding: "UTF-8" | "EUC-KR" | "UTF-8 (깨진 바이트)";
  lineCount: number;
};

/** plx-core/src/blame_session.rs BlameMeta */
export type BlameMeta = {
  /** 작업트리 경로 */
  repo: string;
  /** 저장소 기준 경로(`/` 구분) */
  path: string;
  /** 고정 리비전의 커밋 sha. 작업트리 blame이면 null */
  rev: string | null;
  content: Decoded | null;
  binary: boolean;
};

/** plx-core/src/blame_session.rs SessionEvent: meta → chunk* → done | error | cancelled */
export type SessionEvent =
  | ({ kind: "meta" } & BlameMeta)
  | ({ kind: "chunk" } & Chunk)
  | { kind: "done"; metaMs: number; stats: BlameStats }
  | { kind: "error"; message: string }
  | { kind: "cancelled" };

/** plx-core/src/repo.rs Person */
export type Person = { name: string; email: string; time: number; tzOffset: number };

/** plx-core/src/repo.rs CommitInfo */
export type CommitInfo = {
  sha: string;
  parents: string[];
  summary: string;
  message: string;
  author: Person;
  committer: Person;
  changedFiles: number;
};

/** plx-core/src/navigate.rs Version: 파일의 한 버전. rev가 null이면 작업트리 */
export type Version = { rev: string | null; path: string };

/** src-tauri/src/git.rs Neighbor */
export type Neighbor = { version: Version; line: number };
