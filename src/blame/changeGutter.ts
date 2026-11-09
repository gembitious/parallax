// 작업트리 blame의 바뀐 줄 거터(M13): 줄 번호 오른쪽에 HEAD 뒤로 더한 줄(초록)·바꾼 줄(파랑) 막대와 지운 자리(빨강
// 삼각형)를 그린다. VS Code의 줄 변경 표시와 같다. 누르면 작업트리 변경에서 이 파일을 연다.
// 작업트리 blame은 파일이 바뀌면 화면을 새로 만들므로 표시는 화면 동안 바뀌지 않는다.
import { EditorView, GutterMarker, gutter, type BlockInfo } from "@codemirror/view";
import { lineMarks, markTitle, type LineChange, type LineMark } from "./lineChanges";

class ChangeMarker extends GutterMarker {
  constructor(readonly mark: LineMark) {
    super();
  }

  eq(other: GutterMarker): boolean {
    return (
      other instanceof ChangeMarker &&
      other.mark.kind === this.mark.kind &&
      other.mark.deleted === this.mark.deleted &&
      other.mark.change === this.mark.change
    );
  }

  toDOM(): Node {
    const el = document.createElement("div");
    const classes = ["cm-change"];
    if (this.mark.kind) classes.push(this.mark.kind);
    if (this.mark.deleted) classes.push(`deleted-${this.mark.deleted}`);
    el.className = classes.join(" ");
    el.title = markTitle(this.mark);
    return el;
  }
}

const lineOf = (view: EditorView, block: BlockInfo) => view.state.doc.lineAt(block.from).number;

/** `changes`의 거터. `onClick(line)`은 표시가 있는 줄을 눌렀을 때다. */
export function changeGutter(changes: readonly LineChange[], lineCount: number, onClick: (line: number) => void) {
  const marks = lineMarks(changes, lineCount);
  const markers = new Map<number, ChangeMarker>();
  for (const [line, mark] of marks) markers.set(line, new ChangeMarker(mark));
  return gutter({
    class: "cm-change-gutter",
    lineMarker: (view, block) => markers.get(lineOf(view, block)) ?? null,
    domEventHandlers: {
      mousedown(view, block) {
        const line = lineOf(view, block);
        if (!markers.has(line)) return false;
        view.dispatch({ selection: { anchor: view.state.doc.line(line).from } });
        view.focus();
        onClick(line);
        return true;
      },
    },
  });
}
