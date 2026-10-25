// 파일 diff 뷰: 분할(MergeView, 왼쪽이 이전) 또는 통합(unifiedMergeView). 읽기 전용이고 바뀌지 않은 긴 부분은 접는다.
import { defaultKeymap } from "@codemirror/commands";
import { MergeView, getChunks, getOriginalDoc, goToNextChunk, goToPreviousChunk, unifiedMergeView } from "@codemirror/merge";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { EditorState, type Extension } from "@codemirror/state";
import { BlockType, EditorView, drawSelection, highlightSpecialChars, keymap, lineNumbers } from "@codemirror/view";
import { codeSupport, pointCursor, selectedLines } from "../code";

export type DiffMode = "split" | "unified";

/** diff의 두 버전: 바꾸기 전(첫 부모)과 이 커밋 */
export type DiffSide = "old" | "new";

/** 우클릭한 diff 줄 */
export type DiffPoint = {
  side: DiffSide;
  /** 그 버전의 줄(1부터) */
  line: number;
  /** 그 버전에서 고른 줄들(우클릭한 자리가 고른 글 안이면 그 범위, 아니면 그 줄) */
  lines: { start: number; end: number };
  /** 고른 글(복사) */
  text: string;
};

export type DiffView = {
  destroy(): void;
  /** 다음·이전 바뀐 부분으로 */
  next(): void;
  previous(): void;
  focus(): void;
  /** 새 버전에서 처음 바뀐 줄(1부터). 바뀐 곳이 없거나 지운 파일이면 null */
  firstChangedLine(): number | null;
  /**
   * 우클릭한 자리의 줄: 그 줄로 커서를 옮기고(고른 글 안이면 고른 것을 둔다) 어느 버전의 몇째 줄인지 돌려준다.
   * 줄이 아닌 자리(접은 줄, 맞춤 빈칸, 빈 버전)면 null.
   */
  pointAt(e: MouseEvent): DiffPoint | null;
};

export type DiffOptions = {
  /** 언어(구문 강조)를 고르는 데 쓴다 */
  path: string;
  old: string;
  new: string;
  mode: DiffMode;
};

/** 바뀐 곳 앞뒤로 보여 줄 줄 수와, 이보다 짧은 같은 부분은 접지 않는 기준 */
const COLLAPSE = { margin: 3, minSize: 6 };

/**
 * 접은 줄 표시. 기본 테마가 앞뒤에 붙이는 '⦚'(U+299A)는 코드·UI 글꼴에 없는 글자라 처음 그릴 때 시스템 글꼴 대체를
 * 찾는다(앱이 뜬 뒤 첫 diff의 첫 프레임이 늦었다, bench/results.md). 그 글자를 빼고 문구를 한국어로 쓴다.
 */
const collapsedLines: Extension = [
  EditorView.theme({
    ".cm-collapsedLines": { "&:before": { content: "none" }, "&:after": { content: "none" } },
  }),
  EditorState.phrases.of({ "$ unchanged lines": "바뀌지 않은 줄 $개" }),
];

export function createDiffView(parent: HTMLElement, options: DiffOptions): DiffView {
  const support = codeSupport(options.path);
  const base: Extension = [
    // git과 같은 줄 단위로 비교한다(CRLF는 미리 LF로 바꿔 둔다).
    EditorState.lineSeparator.of("\n"),
    support.extensions,
    collapsedLines,
    lineNumbers(),
    highlightSpecialChars(),
    drawSelection(),
    highlightSelectionMatches(),
    search({ top: true }),
    keymap.of([...searchKeymap, ...defaultKeymap]),
  ];

  if (options.mode === "split") {
    const merge = new MergeView({
      a: { doc: options.old, extensions: base },
      b: { doc: options.new, extensions: base },
      parent,
      gutter: true,
      highlightChanges: true,
      collapseUnchanged: COLLAPSE,
    });
    const detach = support.attach([merge.a, merge.b]);
    return {
      destroy() {
        detach();
        merge.destroy();
      },
      next: () => void goToNextChunk(merge.b),
      previous: () => void goToPreviousChunk(merge.b),
      focus: () => merge.b.focus(),
      firstChangedLine: () => firstLine(merge.b),
      pointAt(e) {
        const target = e.target as Node | null;
        if (merge.a.dom.contains(target)) return lineAt(merge.a, "old", e);
        if (merge.b.dom.contains(target)) return lineAt(merge.b, "new", e);
        return null;
      },
    };
  }

  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: options.new,
      extensions: [
        base,
        unifiedMergeView({
          original: options.old,
          mergeControls: false,
          gutter: true,
          highlightChanges: true,
          syntaxHighlightDeletions: true,
          collapseUnchanged: COLLAPSE,
        }),
      ],
    }),
  });
  const detach = support.attach([view]);
  return {
    destroy() {
      detach();
      view.destroy();
    },
    next: () => void goToNextChunk(view),
    previous: () => void goToPreviousChunk(view),
    focus: () => view.focus(),
    firstChangedLine: () => firstLine(view),
    pointAt: (e) => (view.dom.contains(e.target as Node | null) ? (deletedLineAt(view, e) ?? lineAt(view, "new", e)) : null),
  };
}

function firstLine(view: EditorView): number | null {
  const chunk = getChunks(view.state)?.chunks[0];
  if (!chunk || view.state.doc.length === 0) return null;
  return view.state.doc.lineAt(Math.min(chunk.fromB, view.state.doc.length)).number;
}

/**
 * 코드 뷰 한쪽의 그 높이 줄. 빈 버전(새로 만든 파일의 이전, 지운 파일의 이후)이나 줄이 아닌 블록(접은 줄, 분할 보기의
 * 맞춤 빈칸, 통합 보기의 지운 줄)이면 null
 */
function lineAt(view: EditorView, side: DiffSide, e: MouseEvent): DiffPoint | null {
  if (view.state.doc.length === 0) return null;
  const block = view.elementAtHeight(e.clientY - view.documentTop);
  if (block.type !== BlockType.Text) return null;
  pointCursor(view, e);
  const { state } = view;
  const { from, to } = state.selection.main;
  return { side, line: state.doc.lineAt(block.from).number, lines: selectedLines(state), text: state.sliceDoc(from, to) };
}

/**
 * 통합 보기에서 지운 줄(이전 버전의 줄을 위젯으로 그린 것) 위를 눌렀으면 그 줄. 거터를 눌렀어도 높이로 찾는다.
 * 코드 뷰의 글이 아니라 커서를 옮기지 않는다.
 */
function deletedLineAt(view: EditorView, e: MouseEvent): DiffPoint | null {
  for (const chunkDom of view.contentDOM.querySelectorAll<HTMLElement>(".cm-deletedChunk")) {
    const lines = chunkDom.querySelectorAll(":scope > .cm-deletedLine");
    for (let i = 0; i < lines.length; i++) {
      const box = lines[i].getBoundingClientRect();
      if (e.clientY < box.top || e.clientY >= box.bottom) continue;
      // 위젯은 그 덩어리가 시작하는 새 버전 자리에 있다(@codemirror/merge의 acceptChunk와 같이 찾는다).
      const at = view.posAtDOM(chunkDom);
      const chunk = getChunks(view.state)?.chunks.find((c) => c.fromB <= at && c.endB >= at);
      if (!chunk) return null;
      const line = getOriginalDoc(view.state).lineAt(chunk.fromA).number + i;
      return { side: "old", line, lines: { start: line, end: line }, text: window.getSelection()?.toString() ?? "" };
    }
  }
  return null;
}
