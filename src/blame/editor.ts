// 읽기 전용 코드 뷰(CodeMirror 6). blame 거터, 줄 번호, 검색, 구문 강조(언어는 필요할 때 불러온다).
import { cursorLineDown, cursorLineUp, defaultKeymap, selectLineDown, selectLineUp } from "@codemirror/commands";
import { findNext, findPrevious, highlightSelectionMatches, openSearchPanel, search, searchKeymap } from "@codemirror/search";
import { EditorState, Prec } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { codeSupport } from "../code";
import { blameGutter, type GutterContext } from "./gutter";

/** 화면 단축키. true를 돌려주면 처리한 것이다. CLAUDE.md 키맵을 따른다. */
export type EditorKeys = {
  copySha(): boolean;
  enter(): boolean;
  /** `,` 부모 리비전 blame */
  parent(): boolean;
  /** `<` 이전 리비전 */
  older(): boolean;
  /** `>` 다음 리비전 */
  newer(): boolean;
  /** `H` 파일 히스토리 */
  history(): boolean;
  /** `L` 라인 히스토리(선택한 줄들, 없으면 커서 줄) */
  lineHistory(): boolean;
  /** `G` 커밋 그래프(커서 줄의 커밋을 골라서) */
  graph(): boolean;
  /** `W` 워크트리 고르기 */
  worktrees(): boolean;
  /** `f`·Ctrl+P(⌘P) 파일 찾기 */
  files(): boolean;
  back(): boolean;
  help(): boolean;
  escape(): boolean;
};

export type EditorOptions = {
  doc: string;
  path: string;
  /** 처음 선택할 줄(1부터) */
  line: number | null;
  gutter: GutterContext;
  keys: EditorKeys;
};

export type Editor = { view: EditorView; destroy(): void };

export function createEditor(parent: HTMLElement, options: EditorOptions): Editor {
  const support = codeSupport(options.path);
  const { keys } = options;
  const run = (f: () => boolean) => () => f();

  const doc = options.doc;
  const state = EditorState.create({
    doc,
    extensions: [
      // git blame과 줄 번호를 맞춘다: 줄은 \n으로만 나눈다(외톨이 \r은 줄을 나누지 않는다).
      EditorState.lineSeparator.of("\n"),
      support.extensions,
      blameGutter(options.gutter),
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightActiveLine(),
      highlightSpecialChars(),
      drawSelection(),
      highlightSelectionMatches(),
      search({ top: true }),
      Prec.highest(
        keymap.of([
          { key: "j", run: cursorLineDown },
          { key: "k", run: cursorLineUp },
          // Shift: 줄 선택을 넓힌다(라인 히스토리 범위)
          { key: "J", run: selectLineDown },
          { key: "K", run: selectLineUp },
          { key: "y", run: run(keys.copySha) },
          { key: "/", run: openSearchPanel },
          { key: "n", run: findNext },
          { key: "N", run: findPrevious },
          { key: "Enter", run: run(keys.enter) },
          { key: ",", run: run(keys.parent) },
          { key: "<", run: run(keys.older) },
          { key: ">", run: run(keys.newer) },
          { key: "H", run: run(keys.history) },
          { key: "L", run: run(keys.lineHistory) },
          { key: "G", run: run(keys.graph) },
          { key: "W", run: run(keys.worktrees) },
          { key: "f", run: run(keys.files) },
          // 브라우저의 인쇄(Ctrl+P) 대신
          { key: "Mod-p", run: run(keys.files), preventDefault: true },
          { key: "q", run: run(keys.back) },
          { key: "?", run: run(keys.help) },
          { key: "Escape", run: run(keys.escape) },
        ]),
      ),
      keymap.of([...searchKeymap, ...defaultKeymap]),
      EditorView.contentAttributes.of({ "aria-label": options.path }),
    ],
    selection: { anchor: lineStart(doc, options.line) },
  });

  const view = new EditorView({
    state,
    parent,
    scrollTo: options.line ? EditorView.scrollIntoView(state.selection.main.head, { y: "center" }) : undefined,
  });

  const detach = support.attach([view]);

  return {
    view,
    destroy() {
      detach();
      view.destroy();
    },
  };
}

/** `line`번째 줄(1부터, 범위 밖이면 가까운 줄)의 시작 위치. */
function lineStart(doc: string, line: number | null): number {
  if (!line || line <= 1) return 0;
  let pos = 0;
  for (let i = 1; i < line; i++) {
    const next = doc.indexOf("\n", pos);
    if (next < 0) break;
    pos = next + 1;
  }
  return pos;
}
