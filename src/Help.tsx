// `?` 도움말. 키맵은 CLAUDE.md를 따른다. 아직 없는 기능은 어느 마일스톤에서 붙는지 적는다.
type Key = { keys: string; what: string; note?: string; planned?: boolean };

const COMMON: Key[] = [
  { keys: "q", what: "뒤로", note: "맨 앞 화면이면 창 숨기기" },
  { keys: "?", what: "이 도움말" },
  { keys: "`", what: "성능 HUD", note: "개발 빌드, PLX_PERF=1" },
  { keys: "Esc", what: "닫기" },
];

const KEYS: Record<"blame" | "commit", Key[]> = {
  blame: [
    { keys: "j / k", what: "아래·위 줄" },
    { keys: "Enter", what: "이 줄 커밋의 상세" },
    { keys: ",", what: "부모 리비전 blame (이 줄을 바꾸기 전)" },
    { keys: "< / >", what: "이전·다음 리비전 (파일을 바꾼 커밋 단위)" },
    { keys: "y", what: "커밋 sha 복사" },
    { keys: "/", what: "검색" },
    { keys: "n / N", what: "다음·이전 검색 결과" },
    { keys: "거터 클릭", what: "커밋 카드 고정" },
    { keys: "H / L", what: "파일·라인 히스토리", note: "M2", planned: true },
    { keys: "G", what: "커밋 그래프", note: "M3", planned: true },
  ],
  commit: [
    { keys: "j / k", what: "다음·이전 파일" },
    { keys: "n / N", what: "다음·이전 바뀐 부분" },
    { keys: "u", what: "분할 ↔ 통합 보기" },
    { keys: "Enter", what: "이 리비전의 파일 blame" },
    { keys: "y", what: "커밋 sha 복사" },
  ],
};

export function Help({ view, onClose }: { view: "blame" | "commit"; onClose(): void }) {
  return (
    <div className="overlay" role="dialog" aria-label="단축키" onClick={onClose}>
      <div className="help" onClick={(e) => e.stopPropagation()}>
        <h2>단축키 · {view === "blame" ? "blame" : "커밋 상세"}</h2>
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
