// 읽기 전용 코드 뷰(CodeMirror 6). blame 거터(또는 커서 줄 끝의 blame 한 줄), 줄 번호, 검색, 줄 이동, 구문 강조
// (언어는 필요할 때 불러온다).
import { cursorLineDown, cursorLineUp, defaultKeymap, selectLineDown, selectLineUp } from "@codemirror/commands";
import {
  SearchQuery,
  findNext,
  findPrevious,
  getSearchQuery,
  gotoLine,
  highlightSelectionMatches,
  openSearchPanel,
  search,
  searchKeymap,
  searchPanelOpen,
  setSearchQuery,
} from "@codemirror/search";
import { Compartment, EditorState, Prec } from "@codemirror/state";
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
import { inlineBlame } from "./inline";

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
  /** `i` 커서 줄 커밋 카드 보이기·닫기 */
  card(): boolean;
  /** `o` 에디터로 열기 */
  openEditor(): boolean;
  /** `b` blame 거터 보이기·숨기기 */
  toggleGutter(): boolean;
  /** `:`·Ctrl+Shift+P(⌘⇧P) 명령 팔레트 */
  palette(): boolean;
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
  /** blame 거터를 보인다. 아니면 커서 줄 끝에 blame 한 줄을 쓴다. */
  showGutter: boolean;
  keys: EditorKeys;
  /** 검색 패널(`/`)을 열거나 닫았다 */
  onSearchPanel?(open: boolean): void;
  /** 커서가 옮겨 갔다(처음에도 부른다). 줄·열은 1부터 */
  onCursor?(line: number, col: number): void;
};

/** 다시 연 코드 뷰로 넘기는 검색: 검색어·옵션(`n`/`N`이 이어서 찾는다)과 패널이 열려 있었는지. */
export type SearchState = {
  query: { search: string; caseSensitive: boolean; regexp: boolean; wholeWord: boolean };
  open: boolean;
};

export type Editor = {
  view: EditorView;
  /** blame 거터 ↔ 커서 줄 끝의 blame 한 줄 */
  setGutter(show: boolean): void;
  destroy(): void;
};

export function createEditor(parent: HTMLElement, options: EditorOptions): Editor {
  const support = codeSupport(options.path);
  const { keys } = options;
  const run = (f: () => boolean) => () => f();

  const doc = options.doc;
  const blame = new Compartment();
  const blameDisplay = (show: boolean) => (show ? blameGutter(options.gutter) : inlineBlame(options.gutter));
  const state = EditorState.create({
    doc,
    extensions: [
      // git blame과 줄 번호를 맞춘다: 줄은 \n으로만 나눈다(외톨이 \r은 줄을 나누지 않는다).
      EditorState.lineSeparator.of("\n"),
      support.extensions,
      blame.of(blameDisplay(options.showGutter)),
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
          { key: "i", run: run(keys.card) },
          { key: "o", run: run(keys.openEditor) },
          { key: "b", run: run(keys.toggleGutter) },
          // 줄 이동(VS Code와 같은 Ctrl+G, macOS도). 검색 키맵의 Mod-g(다음 찾기)보다 먼저 받는다.
          { key: "Ctrl-g", run: gotoLine, preventDefault: true },
          { key: ":", run: run(keys.palette) },
          { key: "Mod-Shift-p", run: run(keys.palette), preventDefault: true },
          { key: "q", run: run(keys.back) },
          { key: "?", run: run(keys.help) },
          { key: "Escape", run: run(keys.escape) },
        ]),
      ),
      keymap.of([...searchKeymap, ...defaultKeymap]),
      EditorView.contentAttributes.of({ "aria-label": options.path }),
      EditorView.updateListener.of((update) => {
        const open = searchPanelOpen(update.state);
        if (open !== searchPanelOpen(update.startState)) options.onSearchPanel?.(open);
        if (update.selectionSet) reportCursor(update.view);
      }),
    ],
    selection: { anchor: lineStart(doc, options.line) },
  });

  const view = new EditorView({
    state,
    parent,
    scrollTo: options.line ? EditorView.scrollIntoView(state.selection.main.head, { y: "center" }) : undefined,
  });

  const detach = support.attach([view]);
  reportCursor(view);

  function reportCursor(v: EditorView) {
    if (!options.onCursor) return;
    const head = v.state.selection.main.head;
    const line = v.state.doc.lineAt(head);
    options.onCursor(line.number, head - line.from + 1);
  }

  return {
    view,
    setGutter(show) {
      view.dispatch({ effects: blame.reconfigure(blameDisplay(show)) });
    },
    destroy() {
      detach();
      view.destroy();
    },
  };
}

/** 지금 검색(검색어도 패널도 없으면 null). */
export function searchStateOf(view: EditorView): SearchState | null {
  const { search: text, caseSensitive, regexp, wholeWord } = getSearchQuery(view.state);
  const open = searchPanelOpen(view.state);
  return text || open ? { query: { search: text, caseSensitive, regexp, wholeWord }, open } : null;
}

/** 넘겨받은 검색을 되살린다(패널이 열려 있었으면 연다). */
export function restoreSearch(view: EditorView, state: SearchState): void {
  view.dispatch({ effects: setSearchQuery.of(new SearchQuery(state.query)) });
  if (state.open) openSearchPanel(view);
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
