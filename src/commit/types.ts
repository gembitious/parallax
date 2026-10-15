// 커밋 상세 데이터. crates/plx-core/src/commit.rs의 serde 모양과 같아야 한다.
import type { CommitInfo, Decoded } from "../blame/types";

export type FileStatus = "added" | "deleted" | "modified" | "renamed" | "copied" | "typeChanged";

export type FileChange = {
  status: FileStatus;
  /** 커밋 뒤 경로(지운 파일이면 지우기 전 경로) */
  path: string;
  /** rename·copy의 원래 경로 */
  oldPath: string | null;
  oldId: string | null;
  newId: string | null;
  binary: boolean;
  submodule: boolean;
  additions: number;
  deletions: number;
};

export type CommitDetail = {
  info: CommitInfo;
  files: FileChange[];
  additions: number;
  deletions: number;
};

export type FileVersions = { old: Decoded | null; new: Decoded | null; binary: boolean };

/** git diff --name-status와 같은 글자 */
export const STATUS_LETTER: Record<FileStatus, string> = {
  added: "A",
  deleted: "D",
  modified: "M",
  renamed: "R",
  copied: "C",
  typeChanged: "T",
};

export const STATUS_NAME: Record<FileStatus, string> = {
  added: "추가",
  deleted: "삭제",
  modified: "수정",
  renamed: "이름 바꿈",
  copied: "복사",
  typeChanged: "종류 바뀜",
};
