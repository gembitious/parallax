// `?` 도움말. 키맵은 CLAUDE.md를 따른다. 아직 없는 기능은 어느 마일스톤에서 붙는지 적는다.
import { openNotices } from "./notices/Notices";
import { IS_MAC, MOD } from "./workspace/keys";

type Key = { keys: string; what: string; note?: string; planned?: boolean };

const COMMON: Key[] = [
  { keys: ": / Ctrl+Shift+P", what: "명령 팔레트", note: "macOS ⌘⇧P · 할 일을 이름·초성으로 찾는다" },
  {
    keys: "f / Ctrl+P",
    what: "파일 찾기 (Quick Open)",
    note: `macOS ⌘P · 초성 ㅍㅇ · 파일:줄 · Shift+Enter 히스토리 · ${MOD}Enter 옆에 열기`,
  },
  { keys: "Ctrl+O", what: "저장소 열기", note: "macOS ⌘O · 폴더나 파일을 창에 끌어 놓아도 연다" },
  { keys: "o", what: "에디터로 열기", note: "VS Code·Zed 등 작업트리 파일의 그 줄, PLX_EDITOR로 고른다" },
  { keys: "W", what: "워크트리 고르기", note: "그 워크트리의 같은 화면으로" },
  { keys: "q", what: "뒤로", note: "맨 앞 화면이면 창 숨기기" },
  { keys: "?", what: "이 도움말" },
  { keys: "`", what: "성능 HUD", note: "개발 빌드, PLX_PERF=1" },
  { keys: "Esc", what: "닫기" },
];

/** 작업 공간(트리·탭·패널) 키. 어느 화면에서든 받는다. */
const WORKSPACE: Key[] = [
  { keys: `${MOD}B`, what: "파일 트리 보이기·숨기기" },
  { keys: IS_MAC ? "⌘⇧E" : "Ctrl+Shift+E", what: "파일 트리로", note: "Esc로 돌아온다" },
  { keys: `${MOD}J`, what: "아래 패널 열기·닫기", note: "히스토리·그래프·커밋 상세" },
  { keys: "Ctrl+Tab / Ctrl+Shift+Tab", what: "다음·이전 탭", note: "Ctrl+PageDown·PageUp" },
  { keys: `${MOD}W`, what: "탭 닫기", note: "탭이 없으면 창 숨기기 · 가운데 단추로 탭을 눌러도 닫는다" },
  { keys: `${MOD}\\`, what: "에디터 나누기", note: "보이는 탭을 오른쪽에 하나 더 · 탭을 끌어 그룹의 옆에 놓아도 된다" },
  { keys: `${MOD}1~4`, what: "에디터 그룹으로", note: "왼쪽부터" },
  { keys: "D", what: "작업트리 변경", note: "커밋하지 않은 것을 HEAD와 견준다 · 파일을 고치면 바로 다시 견준다" },
];

/** 파일 트리에는 없는 화면 키(에디터로 열기, 워크트리, 뒤로) */
const TREE_SKIPS = new Set(["o", "W", "q"]);

export type HelpView = "workspace" | "blame" | "commit" | "history" | "graph";

const TITLES: Record<HelpView, string> = {
  workspace: "파일 트리",
  blame: "blame",
  commit: "커밋 상세",
  history: "파일 히스토리",
  graph: "커밋 그래프",
};

const KEYS: Record<HelpView, Key[]> = {
  workspace: [
    { keys: "j / k", what: "아래·위 파일" },
    { keys: "→ / ←", what: "폴더 펼치기·접기", note: "l·h, 접힌 다음 ←는 부모 폴더로" },
    { keys: "Enter", what: "탭으로 열기 (작업트리 blame)", note: "폴더면 펼치기·접기 · 마우스로 두 번 누르기와 같다" },
    { keys: `${MOD}Enter`, what: "옆에 열기", note: "오른쪽 에디터 그룹" },
    { keys: "Space", what: "미리보기로 열기", note: "마우스로 한 번 누르기와 같다 · 다음 미리보기가 그 탭 자리에 열린다" },
    { keys: "Esc / q", what: "에디터로 돌아가기" },
    { keys: "G", what: "커밋 그래프 (아래 패널)" },
    { keys: "b", what: "blame 거터 보이기·숨기기" },
  ],
  blame: [
    { keys: "j / k", what: "아래·위 줄" },
    { keys: "J / K", what: "줄 선택 넓히기 (라인 히스토리 범위)" },
    { keys: "Enter", what: "이 줄 커밋의 상세" },
    { keys: ",", what: "부모 리비전 blame (이 줄을 바꾸기 전)" },
    { keys: "< / >", what: "이전·다음 리비전 (파일을 바꾼 커밋 단위)" },
    { keys: "y", what: "커밋 sha 복사" },
    { keys: "/", what: "검색" },
    { keys: "n / N", what: "다음·이전 검색 결과" },
    { keys: "i", what: "커밋 카드 보이기·닫기", note: "거터 클릭으로도 고정" },
    { keys: "✦ 보라 줄", what: "AI가 쓴 줄 (git-ai 노트)", note: "카드에 에이전트·모델·시킨 사람" },
    { keys: "H", what: "파일 히스토리 (이 리비전부터)" },
    { keys: "L", what: "라인 히스토리 (선택한 줄, 없으면 커서 줄)" },
    { keys: "G", what: "커밋 그래프 (이 줄의 커밋을 골라서)" },
    { keys: "b", what: "blame 거터 보이기·숨기기", note: "끄면 커서 줄 끝에만 blame 한 줄" },
    { keys: "Ctrl+G", what: "줄로 이동", note: "상태 막대의 줄·열을 눌러도 된다" },
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
  const rows = [...KEYS[view], ...WORKSPACE, ...COMMON.filter((key) => view !== "workspace" || !TREE_SKIPS.has(key.keys))];
  return (
    <div className="overlay" role="dialog" aria-label="단축키" onClick={onClose}>
      <div className="help" onClick={(e) => e.stopPropagation()}>
        <h2>단축키 · {TITLES[view]}</h2>
        <table>
          <tbody>
            {rows.map(({ keys, what, note, planned }) => (
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
        <p className="muted help-footer">
          마우스로도 모두 한다: 맨 위 도구 막대, {IS_MAC ? "화면 위 메뉴 막대" : "왼쪽 위 메뉴(파일 · 보기 · 화면 이름 · 도움말)"},
          줄·행·파일·탭·머리줄 어디서든 오른쪽 단추로 여는 그 자리의 메뉴(diff의 줄은 누른 버전의 그 줄), blame 카드의 단추.
          메뉴에 단축키가 함께 보인다. 탭은 끌어서 옮긴다: 다른 그룹, 그룹의 왼쪽·오른쪽 끝(나눠 보기), 아래 패널. 패널은
          머리줄을 끌어 에디터에 놓으면 탭이 된다.
        </p>
        <p className="muted help-footer">
          <button
            type="button"
            className="link"
            onClick={() => {
              onClose();
              openNotices();
            }}
          >
            오픈 소스 라이선스
          </button>{" "}
          · 명령 팔레트에서도 연다
        </p>
      </div>
    </div>
  );
}
