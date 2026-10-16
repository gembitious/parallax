// `?` 도움말. 키맵은 CLAUDE.md를 따른다. 아직 없는 기능은 어느 마일스톤에서 붙는지 적는다.
type Key = { keys: string; what: string; note?: string; planned?: boolean };

const COMMON: Key[] = [
  { keys: "f / Ctrl+P", what: "파일 찾기 (Quick Open)", note: "macOS ⌘P · 초성 ㅍㅇ · 파일:줄 · Shift+Enter 히스토리" },
  { keys: "W", what: "워크트리 고르기", note: "그 워크트리의 같은 화면으로" },
  { keys: "q", what: "뒤로", note: "맨 앞 화면이면 창 숨기기" },
  { keys: "?", what: "이 도움말" },
  { keys: "`", what: "성능 HUD", note: "개발 빌드, PLX_PERF=1" },
  { keys: "Esc", what: "닫기" },
];

export type HelpView = "blame" | "commit" | "history" | "graph";

const TITLES: Record<HelpView, string> = {
  blame: "blame",
  commit: "커밋 상세",
  history: "파일 히스토리",
  graph: "커밋 그래프",
};

const KEYS: Record<HelpView, Key[]> = {
  blame: [
    { keys: "j / k", what: "아래·위 줄" },
    { keys: "J / K", what: "줄 선택 넓히기 (라인 히스토리 범위)" },
    { keys: "Enter", what: "이 줄 커밋의 상세" },
    { keys: ",", what: "부모 리비전 blame (이 줄을 바꾸기 전)" },
    { keys: "< / >", what: "이전·다음 리비전 (파일을 바꾼 커밋 단위)" },
    { keys: "y", what: "커밋 sha 복사" },
    { keys: "/", what: "검색" },
    { keys: "n / N", what: "다음·이전 검색 결과" },
    { keys: "거터 클릭", what: "커밋 카드 고정" },
    { keys: "H", what: "파일 히스토리 (이 리비전부터)" },
    { keys: "L", what: "라인 히스토리 (선택한 줄, 없으면 커서 줄)" },
    { keys: "G", what: "커밋 그래프 (이 줄의 커밋을 골라서)" },
  ],
  commit: [
    { keys: "j / k", what: "다음·이전 파일" },
    { keys: "n / N", what: "다음·이전 바뀐 부분" },
    { keys: "u", what: "분할 ↔ 통합 보기" },
    { keys: "Enter", what: "이 리비전의 파일 blame" },
    { keys: "a", what: "이 파일만 ↔ 모든 파일", note: "히스토리에서 열었을 때" },
    { keys: "y", what: "커밋 sha 복사" },
    { keys: "G", what: "커밋 그래프 (이 커밋을 골라서)" },
  ],
  history: [
    { keys: "j / k", what: "아래·위 커밋" },
    { keys: "PageDown / PageUp", what: "한 화면씩" },
    { keys: "Home / End", what: "처음·끝" },
    { keys: "Enter", what: "커밋 상세 (이 파일 diff)" },
    { keys: "b", what: "이 리비전의 파일 blame", note: "라인 히스토리면 그 범위의 줄로" },
    { keys: ",", what: "바꾸기 전 리비전의 파일 blame" },
    { keys: "y", what: "커밋 sha 복사" },
    { keys: "/", what: "검색 (요약·작성자·sha·경로)" },
    { keys: "n / N", what: "다음·이전 검색 결과" },
    { keys: "G", what: "커밋 그래프 (고른 커밋을 골라서)" },
  ],
  graph: [
    { keys: "j / k", what: "아래·위 커밋" },
    { keys: "PageDown / PageUp", what: "한 화면씩" },
    { keys: "Home / End", what: "처음·끝" },
    { keys: "Enter", what: "커밋 상세" },
    { keys: "y", what: "커밋 sha 복사" },
    { keys: "R", what: "원격 브랜치 모두 ↔ 로컬 브랜치와 upstream만" },
    { keys: "/", what: "검색 (메시지·작성자·sha)", note: "path:경로면 그 경로를 바꾼 커밋" },
    { keys: "n / N", what: "다음·이전 검색 결과" },
  ],
};

export function Help({ view, onClose }: { view: HelpView; onClose(): void }) {
  return (
    <div className="overlay" role="dialog" aria-label="단축키" onClick={onClose}>
      <div className="help" onClick={(e) => e.stopPropagation()}>
        <h2>단축키 · {TITLES[view]}</h2>
        <table>
          <tbody>
            {[...KEYS[view], ...COMMON].map(({ keys, what, note, planned }) => (
              <tr key={keys} className={planned ? "planned" : undefined}>
                <td>
                  <kbd>{keys}</kbd>
                </td>
                <td>
                  {what}
                  {note && <span className="muted"> ({note})</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
