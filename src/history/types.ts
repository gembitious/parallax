// 파일 히스토리 데이터. crates/plx-core의 log.rs·history.rs serde 모양과 같아야 한다.
import type { FileStatus } from "../commit/types";

/** plx-core/src/log.rs LogEntry: 파일을 바꾼 커밋 하나 */
export type LogEntry = {
  sha: string;
  parents: string[];
  author: string;
  authorMail: string;
  /** Unix 시각(초) */
  authorTime: number;
  /** 예: "+0900" */
  authorTz: string;
  summary: string;
  /** merge 커밋은 git이 알려 주지 않아 null */
  status: FileStatus | null;
  /** 이 커밋 뒤의 경로. 이름을 바꾸기 전 구간이면 옛 이름이다. */
  path: string;
  /** 이 커밋에서 이름이 바뀌었으면 원래 경로 */
  oldPath: string | null;
};

/** plx-core/src/history.rs HistoryMeta */
export type HistoryMeta = {
  /** 작업트리 경로 */
  repo: string;
  /** 저장소 기준 경로. 저장소 전체면 "." */
  path: string;
  /** 리비전을 정해 요청했으면 그 커밋 sha */
  rev: string | null;
  /** 거슬러 올라가기 시작한 커밋. 커밋이 없는 저장소면 null */
  start: string | null;
};

/** plx-core/src/history.rs HistoryStats. 시각은 요청을 받은 때부터(ms) */
export type HistoryStats = {
  /** git이 50개째(적으면 마지막 git이 끝난 때)를 낸 때. 캐시면 null */
  gitPageMs: number | null;
  firstEntryMs: number | null;
  firstPageMs: number | null;
  totalMs: number;
  entries: number;
  /** git log를 돌린 구간 수(rename마다 하나) */
  segments: number;
  cached: boolean;
};

/** plx-core/src/history.rs HistoryEvent: meta → entries* → done | error | cancelled */
export type HistoryEvent =
  | ({ kind: "meta" } & HistoryMeta)
  | { kind: "entries"; entries: LogEntry[] }
  | { kind: "done"; metaMs: number; stats: HistoryStats }
  | { kind: "error"; message: string }
  | { kind: "cancelled" };

/** 볼 파일. `repo`가 있으면 `path`는 저장소 기준 경로, 없으면 절대 경로다. `rev`가 null이면 HEAD부터다. */
export type HistoryTarget = { repo: string | null; path: string; rev: string | null };

/** 성능 예산(docs/MILESTONES.md)의 "첫 50개" */
export const PAGE = 50;
