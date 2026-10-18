// 파일 diff 뷰: 분할(MergeView, 왼쪽이 이전) 또는 통합(unifiedMergeView). 읽기 전용이고 바뀌지 않은 긴 부분은 접는다.
import { defaultKeymap } from "@codemirror/commands";
import { MergeView, getChunks, goToNextChunk, goToPreviousChunk, unifiedMergeView } from "@codemirror/merge";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, drawSelection, highlightSpecialChars, keymap, lineNumbers } from "@codemirror/view";
import { codeSupport } from "../code";

export type DiffMode = "split" | "unified";

export type DiffView = {
  destroy(): void;
  /** 다음·이전 바뀐 부분으로 */
  next(): void;
  previous(): void;
  focus(): void;
  /** 새 버전에서 처음 바뀐 줄(1부터). 바뀐 곳이 없거나 지운 파일이면 null */
  firstChangedLine(): number | null;
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
  };
}

function firstLine(view: EditorView): number | null {
  const chunk = getChunks(view.state)?.chunks[0];
  if (!chunk || view.state.doc.length === 0) return null;
  return view.state.doc.lineAt(Math.min(chunk.fromB, view.state.doc.length)).number;
}
