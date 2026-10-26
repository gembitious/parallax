// 커밋 상세 데이터. crates/plx-core/src/commit.rs의 serde 모양과 같아야 한다.
import type { CommitInfo, Decoded } from "../blame/types";

export type FileStatus = "added" | "deleted" | "modified" | "renamed" | "copied" | "typeChanged";

/** 파일 하나의 줄 수 */
export type FileStat = {
  /** 내용을 줄로 비교하지 않는다(앞 8000바이트에 NUL, `binary`·`-diff` 속성). git의 numstat `-` */
  binary: boolean;
  /** blob이 없다(부분 클론에서 받지 않았다). 줄 수를 모른다. */
  missing: boolean;
  additions: number;
  deletions: number;
};

export type FileChange = {
  status: FileStatus;
  /** 커밋 뒤 경로(지운 파일이면 지우기 전 경로) */
  path: string;
  /** rename·copy의 원래 경로 */
  oldPath: string | null;
  /** 경로가 UTF-8이 아니라 깨진 글자로 바꿔 읽었다. 이 경로로는 blame·히스토리·에디터를 열 수 없다. */
  pathLossy: boolean;
  oldId: string | null;
  newId: string | null;
  /** git 파일 모드(`100644`, `100755`, `120000`, `160000`). 추가면 oldMode, 삭제면 newMode가 없다. */
  oldMode: string | null;
  newMode: string | null;
  submodule: boolean;
  /** 줄 수. null이면 아직 세지 않았다(큰 커밋은 목록을 먼저 보이고 `file_stats`로 나머지를 받는다). */
  stat: FileStat | null;
  /** 바뀐 뒤가 작업트리 파일이다(작업트리 변경). `newId`는 그 내용의 해시라 저장소에 없다. */
  worktree: boolean;
};

export type CommitDetail = {
  info: CommitInfo;
  /** 경로 순 */
  files: FileChange[];
  /** 센 파일들의 합계(`counted`가 아니면 일부다) */
  additions: number;
  deletions: number;
  /** 모든 파일의 줄 수를 셌다. */
  counted: boolean;
  /** blob이 없어 rename을 다 견주지 못했다(부분 클론). */
  renamesIncomplete: boolean;
  /** 지운 파일 × 새 파일이 git의 한도(1,000²)를 넘어 유사도로 rename을 찾지 않았다(git도 건너뛴다). */
  renamesLimited: boolean;
};

/** 두 지점의 비교(`compare`). plx-core/src/changes.rs의 Comparison과 같은 모양. */
export type Comparison = Omit<CommitDetail, "info"> & {
  /** 바뀌기 전 커밋. 커밋이 없는 저장소의 작업트리 비교면 없다(빈 트리). */
  base: CommitInfo | null;
  /** 바뀐 뒤 커밋. 작업트리면 없다. */
  target: CommitInfo | null;
  /** `base`는 두 리비전이 갈라진 지점이다. */
  mergeBase: boolean;
  /** 보이지 않은 추적하지 않는 파일 수(한도를 넘은 것) */
  untrackedOmitted: number;
};

/** `file_stats`에 보내는 파일 하나 */
export type StatRequest = Pick<FileChange, "path" | "oldPath" | "oldId" | "newId" | "submodule" | "worktree">;

export type FileVersions = {
  /** 바뀌기 전 내용. 추가된 파일이거나 바이너리·큰 파일·받지 않은 blob이면 없다. */
  old: Decoded | null;
  new: Decoded | null;
  binary: boolean;
  /** blob이 없다(부분 클론에서 받지 않았다). */
  missing: boolean;
  /** 너무 커서 내용을 보내지 않았다: 두 쪽의 바이트 수 합. `full`로 다시 요청하면 보낸다. */
  tooLarge: number | null;
  /** 줄 diff에는 보이지 않는 바뀜(줄 끝, 끝 개행, BOM, 인코딩) */
  notes: string[];
};

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
