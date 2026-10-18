// 커밋 그래프 데이터. crates/plx-core/src/graph.rs의 serde 모양과 같아야 한다.

/** 어떤 커밋에서 걷기 시작할지 */
export type GraphScope = "head" | "branches" | "all";

export type LabelKind = "head" | "branch" | "remote" | "tag";

export type Label = {
  /** `main`, `origin/main`, `v1.0`, `HEAD` */
  name: string;
  kind: LabelKind;
  /** HEAD가 가리키는 브랜치(detached면 `HEAD` 이름표) */
  current: boolean;
  /** 다른 워크트리가 체크아웃한 브랜치·HEAD면 그 워크트리 이름 */
  worktree?: string;
};

/** (레인, 색) */
export type Line = [number, number];
/** (위 가장자리 레인, 아래 가장자리 레인, 색) */
export type Through = [number, number, number];

export type GraphRow = {
  sha: string;
  parents: string[];
  summary: string;
  author: string;
  authorMail: string;
  /** Unix 시각(초) */
  authorTime: number;
  /** UTC와의 차이(초) */
  authorTzOffset: number;
  labels: Label[];
  /** 커밋 점이 놓인 레인 */
  lane: number;
  color: number;
  /** 커밋을 지나쳐 가는 선 */
  through: Through[];
  /** 위 가장자리에서 커밋 점으로 들어오는 선(자식들) */
  up: Line[];
  /** 커밋 점에서 아래 가장자리로 나가는 선(부모 순서대로) */
  down: Line[];
};

export type GraphMeta = {
  /** 작업트리 경로 */
  repo: string;
  head: string | null;
  branch: string | null;
  tips: number;
  scope: GraphScope;
  hiddenBranches: number;
  hiddenRemotes: number;
  commitGraph: boolean;
};

export type GraphStats = {
  loadMs: number;
  sortMs: number;
  firstPageMs: number;
  totalMs: number;
  rows: number;
  width: number;
  fromGraph: number;
  fromTable: number;
  fromObjects: number;
};

export type GraphEvent =
  | ({ kind: "meta" } & GraphMeta)
  | { kind: "loading"; commits: number }
  | { kind: "page"; rows: GraphRow[]; total: number }
  | { kind: "progress"; available: number; width: number }
  | { kind: "done"; stats: GraphStats }
  | { kind: "error"; message: string }
  | { kind: "cancelled" };

export type GraphWindow = {
  start: number;
  rows: GraphRow[];
  total: number;
  available: number;
  width: number;
  done: boolean;
};

/** 첫 화면으로 오는 행 수(plx-core graph::PAGE) */
export const PAGE = 100;
/** 레인 색 수(plx-core graph::COLORS) */
export const COLORS = 8;
