// Quick Open 결과. plx-core/src/files.rs와 같은 모양.

/** plx-core files::FileMatch */
export type FileMatch = {
  /** 저장소 기준 경로 */
  path: string;
  /** 맞은 글자 위치(코드 포인트 단위, 오름차순) */
  positions: number[];
  /** 인덱스에는 있지만 작업트리에 없다(지우고 아직 커밋하지 않았다) */
  missing: boolean;
};

/** plx-core files::FileSearch */
export type FileSearch = {
  /** 작업트리 경로(고른 파일을 열 때 쓴다) */
  repo: string;
  /** 추적 파일 수 */
  files: number;
  /** 맞은 파일 수 */
  matched: number;
  /** 점수 순 앞부분. 빈 검색어면 최근 연 파일부터. */
  items: FileMatch[];
  /** 검색어 끝의 `:줄` */
  line: number | null;
  /** 이번에 목록을 (다시) 읽었으면 걸린 ms */
  loadMs: number | null;
  /** 백엔드가 찾는 데 걸린 ms */
  searchMs: number;
  /** 앞 결과 안에서만 찾았다 */
  narrowed: boolean;
};
