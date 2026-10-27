// 탭에 보이는 이름. 탭 막대·탭 메뉴·끌 때 따라오는 이름표가 쓴다. 순수 함수라 단위 테스트한다(tabs.test.ts).
import { shortSha } from "../blame/format";
import { compareTitle, lastName, titleOf, type StackEntry } from "../nav";

export type TabLabel = {
  /** 탭 이름 */
  name: string;
  /** 이름 뒤에 흐리게(리비전, 줄 범위) */
  detail: string | null;
  /** 풍선 도움말(저장소 기준 경로 전체) */
  title: string;
};

/** 경로의 이름. 저장소 전체(`.`)는 "저장소"다. */
const nameOf = (path: string) => (path === "." ? "저장소" : lastName(path));

/**
 * 화면의 탭 이름: blame은 파일 이름(리비전), 히스토리는 "파일 히스토리"(라인 히스토리는 줄 범위), 커밋 상세는
 * "커밋 sha", 그래프는 "그래프".
 */
export function tabLabel(entry: StackEntry): TabLabel {
  const title = titleOf(entry);
  switch (entry.view) {
    case "blame": {
      const { path, rev } = entry.target;
      return { name: nameOf(path), detail: rev ? shortSha(rev) : null, title };
    }
    case "history": {
      const { path, rev } = entry.target;
      const at = rev ? ` @${shortSha(rev)}` : "";
      return entry.lines
        ? { name: `${nameOf(path)} 라인 히스토리`, detail: `${entry.lines.start}-${entry.lines.end}${at}`, title }
        : { name: `${nameOf(path)} 히스토리`, detail: rev ? shortSha(rev) : null, title };
    }
    case "commit":
      return { name: entry.compare ? compareTitle(entry.compare) : `커밋 ${shortSha(entry.rev)}`, detail: null, title };
    case "graph":
      return { name: "그래프", detail: null, title };
    case "search":
      return { name: "검색", detail: entry.rev ? shortSha(entry.rev) : null, title };
  }
}
